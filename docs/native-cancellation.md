# Native syntax cancellation

`DocumentParserConfig::cancellation` now reaches native syntax exploration and
AdditionalParse sessions. A caller clones `DocumentCancellationToken`, supplies
one clone in the config and requests cancellation on the other. Default callers
still wait for completion under the existing quotas. This adds execution control;
it does not change syntax selection or connect an LSP client.

## Boundaries and outcomes

Native `PatternMatchEnvironment` and split `PatternMatchHooks` expose an optional
`cancellation_requested()` probe with a default of `false`. A cancellation
request is monotonic for the current parse. The WASM host temporarily attaches
the document token for the syntax phase, so nested builtin and custom
AdditionalParse work uses the same request. The previous host token is restored
on ordinary success and typed error paths, before mapping the document result.

Probes run before matcher/candidate work, matcher states and regex endpoints,
Expression recursion and memo lookup, Function candidates and default capture
completion, semantic candidate scopes, Structure/Section/Entry nodes, Type dispatch and each
AdditionalParse request. Each syntax guest invocation or continuation also
checks before opening its invocation. Probes neither consume existing matcher
quotas nor reorder candidates.

`ParseCancelled` is a dedicated Rust error cause. Matcher, Expression and host
error enums add a `Cancelled` variant; exhaustive downstream Rust matches need
to handle it. `parse_cancellation()` traverses typed error sources without
matching message text. The document boundary returns
`DocumentParseError::Cancelled { stage: Syntax, .. }`, rather than a syntax
diagnostic, no-match, recovered extension fault or successful partial result.
Existing phase checks and the final commit check remain in place.

The environment traits retain their existing String-returning callbacks. A
callback forwarding an actual typed cancellation stores its cause for one
`take_parse_cancellation()` call. Native adapters consume it immediately when
converting that callback's error, before cleanup. A String that merely says
"cancelled" has no such meaning. Cleanup failure remains the returned failure
even when cancellation was requested; checking the token alone must not replace
an actual rollback error.

## State and memo

Matcher rejection finalization, Function/semantic scope cleanup, AdditionalParse
active-request cleanup and the document rollback remain responsible for their
existing state boundaries. Dispatch cancellation first restores the continuation
or dispatch savepoint. A fatal document error then discards speculative
StateStore writes, dynamic syntax and the uncommitted Function registry. A
cleanup error takes precedence over the cancellation being cleaned up.

Cancellation is checked before a memo hit and clears the session memo when
observed. Error unwinding still removes active keys. It cannot become a cached
failed parse or reuse an interrupted subtree. Source, Catalog, configuration and
dynamic snapshot are fixed for the session; no semantic cache is added across
documents or transactions. These changes do not resolve the separate provider
contract for arbitrary guest globals, depth-dependent semantics or memo
eligibility, and do not change left-recursive growth or local resource quotas.

## Remaining scope

This is cooperative cancellation. A running regex or WASM invocation completes
or reaches its existing quota before another probe can stop native work. RawTree
construction, index preparation, macro stages outside native syntax and AST
projection retain their existing stage-boundary cancellation behavior. There is
no new global work limit, wall-clock latency guarantee, guest preemption or
automatic editor revision integration.

SSG integrity/reference validation and existing aliases/BlockData status remain
unchanged. The current export cannot distinguish a legitimately empty converter
registry from a failed hook that exported an empty registry. Detecting that case
requires producer-side health evidence or an explicit generation failure in the
separate SSG repository. Rust-only inference from capability flags or empty
arrays would not solve it; that producer change is outside this commit.
The SSG main checked on 2026-10-16 is still the audited
`27366cdeff5b120b0cafe4ef21a79ed7788eb8e9`: `ConverterCollector` reads the hook
map, while `HookManager.init` logs initialization failure without returning a
collection outcome to Snapshot generation. A producer failure signal and the
consumer's handling of unavailable domains need a separate cross-repository
scope decision.

The known FunctionCall argument-binding omission in the final AstTree/ParseResult
projection remains separate from capture equivalence and cancellation.

## Regression coverage

Nine cancellation tests cover typed nested errors and the String forwarding
channel, cancellation before a memo hit, matcher cleanup, cleanup-failure
priority, nested Structure bodies, native document interruption/next revision,
pending StateStore rollback, dispatch before the first guest, and cancellation
inside a fresh builtin AdditionalParse session. The last two include uncancelled
controls proving that their targets perform real work.

The document tests use a `cfg(test)` probe budget, rather than wall-clock sleeps
or a production work limit, to request cancellation reproducibly inside syntax
exploration. The Message/default, Function/nested-call, and Command/Condition
documents complete with 226, 935, and 216 native probes respectively. Each is
interrupted at budgets 0, half, and one below the completed count. All nine
interrupted parses return typed Syntax cancellation, release invocation/request
state, restore the host token, unfreeze dynamic syntax, discard the Function
registry, and permit a successful next revision. A separate test seeds a real
pending StateStore write and verifies it is absent after document rollback.

| Run | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| Three affected crates, including doctests | 658 | 0 | 7 |
| Final document tests after the last two test-only additions | 5 | 0 | 0 |

Three document tests repeat the first run; the final two add dispatch and
AdditionalParse position controls. Thus 660 distinct regression tests passed
across the runs. No production Rust source changed between these runs or after
the no-cancellation measurement. All 14 changed Rust files match the final LXC
copy by SHA-256; the measured source differs only by the two final `cfg(test)`
test functions. Formatting and diff-whitespace checks pass. Full-workspace CI,
native Windows execution and release-profile performance were not run.

Printed `native-cancel` elapsed times start before the whole parse, including
preprocessing before the request is triggered. They are diagnostic single
observations, not request-to-stop latency measurements. The tests assert no
wall-clock bound.

```sh
cargo test -p skript-parser -p parser-wasm -p skript-repl --locked --offline -- \
  --test-threads=2
cargo test -p parser-wasm --lib --locked --offline host::document::tests:: -- \
  --nocapture --test-threads=2
```

## No-cancellation pipeline comparison

The [document pipeline harness](capture-view-measurement.md#archived-server-export-warm-document-pipeline)
is reused without changes: the same complete schema-7 server export, three
Event-wrapped corpus inputs, one warmup and one measured revision per case,
15 processes per implementation and alternating order. The fixed borrowed-only
binary contains `afdf925` plus the harness committed as `93b33fa`; its SHA-256
is reused from the earlier comparison. The other binary adds native
cancellation. No cancellation is requested. Probe-budget instrumentation is
compiled out of the `parser-wasm` dependency in these REPL runs; the ordinary
document token remains active throughout.

In the raw CSV/driver, historical mode labels are retained: `baseline` means
borrowed-only and `borrowed` means borrowed plus native cancellation. They do
not mean the original pre-borrowing baseline. `parse` and `report` retain the
intervals defined in the linked measurement. Both binaries use Debian 13 LXC,
i7-8700, four visible CPUs, Rust 1.97.1, test profile `opt-level=1`, line-table
debug information, no incremental compilation, and one benchmark test thread.
Builds use two jobs/niceness 10 and do not overlap measurement. CPU frequency,
scheduling and the OS page cache are uncontrolled. This test-build comparison
does not establish release overhead, allocation counts or peak memory.

The first binary-copy attempt ran out of disk space before any sample ran.
The task's redundant baseline build target was removed while source, frozen
binaries and prior evidence were retained. A hardlink to the completed native
test executable avoids duplicating it. The comparison then ran once; its
failed setup log is retained separately from the complete sample logs.

Milliseconds per document, median [minimum, maximum] over 15 samples per mode:

| Case | Borrowed-only parse | With native cancellation | Observed median change |
| --- | ---: | ---: | ---: |
| doc-message-event | 164.357 [159.948, 200.212] | 166.173 [158.359, 173.169] | +1.11% |
| short-teleport | 1206.792 [1184.229, 1295.187] | 1206.089 [1169.968, 1301.056] | -0.06% |
| research-teleport | 7926.069 [7757.550, 8212.345] | 7973.049 [7803.689, 8063.579] | +0.59% |

| Case | Borrowed-only report | With native cancellation |
| --- | ---: | ---: |
| doc-message-event | 0.077 [0.073, 0.140] | 0.080 [0.074, 0.123] |
| short-teleport | 0.144 [0.137, 0.208] | 0.139 [0.128, 0.187] |
| research-teleport | 0.299 [0.279, 0.398] | 0.293 [0.282, 0.469] |

The observed parse-median changes range from -0.06% to +1.11%, with overlapping
sample ranges. These observations neither establish zero overhead nor predict
other inputs. Report times show no consistent direction. Cancellation response
latency is not measured by this no-cancellation comparison.

For each case, all 30 full JSON observations agree after removing only
`parseDurationNs`, and also equal the saved observations from the earlier
baseline/borrowed comparison. All 45 paired checksums and 30 process exits agree.
The [raw samples](native-cancellation-samples.csv) preserve all 90 batches.
The harness, all 21 fixture files and embedded CoreLibrary match the earlier
comparison; the borrowed-only binary hash is identical. Complete observations,
binary hashes, execution order, logs and source fingerprints are retained with
the review evidence.

To reproduce, compile each version with the identical harness/artifacts as
described in the linked measurement, then run:

```sh
python3 utilities/skript-repl/tests/measurements/run_capture_pipeline.py \
  BORROWED_ONLY_TEST_BINARY NATIVE_CANCEL_TEST_BINARY NEW_OUTPUT_DIRECTORY
```

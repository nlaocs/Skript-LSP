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
completion, semantic candidate scopes, Structure/Section/Entry nodes, Type
dispatch and each AdditionalParse request. Each syntax guest invocation or
continuation also checks before opening its invocation. Probes neither consume
existing matcher quotas nor reorder candidates.

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
consumer's handling of unavailable domains are deferred from this PR and need
their own cross-repository scope.

The known FunctionCall argument-binding omission in the final AstTree/ParseResult
projection remains separate from capture equivalence and cancellation.

## Regression coverage

Ten cancellation tests cover typed nested errors and the String forwarding
channel, cancellation before a memo hit, matcher cleanup, cleanup-failure
priority, nested Structure bodies, native document interruption/next revision,
pending StateStore rollback, dispatch before the first guest, and cancellation
inside a fresh builtin AdditionalParse session. The last two include uncancelled
controls proving that their targets perform real work.

The additional `cancellation_during_recursive_work_is_not_memoized` test requests
cancellation in an inner literal callback while parsing `(x)`. With registered
Expressions disabled, the inner parse accepts a literal and inserts a nonempty
memo seed. The next seed-growth pass observes cancellation. The typed error
unwinds both nested active keys, and the test asserts that memo and active keys
are empty. This covers interrupted exploration separately from the memo-hit
test; no failed or partial cancellation result is retained.

The document tests use a `cfg(test)` probe budget, rather than wall-clock sleeps
or a production work limit, to request cancellation reproducibly inside syntax
exploration. The Message/default, Function/nested-call, and Command/Condition
documents complete with 226, 935, and 216 native probes respectively. Each is
interrupted at budgets 0, half, and one below the completed count. All nine
interrupted parses return typed Syntax cancellation, release invocation/request
state, restore the host token, unfreeze dynamic syntax, discard the Function
registry, and permit a successful next revision. A separate test seeds a real
pending StateStore write and verifies it is absent after document rollback.

Initial regression runs at `984f48c`, before publication-preparation additions:

| Run | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| Three affected crates, including doctests | 658 | 0 | 7 |
| Final document tests after the last two test-only additions | 5 | 0 | 0 |

Three document tests repeated the first run; the final two added dispatch and
AdditionalParse position controls. Thus 660 distinct regression tests passed
across those runs. No production Rust source changed between those runs or after
the no-cancellation measurement. All 14 changed Rust files matched the LXC
copy by SHA-256 at `984f48c`; its measured source differed only by the two final
`cfg(test)` test functions. Those initial runs did not cover full-workspace CI,
native Windows execution or release-profile performance. The later verification
below records the publication-preparation additions.

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
test executable avoided duplicating it. The comparison then ran once; its
failed setup log is retained separately from the complete sample logs.
Before later CI builds, the fixed native binary was detached into an independent
copy and its SHA-256 rechecked to protect the measured bytes from Cargo writes.

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

## Publication-preparation verification

The additional memo-seed test is committed as `6c5bb6b`. Production Rust code is
unchanged from `984f48c`. The PR path in `.github/workflows/ci.yml` rebuilds
CoreLibrary and the parser test components, then runs workspace tests with all
features, including doctests. There is no fmt or clippy job in that workflow.
The local verification uses the existing Debian 13 LXC and Rust 1.97.1; the
workflow's runner is `ubuntu-latest` with the stable toolchain.

```sh
cargo run -p xtask --locked --offline -- build-core-library
cargo run -p xtask --locked --offline -- build-test-components
cargo test --workspace --all-features --locked --offline --jobs 2 -- \
  --test-threads=2
```

The two build commands inherit two Cargo jobs and niceness 10. Test-profile
assertions, overflow checks and `opt-level=1` are retained, with line-table debug
information and incremental compilation disabled. The local run adds offline
mode and limits runtime test threads to two. All 542 tracked files in the source
archive for `6c5bb6b` were checked by SHA-256 before building, before the retry
and after testing; all three manifests agree.

Both artifact-build commands exited successfully. The workspace run completed
with **1194 passed, zero failed and seven ignored** across 84 test-result groups,
including doctests. These are the final workspace results, not a sum with the
earlier three-crate runs. All 12 WASM artifacts were byte-identical before the
rebuild and after testing, including the previously measured CoreLibrary. The
three fixed document-pipeline binaries also retained their recorded SHA-256.

The first workspace attempt failed during compilation with `ENOSPC` (exit 101),
before tests ran. After that process exited, 84 old ELF test executables totaling
9,051,887,264 bytes were removed only from this task's
`target-candidate/debug/deps` directory. Paths, sizes, inodes and SHA-256 were
recorded before removal; source, rlibs and fixed measurement binaries were kept.
The same source and workspace command then completed successfully. The original
failure and recovery records remain alongside the successful retry logs. This
is local CI-equivalent verification, not a hosted GitHub Actions result.

On Windows x86_64 MSVC, Rust 1.97.1 ran the existing native cancellation targets:

```sh
cargo test -p skript-parser --lib --test pattern_match --locked --offline \
  --jobs 1 cancellation -- --nocapture --test-threads=1
```

All six selected tests passed (four library and two matcher integration tests),
with zero failures. The process used BelowNormal priority, a separate local
target directory, one Cargo job and one runtime test thread. The same test
profile settings were used. This verifies the native cancellation paths on
Windows; the WASM host and full workspace were not run on Windows. No RVC
process or system setting was changed.

The manual [release traversal follow-up](capture-view-measurement.md#release-traversal-follow-up-2026-10-16-utc)
also completed successfully: 150 batches, all 75 paired checksums equal. It is
the same preconstructed-tree read traversal, not a release document pipeline or
a cancellation response test. The full release document/WASM pipeline was not
built or measured because of its additional build/cache cost. Release
cancellation overhead and request-to-stop latency therefore remain unverified.

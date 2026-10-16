# Borrowed capture traversal

Baseline: `982531dfeb8a1047b9ecc101ceb6857d31f37450`.

`ExpressionNode::parsed_capture_refs()` enumerates typed children and routed
captures without cloning recursive values or constructing semantic summaries.
`ParsedCaptureRef::summary()` constructs a typed child's summary on demand and
borrows an existing routed summary. `to_owned()` and the existing
`parsed_captures()` API explicitly retain owning behavior.

The AstTree builder, AdditionalParse result arena and REPL source coloring use
the borrowed iterator. Capture ordering, unresolved-slot skipping, grouped
summary delegation, default provenance and existing routed failure behavior are
preserved. FunctionCall argument binding remains present in native values; its
known omission from the final AstTree/ParseResult projection is a separate issue.

## Reproduce

```sh
CARGO_PROFILE_TEST_DEBUG=line-tables-only CARGO_INCREMENTAL=0 \
  cargo test -p skript-parser --lib --locked measure_capture_traversal -- \
  --ignored --nocapture --test-threads=1
```

The ignored test keeps the pre-change capture construction algorithm in
`legacy_captures`, independent of the new iterator. It compares owning and
borrowed traversal in the same test binary, checks equal checksums, warms each
mode five times and alternates their order across 15 samples. Each sample runs
50 traversals. Benchmark output fields are
`capture-view,shape,sample,mode,repetitions,total_ns,checksum`;
the saved CSV omits the `capture-view` marker column.
Divide `total_ns` by `repetitions` before summarizing samples.

Inputs are synthetic preconstructed trees: chains of 8/32/128 Expressions, one
root with 128 leaf children, and a routed Condition containing a 32-Expression
chain. Node construction, fixture loading, parsing, compiler work and WASM host
initialization are outside the timed interval. Nodes carry type, public data,
metadata and shared effects. The owning mode includes temporary captures,
summaries and recursive clones, matching the old read traversal; the borrowed
mode does not request summaries.

## Measured results (2026-10-16 UTC)

Microseconds per traversal, summarized over 15 samples per mode:

| Shape | Owning median | Owning min–max | Borrowed median | Borrowed min–max |
| --- | ---: | ---: | ---: | ---: |
| chain-8 | 27.302 | 27.157–54.170 | 0.099 | 0.098–0.101 |
| chain-32 | 357.568 | 353.010–458.879 | 0.469 | 0.462–0.541 |
| chain-128 | 6442.118 | 6051.252–7251.954 | 2.355 | 2.176–2.859 |
| wide-128 | 176.008 | 172.868–195.727 | 1.114 | 1.108–1.436 |
| routed-condition-32 | 416.899 | 393.979–477.951 | 0.481 | 0.467–0.993 |

All scenarios returned equal traversal checksums. These values describe the
synthetic read traversal above; they are not estimates of document parse latency.
The [raw samples](capture-view-samples.csv) contain all 150 batches.

## Release traversal follow-up (2026-10-16 UTC)

The same synthetic traversal harness was run at `6c5bb6b` using the default
`--release` profile, with no release-profile overrides. Production Rust remains
unchanged from `984f48c`. It keeps the frozen pre-change owning algorithm and
borrowed iterator in one binary, using the same five warmups, 15 alternating
samples per mode and 50 traversals per sample. The machine, toolchain and
uncontrolled CPU/page-cache conditions are the same as above. Compilation used
one Cargo job and niceness 10, after all workspace/Windows tests had finished;
no build overlapped measurement.

```sh
CARGO_INCREMENTAL=0 cargo test -p skript-parser --release --lib --locked \
  --offline --jobs 1 measure_capture_traversal -- \
  --ignored --nocapture --test-threads=1
```

Microseconds per traversal, median [minimum, maximum] over 15 samples per mode:

| Shape | Owning | Borrowed |
| --- | ---: | ---: |
| chain-8 | 24.497 [23.778, 25.372] | 0.026 [0.025, 0.028] |
| chain-32 | 322.114 [314.866, 394.355] | 0.111 [0.101, 0.247] |
| chain-128 | 5902.033 [5527.467, 6453.541] | 0.808 [0.726, 0.933] |
| wide-128 | 160.313 [157.035, 190.517] | 0.377 [0.361, 0.398] |
| routed-condition-32 | 371.482 [361.256, 376.253] | 0.117 [0.109, 0.124] |

All 75 paired checksums agree. The
[raw release samples](capture-view-release-samples.csv) contain all 150 batches;
the complete log and binary SHA-256 are retained with the evidence. These values
measure only the synthetic read traversal. A release document/WASM pipeline
build was not attempted because of its additional build/cache cost. Release
document speed, cancellation overhead, request-to-stop latency, allocation
counts and peak memory remain unmeasured. This does not establish whole-parser
linear complexity or a pipeline speedup factor.

## Regression verification

| Run | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| Baseline `skript-parser`, including doctests | 228 | 0 | 0 |
| Borrowed `skript-parser`, including doctests | 231 | 0 | 1 |
| Borrowed `parser-wasm` and `skript-repl`, including doctests | 420 | 0 | 5 |
| Explicit manual microbenchmark | 1 | 0 | 0 |

The three affected crates therefore passed 651 regression tests, with six
ignored entries; one is the manually run microbenchmark. Seven equivalence
tests were added. All 12 test WASM artifacts were rebuilt from the baseline
source and held fixed. Baseline and candidate used distinct target directories;
new test names and source hashes were checked. Formatting and diff whitespace
checks passed. These initial runs did not include full-workspace CI or native
Windows execution; later verification is recorded in
[native cancellation](native-cancellation.md).
An import-only cleanup was followed by another REPL suite run: 114 passed,
zero failed, two ignored; this repeat is excluded from the 651 total. The six
changed Rust source files matched the measured/tested copy by SHA-256.

## Conditions and scope

The measurement uses Debian 13 x86_64 in LXC, an Intel Core i7-8700, four visible
CPUs, Rust 1.97.1, the repository's test profile (`opt-level=1`), line-table debug
information, incremental compilation disabled and one benchmark test thread.
Builds use two jobs with niceness 10. There is no CPU pinning, frequency lock or
OS page-cache reset. This is a traversal microbenchmark, not an end-to-end parse
or projection benchmark. Allocation counts and peak memory are not measured.

Regression tests separately compare the frozen owning enumeration, AstTree
capture/node projection, arena IDs/nodes/parser IDs/attachments, and REPL
span/color/depth/order/rendering. Native Function values, defaults, grouped
summaries, mixed capture indices, missing children and routed failures are covered.

Routed capture lookup still searches the stored capture list. Requested grouped
and list summaries can still inspect descendants. Owning callers, parse memo
clones and result construction are unchanged. These results do not establish
linear complexity for the whole parser or a speedup on real addon documents.
Cancellation and SSG collection health remain separate work units.

## Archived server export: warm document pipeline

A second measurement compares baseline `982531dfeb8a1047b9ecc101ceb6857d31f37450`
with the borrowed implementation committed as `afdf925`. Both receive the same
manual test harness from `utilities/skript-repl/tests/measurements/` and its test
module declaration in `session.rs`. They use separate build targets and fixed
copies of the resulting test binaries. No cancellation changes are in these
binaries.

The fixture is the existing complete 21-file `skript-2.16.2-java25` server export:
schema 7, Snapshot ID
`b7c14c3cf6b8864973c14d5cfa773334e9e502013e5c8fae70b3afc9506be04d`,
Paper 26.2 / Java 25 / Skript 2.16.2, two captured plugins (Skript and SSG).
See the existing corpus README for producer and JAR provenance. This comparison
does not certify collection health for every domain or multi-addon behavior.

Three existing corpus Effects are wrapped in their recorded Event header and
one indented body line: `doc-message-event`, `short-teleport` and
`research-teleport`. These are curated documentation/research inputs against a
real registry export, not a survey of production script projects. All three
documents must be matched, without component faults.

For each case, a fresh host is initialized outside the timed interval. One
warmup revision precedes one measured revision using that same host. Fifteen
independent processes per implementation alternate baseline/borrowed order by
sample index. Each batch contains one parse. The CPU, Rust version, test profile
and build limits are the same as above; no build runs concurrently with this
measurement.

`parse` times `ParserHost::parse_document`: raw tree, macros, native/WASM syntax,
AdditionalParse results, AstTree projection and commit. `report` times
`DocumentAnalysisReport::from_result`, including document DTO/color/diagnostic
construction and destruction of the consumed native result. JSON serialization,
output comparison, thread creation and host initialization are outside both
intervals. The changed single-line Effect coloring traversal is not exercised
by this document-report interval.

Milliseconds per document, median [minimum, maximum] over 15 samples:

| Case | Baseline parse | Borrowed parse | Observed median change |
| --- | ---: | ---: | ---: |
| doc-message-event | 168.868 [161.625, 172.995] | 162.749 [158.167, 171.749] | -3.62% |
| short-teleport | 1229.078 [1199.846, 1353.726] | 1201.294 [1175.479, 1232.366] | -2.26% |
| research-teleport | 8056.842 [7830.031, 8171.359] | 7915.703 [7854.569, 8149.988] | -1.75% |

| Case | Baseline report | Borrowed report |
| --- | ---: | ---: |
| doc-message-event | 0.078 [0.072, 0.117] | 0.081 [0.074, 0.086] |
| short-teleport | 0.139 [0.127, 0.148] | 0.142 [0.133, 0.164] |
| research-teleport | 0.293 [0.275, 0.535] | 0.291 [0.274, 0.303] |

The observed parse medians are 1.75–3.62% lower, with overlapping ranges.
CPU frequency and scheduling are uncontrolled; these measurements do not
establish a general parser speedup or predict other documents. The long case
still takes about eight seconds. Report-construction times show no consistent
improvement. Synthetic traversal ratios therefore cannot be used as pipeline
speedup factors.

For each case, all 30 full JSON observations agree exactly after removing only the
`parseDurationNs` field. All 45 paired checksums agree, and all processes exit
successfully. The [raw samples](capture-pipeline-samples.csv) contain 90 batches.
Both measured harness hashes, all 21 fixture file hashes and the embedded
CoreLibrary hash agree. The six borrowed Rust source hashes match the earlier
verified implementation. Binary hashes, complete observations, per-process logs
and source fingerprints are retained with the review evidence.

To repeat, apply only the harness and its module declaration to both source
copies, use the same fixed WASM artifacts, and compile each copy separately:

```sh
CARGO_PROFILE_TEST_DEBUG=line-tables-only CARGO_INCREMENTAL=0 \
  cargo test -p skript-repl --lib --locked --no-run --message-format=json
```

Take each `skript_repl` test executable from its compiler-artifact record, then:

```sh
python3 utilities/skript-repl/tests/measurements/run_capture_pipeline.py \
  BASELINE_TEST_BINARY BORROWED_TEST_BINARY NEW_OUTPUT_DIRECTORY
```

The driver preserves every log and batch, records binary/harness hashes and
execution order, and rejects differing full observations. It does not replace
the regression suites or establish upstream final-tree equivalence.

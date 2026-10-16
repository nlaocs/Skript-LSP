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
checks passed. Full-workspace CI and native Windows execution were not run.
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

# Catalog class reachability reuse

The measurements used `perf/research-teleport`, originally created from
`ec4e47ccb5bd32fd6460de7618fec7eeb80d7a2f`, the head of
[PR #129](https://github.com/nlaocs/Skript-LSP/pull/129). That PR was open and
unmerged when this branch was created. The follow-up was local and unpublished
when these measurements were recorded.

## Reproduction and timing boundaries

The same [archived server export and three document inputs](capture-view-measurement.md#archived-server-export-warm-document-pipeline)
are used: complete schema-7 `skript-2.16.2-java25`, 21 fixture files, Snapshot ID
`b7c14c3cf6b8864973c14d5cfa773334e9e502013e5c8fae70b3afc9506be04d`.
The research input is the existing six-argument `location(...)` expression,
wrapped in its recorded `on load` Event. No input-specific product path is added.

Three runs of the saved native-cancellation binary first reproduced the long
case at 7.811, 8.059 and 7.975 seconds. All three cases' full JSON observations
agreed between these runs and the later baseline/candidate comparison.

`parse` still covers `parse_document`, including RawTree, macros, native/WASM
syntax, AdditionalParse, AST projection and commit. `report` covers the consumed
document report. Snapshot loading, session/Catalog/host initialization, JSON
serialization, comparison and worker creation are outside both intervals.

The harness now also prints the first revision's already-measured duration.
`first` means the first document revision in a newly initialized session, not a
fully cold process or cold Catalog: initialization can already query metadata.
`warm` is the next fresh revision of the same input and host. Native expression
sessions are recreated; this is not an AST-result cache-hit benchmark.

All measurements use Debian 13 LXC, i7-8700, four visible CPUs and Rust 1.97.1.
The test profile has `opt-level=1`, assertions/overflow checks enabled, line-table
debug information and no incremental compilation. Builds use one Cargo job and
niceness 10. Each benchmark has one test thread and the existing 32 MiB parser
worker stack. Builds and Windows tests do not overlap timing. CPU frequency,
scheduling and OS page cache are uncontrolled. This is not a release measurement.

## Investigation before changing the algorithm

The LXC has no perf, valgrind or gdb. Temporary synchronous RAII probes measured
22 native/host/guest-boundary regions. A test-binary System allocator wrapper
counted successful allocation, zeroed allocation and reallocation requests on
the parsing thread. Host initialization and report construction remain outside
this instrumentation window. The temporary probes were removed before building
both comparison binaries; they are not product code.

The second instrumented warm parse took 8.646 seconds. Selected **exclusive**
times and allocation-request counts from that observation are:

| Region | Calls | Exclusive ms | Allocation requests |
| --- | ---: | ---: | ---: |
| Type parser candidate selection | 172 | 2769.226 | 15,144,301 |
| Native candidate matching | 6,875 | 1176.358 | 5,122,052 |
| Native Expression recursion | 11,733 | 649.350 | 2,148,145 |
| Registered-node construction | 3,446 | 643.823 | 2,440,193 |
| Hook-effects checkpoint capture/restore | 346,651 | 619.457 | 14,158,768 |
| Memo-hit clone | 11,525 | 611.827 | 12,220,701 |
| Memo seed insertion/clone | 3,160 | 505.828 | 6,733,147 |
| Candidate-list construction | 3,750 | 396.109 | 24,251 |
| WASM Expression hook boundary | 3,528 | 583.885 | 684,299 |
| WASM Type hook boundary | 413 | 113.170 | 631,831 |
| Regex compilation | 116 | 26.556 | 28,536 |
| Regex captures | 1,854 | 0.583 | 1,971 |

All 22 regions' exclusive times sum exactly to the document region's inclusive
time. Nested inclusive times overlap and must not be added. Probe bookkeeping
and allocator accounting perturb execution and can remain in parent exclusive
time; this observation ranks work rather than measuring uninstrumented speed.
WASM hook time includes host imports. Other macro/initialization paths not
instrumented separately remain in their enclosing region.

The allocator observed 64,084,331 requests totaling 5,212,488,641 requested bytes,
including 5,927,900 reallocations. A reallocation contributes its full new size,
not its growth. These are cumulative requests, not live/peak memory. C-side
allocators, other threads and the guest linear heap are not fully covered.
Allocations during profiler bookkeeping are deliberately not counted on reentry.
Six tracked requests lie outside the classified regions; complete time
partitioning does not imply complete allocation attribution.
The [complete region CSV](research-teleport-profile.csv) retains all 22 regions.

Type candidate selection repeatedly calls `Catalog::can_convert`, whose class
checks previously rebuilt a queue and visited set for each pair. The first
optimization therefore reuses complete graph reachability for each **known
source class inside its immutable Catalog**, initialized on demand with
`OnceLock`. Unknown source names are not stored. Self-equality, known reference
types' Object shortcut, primitive behavior, unresolved parent names and cycle
handling retain their existing results. Catalog replacement creates a different
index; no provider, transaction, expression-result or cross-Catalog cache is
introduced. Candidate ordering and hook/state/fault/SourceMap paths are unchanged.

## Comparison without instrumentation

Both binaries use byte-identical timing-harness source, fixed WASM artifacts and
the same compiler profile. The baseline production parser is `ec4e47c`; the
candidate adds only Catalog reachability reuse. The added regression tests and
Python driver options do not run inside either timed interval.

Seven independent processes per mode alternate order by sample. Each process
initializes a new session per case, parses a first revision, then a warm next
revision. All results must be matched without component faults. The harness
compares first/warm full JSON, and the driver compares every process/mode after
removing only `parseDurationNs`.

Milliseconds per document, median [minimum, maximum] over seven samples:

| Warm revision | Baseline | Cached reachability | Median change |
| --- | ---: | ---: | ---: |
| doc-message-event | 166.178 [158.020, 168.819] | 57.247 [55.705, 59.478] | -65.55% |
| short-teleport | 1216.800 [1194.978, 1300.945] | 347.666 [338.033, 373.598] | -71.43% |
| research-teleport | 8000.606 [7942.702, 8075.043] | 4981.282 [4861.185, 5048.536] | -37.74% |

| First revision after session initialization | Baseline | Cached reachability | Median change |
| --- | ---: | ---: | ---: |
| doc-message-event | 197.631 [192.952, 210.310] | 87.420 [85.453, 92.451] | -55.77% |
| short-teleport | 1217.380 [1197.715, 1277.612] | 349.810 [339.307, 375.372] | -71.27% |
| research-teleport | 7904.705 [7878.749, 8089.593] | 4915.801 [4817.385, 5065.730] | -37.81% |

All 14 process exits succeed. The 42 first and 42 warm batches agree in their
checksums, and all full JSON observations agree. The
[raw samples](research-teleport-samples.csv) preserve all 84 batches. The warm
report medians remain about 0.08/0.14/0.29 ms; no report-construction speedup is
established. These are three curated inputs against a real registry export,
not an addon-compatibility survey or a whole-parser complexity result. The
known FunctionCall argument-binding omission in final projection is unchanged;
baseline equality does not certify the unseen upstream final graph.

## Whole-process resource observations and retention risk

After the timing comparison, three more processes per mode run serially in
alternating order. Linux `wait4` records each entire test process, including
fixture/session initialization and all three cases' first/warm parses:

| Metric, median [minimum, maximum] | Baseline | Cached reachability |
| --- | ---: | ---: |
| Peak RSS, KiB | 325064 [324956, 325792] | 321736 [318956, 321768] |
| User CPU, seconds | 25.483 [25.467, 25.588] | 17.566 [17.484, 17.616] |
| Whole-process wall time, seconds | 25.662 [25.620, 25.771] | 17.733 [17.645, 17.791] |

These RSS observations do not isolate the new cache or establish a general
memory reduction. It retains sets and class-name copies, and cloning a warmed
Catalog copies initialized sets. A first class query may traverse more than the
old early-success search. In the worst hierarchy, retained reachability can be
quadratic in class count; this change adds no global memory quota or latency
bound. Large/deep hierarchy behavior and warm-Catalog clone cost need further
measurement before broader performance claims.

As a graph-shape check, this fixture has 1,802 known classes. Filling every source
closure would contain 17,573 entries, with 645,643 UTF-8 name bytes and at most
33 entries per source. This excludes HashSet/String/OnceLock/allocator overhead
and does not measure the subset actually initialized during parsing.

## Verification and reproduction

- Catalog integration targets: 27 passed, zero failed in LXC and Windows MSVC.
- New differential coverage freezes the previous pair-query BFS and compares
  cold/warm/concurrent/clone queries across cyclic and partially missing graphs.
- A separate test checks unresolved parent names and isolation between Catalogs.
- Windows uses BelowNormal priority, one Cargo job and one test thread, offline.
- All-features regression for `syntaxes`, `skript-parser`, `parser-wasm` and
  `skript-repl`: 703 passed, zero failed, seven ignored across 47 result groups,
  including all four crates' doctest targets; exit code zero.
- `cargo fmt --all -- --check` and `git diff --check` pass.

After regression, all 542 transferred source files, 12 fixed WASM artifacts and
the saved comparison/reference binaries retain their expected SHA-256 values.
Both temporary profiler modules are absent. The transfer originally started at
`6c5bb6b`; compared with the local candidate, 533 files are byte-identical,
seven differ only in line endings, and two historical measurement documents
have the later `ec4e47c` edits locally. Those documentation differences are not
compiled inputs. The new follow-up document/CSVs are also written after testing.

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=line-tables-only \
  cargo test -p skript-repl --lib --locked --offline --no-run \
  --message-format=json --jobs 1
python3 utilities/skript-repl/tests/measurements/run_capture_pipeline.py \
  BASELINE_TEST_BINARY CACHED_TEST_BINARY NEW_OUTPUT_DIRECTORY \
  --samples 7 --candidate-label candidate
cargo test -p syntaxes -p skript-parser -p parser-wasm -p skript-repl \
  --all-features --locked --offline --jobs 1 -- --test-threads=2
```

Apply the same additive first-revision harness output to the baseline. Take test
executable paths from Cargo's compiler-artifact records and keep independent
copies before rebuilding. The driver retains every log, full observation,
execution order and binary/harness SHA-256. Defaults remain 15 samples and the
historical `borrowed` candidate label for earlier comparisons.

The accompanying review evidence also keeps both temporary profiling patches,
their exact source fingerprints, raw region/allocation logs, resource records,
source/binary identities and independent reviews. The remaining long-case time
still needs investigation; baseline memo/seed and checkpoint copy costs suggest
next measurements, not a measured claim about the optimized parser's breakdown.
Release pipeline performance, different inputs/snapshots, peak memory isolated
to parsing, and full cold initialization remain unmeasured here.

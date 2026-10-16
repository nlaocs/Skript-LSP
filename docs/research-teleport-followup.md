# Remaining document cost and Catalog cache tradeoffs

This continues the [reachability investigation](research-teleport-measurement.md)
from `31a4c774346376a9e4234f4c91327b789ccbe940` on the same local branch.
Its dependency remains [PR #129](https://github.com/nlaocs/Skript-LSP/pull/129).
The follow-up was local and unpublished when these measurements were recorded.

## Cache ownership and correctness

`Catalog` keeps its normalized `parts` and `index` private. Its public queries
borrow classes; no graph-mutating API exists. `with_unchecked_source` replaces
raw provenance, not the normalized graph. Creating a new Catalog, including
SSG reload, creates fresh `OnceLock` slots. There is no invalidation counter to
forget to advance. A live old `Arc<Catalog>` still refers to its old immutable
graph, as expected.

Each initializer computes the full closure for its known source. Its result
does not depend on the requested target, expected type, parser context,
transaction or guest state. A negative answer is a membership miss in that
metadata closure, not a cached parse failure. Arbitrary caller-supplied target
names are not stored; unknown source names do not allocate a row. The existing
self-equality/Object shortcuts remain before initialization. Missing parent
names are retained so their reachability results match the previous BFS.

Initialization uses one visited set and only `class` lookups, never another
row's `get_or_init`. Cycles terminate without recursively locking rows. The
previous differential tests cover query order, negative-first queries,
concurrent queries, warmed clones and independent Catalogs. Independent code
review found no ownership, invalidation or result-equivalence defect here.

REPL, parser host and dynamic registry share `Arc<Catalog>`. Parsing the next
document revision does not deep-clone the Catalog. Calling `Catalog::clone` or
cloning a `Snapshot` itself does copy initialized closure sets and strings.

## Remaining warm parse work

The same real schema-7 Snapshot and research input are used. The existing
uninstrumented test-profile warm median was 4.981 seconds. The new instrumented
observation takes **5.538 seconds**; instrumentation overhead is included.
Temporary RAII probes partition 22 regions; a System allocator wrapper observes
Rust allocation requests on the parse thread. These probes and the temporary
cache inspection API are removed before either A/B build.

Selected exclusive times from the instrumented warm revision:

| Region | Calls | Exclusive ms | Allocation requests |
| --- | ---: | ---: | ---: |
| Native candidate matching | 6,875 | 1181.012 | 5,122,052 |
| Effects checkpoint capture/restore | 346,651 | 622.584 | 14,158,768 |
| Memo-hit clone | 11,525 | 615.496 | 12,220,701 |
| WASM Expression boundary | 3,528 | 568.500 | 684,299 |
| Memo seed insertion/clone | 3,160 | 506.596 | 6,733,147 |
| Registered-node construction | 3,446 | 383.458 | 1,477,852 |
| Candidate-list construction | 3,750 | 383.143 | 24,251 |
| Native Expression recursion | 11,733 | 335.081 | 262,011 |
| Type parser candidate selection | 172 | 282.323 | 12,951 |
| Host dispatch | 3,944 | 246.823 | 2,521,275 |
| WASM Type boundary | 413 | 109.318 | 631,831 |
| Regex compilation | 116 | 25.397 | 28,536 |
| Regex captures | 1,854 | 0.511 | 1,971 |

The [full CSV](research-teleport-cached-profile.csv) retains all 22 regions.
Their exclusive times sum exactly to document inclusive time, for both measured
revisions. Inclusive nested times overlap. Guest boundaries include host
imports. Allocation requests total 45,979,505 and requested bytes total
3,581,522,857; these are cumulative requests, not peak/live heap or RSS, and
reallocation contributes its full new size. Other threads, C allocators and
guest linear-heap operations are not fully covered. The added allocation meter
also affects timing, so this is a ranking of remaining work, not an
uninstrumented speed comparison with the earlier profiler run.

## Cache retention, first queries and clone cost

The instrumented document starts with zero initialized rows and uses 134 of the
1,802 rows by the end of its first revision. The warm next revision uses the
same 134 rows: 726 reachability entries, total HashSet capacity 932, and 20,758
UTF-8 bytes of class names. The slot vector itself occupies 100,912 bytes on
this 64-bit build. These figures do not include all container/allocator costs
and are not a measurement of this subset's total heap.

A separate isolated test loads the same Snapshot without initializing a host.
Five repetitions run in one process with the same instrumented test profile.
A thread-local meter records successful allocated/reallocated layout sizes
and freed layout sizes. Net bytes are the difference within each measured
operation; retained results stay alive through the end of that operation.
Allocator metadata, fragmentation and OS residency are excluded. Fixture load,
preparing query names, reporting and destruction after each clone lie outside
the measured intervals.

| Operation | Queries | Median ms [min, max] | Net requested bytes |
| --- | ---: | ---: | ---: |
| Previous BFS, first direct-parent query for each source | 1,456 | 0.520 [0.491, 0.524] | 0 |
| Cached closure, same first direct-parent queries | 1,456 | 3.466 [3.271, 4.519] | 922,121 |
| Fill all source closures with negative queries | 1,802 | 4.914 [4.690, 6.668] | 1,333,075 |
| Warm negative queries, ten rounds | 18,020 | 1.847 [1.798, 2.027] | 0 |
| Clone whole cold Catalog | — | 12.129 [11.422, 23.962] | 15,074,232 |
| Clone whole fully warmed Catalog | — | 10.790 [10.668, 11.734] | 16,407,307 |

Direct-parent reference/cached order alternates by repetition. Object shortcuts
mean that only 1,354 rows initialize in that workload. The all-negative workload
fills all 1,802 rows, with 17,573 entries and 645,643 UTF-8 name bytes, agreeing
with the earlier graph-shape calculation. Its net retained increase is about
1.27 MiB, in addition to the already allocated slot vector. Warm negative
queries make no observed allocation requests.

The first direct-parent batch makes 2,708 allocation requests with the reference
BFS and 20,320 with full closure caching; zero net retained bytes for BFS does
not mean zero allocations. Filling all rows makes 28,371 requests. The peak net
requested delta is 922,841 bytes for the direct-parent cache batch and 1,333,175
bytes for all-negative filling, excluding allocations that predate each batch.

Each whole cold clone makes 247,580 allocation requests; the fully warmed clone
makes 266,955. The extra 19,375 requests and 1,333,075 net bytes are the copied
cache contents. Cold-then-warm clone order is fixed, so allocator reuse and
within-process effects prevent interpreting the lower warm clone time as a
clone speedup. The [30 raw rows](research-teleport-cache-costs.csv) retain every
timing, allocation count, net/peak-net delta and cache-shape record.

For the actual document's 134 initialized rows, a second instrumented process
compares an owned clone with a clone of otherwise identical metadata whose
cache was emptied. The initialized clone makes 248,440 requests and retains
15,124,334 net requested bytes; the empty clone makes 247,580 requests and
retains 15,074,232. The **860 requests and 50,102 bytes** difference measures
copying this workload's cache contents. Both also allocate the 100,912-byte
slot vector. These are clone layout differences, not original live heap or RSS.
The [four observations](research-teleport-cache-copy.csv) cover two document
revisions in one process, always initialized clone before empty clone. They do
not support a general clone latency comparison.

First closure construction can cost more than an early-success BFS and has no
internal cancellation probe. Retention remains potentially quadratic for a
large hierarchy. This fixture does not establish a general latency/memory
bound. Sharing the cache across cloned Catalogs would save copies in that API,
but it does not address this pipeline's remaining seconds because the host
already shares the Catalog with Arc. The current step does not change that
ownership policy or introduce quotas.

## Small seed-copy experiment

The final left-recursive pass can add zero candidates while improving the
failure trace. Reusing the previous memo candidate vector in that case avoids
another deep copy. The failure result must still be stored, and a missing seed
must still be reinserted after recovery. Error/limit/cancellation cleanup and
candidate order stay on the existing paths.

Two focused experimental tests passed: a stalled rejected growth preserves its
new trace on the next memo hit, and successful recursive growth preserves
candidate order, mapped spans and deferred-effect identities on hits.

The missing-seed recovery fallback is preserved and independently source-reviewed;
the new focused tests do not force that exact recovery/clear branch.

The extra `native.seed-reuse` probe observes only 128 eligible updates among
3,160 seed updates. Against the instrumented baseline, this one-process
observation records 2,845 fewer of 45,979,505 allocation requests (0.0062%) and
198,094 fewer requested bytes (0.0055%).
It does not remove the whole 506.6 ms seed region. The [23 warm regions from
this experiment](research-teleport-seed-profile.csv) keep the reused path
separate. Instrumented time differences are not an acceptance criterion.

Uninstrumented builds use seven independent processes per version, alternating
baseline/candidate order. Each process measures one first and one next-revision
parse per case. Median research-input parse times, in milliseconds:

| Profile / revision | Existing cache | Seed-copy experiment | Median change |
| --- | ---: | ---: | ---: |
| Test / first | 4998.422 | 4920.840 | -1.55% |
| Test / warm | 4945.198 | 5011.387 | +1.34% |
| Release / first | 4085.099 | 3993.570 | -2.24% |
| Release / warm | 4089.003 | 4093.584 | +0.11% |

Warm results do not show a consistent improvement. **The experiment and its
two tests were removed from the product diff.** An LF-normalized patch, the
final passing focused-test log, earlier failed test-development attempts and
the exact source/binary hashes remain in the evidence bundle. The
[168 sample rows](research-teleport-seed-samples.csv) retain all three cases,
both profiles and both phases, including min/max values recoverable per group.
Every process agrees with the existing full JSON observation for all cases.

## Cycle termination fix

Independent review also found an existing cycle hole outside this cache:
`hierarchy_distance` walks the superclass chain without a visited/bounded walk.
For `A.super=B`, `B.super=A`, `A.interfaces=[I]`, assignability to `I` is true
but searching for the end of the superclass chain does not terminate. The
current SSG validator checks parent existence, not acyclicity. This is a static
code-review finding; the old hanging call was not executed. It predates
reachability reuse and is separate from the cache's cycle-safe membership query.

The retained product change bounds the superclass walk by the number of distinct
known class names. In an immutable graph with at most one superclass edge per
class, a longer walk cannot finish without repeating a class. An exhausted
budget returns `None`, which also represents an unresolved distance. Target
equality stays before the budget check: a chain of N known classes can still
reach a missing target at distance N. No visited collection is added to this
walk. The existing assignability check can still initialize its cache.

Two regressions cover the cyclic interface case and finite-result boundaries:
targets within the cycle, self equality, unknown names, an unresolved target
after all known classes, duplicate class names with last-wins lookup, and the
existing interface-distance convention. Independent review found no blocking
issue in the implementation or those boundaries.

The final release comparison repeats the same seven-process protocol against
the unchanged cached baseline, with only this guard added:

| Case | First baseline / guard ms | Warm baseline / guard ms |
| --- | ---: | ---: |
| Message event | 62.437 / 62.560 | 46.507 / 44.981 |
| Short teleport | 272.433 / 274.480 | 279.595 / 277.796 |
| Research teleport | 4058.437 / 4038.639 | 4072.944 / 4034.902 |

The [84 sample rows](research-teleport-guard-samples.csv) and all 14 process logs
retain the evidence. Full JSON agrees in every process. This small correctness
guard is not claimed as another performance optimization; the variation does
not establish a speedup.

## Conditions, reproduction and validation

Measurements run serially in the previously approved Debian 13 LXC, with four
visible CPUs on an i7-8700 and 16 GiB RAM, Rust 1.97.1, `nice -n 10`, one build
job and offline locked dependencies. The fixed schema-7 Snapshot has 21 files,
1,802 classes and Snapshot ID
`b7c14c3cf6b8864973c14d5cfa773334e9e502013e5c8fae70b3afc9506be04d`.
Core WASM SHA-256 remains
`3a2ae04052e59ac89acfbbe7a8fddf550ead73bc5353883d8f66cb23927309c4`.
No builds or Windows tests overlap the timed A/B runs. GPU training is untouched.
The host is shared with that background workload; these are low-priority
measurements on a shared machine, not an isolated hardware benchmark.

Test builds use opt-level 1, debug assertions and overflow checks, line-table
debug information and disabled incremental compilation. Release builds use
opt-level 3, debug assertions/overflow checks disabled and no debug information.
These are native `cargo test --lib` executables running the real document
pipeline on a 32 MiB parser worker stack matching the CLI. Each case starts a
fresh session. Timings are elapsed wall time, including scheduling effects.
Host/Snapshot setup and report formatting lie outside parse time;
"first" does not measure complete cold process startup. Release results do not
measure an installed editor/LSP request or a CLI command's whole execution.

The evidence bundle includes the baseline source archive, verified overlays,
temporary probes, rejected patch, compiler artifact profiles and SHA-256
identities. It also retains `lxc-run.sh` and the `cc`/`cxx`/`ar` Zig wrappers;
the build scripts pass `--jobs 1` over the runner's default job count.
`apply-followup-source.py` checks every before/after hash and touches
verified overlays to avoid stale Cargo timestamps. `followup-release31.sh`,
`followup-bench-seed.sh`, `followup-profile3.sh`, `followup-profile4.sh` and
`followup-guard.sh` record their exact build/run commands. Their paths are scoped
to the existing task-owned LXC checkout and require adapting for another host.
The checked-in driver can repeat either uninstrumented comparison:

```sh
python3 utilities/skript-repl/tests/measurements/run_capture_pipeline.py \
  BASELINE_TEST_EXECUTABLE CANDIDATE_TEST_EXECUTABLE OUTPUT_DIRECTORY \
  --samples 7 --candidate-label candidate
```

The cache micro-measurement is
`session::capture_pipeline::measure_catalog_cache --exact --ignored --nocapture
--test-threads=1` in the temporary instrumented REPL test executable.
`analyze-followup.py --final` verifies sample counts/order, full observations,
checksum pairs, profiles, timing partitions and the regression summary, then
generates the public CSVs. `check-followup-integrity.py` verifies restored source
inputs, fixed WASM and frozen executable hashes after regression.

Final validation: `syntaxes`, `skript-parser`, `parser-wasm` and `skript-repl`
with all features, locked/offline dependencies, one build job and two test
threads pass **705 tests, zero failures, seven ignored**, across 47 result
groups including doctests. Focused Catalog tests pass 29/29 in both the LXC and
Windows; the Windows run uses one test thread and BelowNormal priority. Formatting
and diff whitespace checks pass. Post-regression SHA-256 checks verify 542 source
inputs, all 12 fixed WASM artifacts and 12 frozen executables; both temporary
profiler modules and the rejected seed-test module are absent. Local comparison
allows seven existing EOL-only differences and two historical documentation
differences from the original transfer archive; executable source content
matches after EOL normalization.

## Remaining limits

The known FunctionCall argument-binding omission in final AST projection is
unchanged. Equality against the prior parser is not an upstream final-graph
certificate. Larger structural changes to memo/AST sharing or checkpoint
storage need their own design and invariants; the present evidence alone does
not establish their correctness or speed.

Raw evidence retains the rejected stale-binary attempt separately: an overlay
archive had mtime zero, Cargo reused a preexisting executable, and the expected
probe/catalog-test output was absent. Those observations are excluded. The
successful retry touches verified source files, recompiles all affected crates
and verifies both the exact test count and probe markers. Subsequent builds use
the corrected overlay procedure.

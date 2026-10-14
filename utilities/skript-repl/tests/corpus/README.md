# Curated native syntax regressions (#103)

This is the first small corpus: 36 Expression/Effect cases across four complete
SSG snapshots. It checks selected identity/pattern, ordered capture slots,
explicit/omitted/null/default state, nested expression kinds, function signature
and parameter-to-child ranges, return/possible types, multiplicity, spans,
tags/marks and diagnostics. It calls the same native library host used by
`SkriptSession`, not the CLI display renderer. No Java or Minecraft server is
needed to run the checked-in tests.

`runner.rs` is a child test module of `session.rs`, so Event setup uses the actual
Event header and private parser context without a new public API. Each case
uses a fresh session and a 32 MiB worker stack; state and a panic do not carry
over to another case. Selected results alone are projected; alternatives and
successful retries are not inserted into the selected tree.

## Data and evidence

- `cases.json` pins each snapshot/schema. It preserves the documentation owner
  Definition/Registration, example index and raw text separately from the parser
  selected identity. Adaptations and explicit Expression controls are labelled.
  Event/no-Event pairs use the same message. Only curated short inputs and the
  previously investigated teleport are included; this is not a full example
  extraction pipeline or the complete acceptance of #103.
- `expected.json` is a **native regression baseline**. It is not evidence of
  upstream final-tree agreement. A change in identity, parent slot/parameter,
  types, cardinality, default/null or diagnostics fails even if parsing still
  succeeds. Incomplete -> clean improvements are reported and allowed; they do
  not acquire upstream certification.
- Schema 5 historical snapshots lack structured default-expression shapes;
  eight current cases are incomplete rather than silently skipped. Their
  collection completeness has not been recertified with SSG #9.
- The `skript-2.16.2-java25` snapshot is the complete 21-file export from
  final SSG source `f7542b9ba42a54d1eb6478d8ff82ebef7fdb572f`, based on SSG main
  `1b6a796d3f5242d1010b8e61903a5b1d5c526baa`. SSG JAR SHA-256 is
  `1d9fe40725f88ca524e49826efdb8e160e3a7b2bfea9a7ed429253be6e748a14`;
  modified Skript 2.16.2 JAR SHA-256 is
  `8ed7254a69f6fc2a9b665b9d45233480719c31ac1483b5834ee14936e2a10ef3`.
  The export and 32 actual returned-object observations were collected in the
  same boot on 2026-10-14: 29 valid and 3 expected plural-scalar rejections.
  Each exported file preserves its collected bytes. All 32 observed graph
  fields agree with the previous capture; the fresh graphs and final pins are
  paired together, without changing assertions or native expected values.
  Paper 26.2-121 / Java 25.0.4.1 is an experimental integration environment.
  Its prior direct registry/export comparison found 50/50 equal from/to/flags/
  order (89 registry entries include 39 derived chains). That collection
  observation and the new loader/digest/reference/owner tests are separate
  evidence. Neither establishes every domain's runtime collection completeness.

### Final producer adoption

The predecessor fixture is retained in LSP commit
`2ee96a84b68e7894ce72522b7f893bf6a54903c6`. The entire export was replaced together
with its pairing pins; individual generated fields were not patched or reordered.
The other three historical fixtures (60 files), `expected.json`, and the corpus
runner are unchanged.

| Provenance | Predecessor | Adopted final pair |
| --- | --- | --- |
| SSG source | `50fab8a63a5cf656d6822d5a7c73e5dfca4d99aa` | `f7542b9ba42a54d1eb6478d8ff82ebef7fdb572f` |
| SSG JAR SHA-256 | `8fc84d9b93a33e0d14ed1273e21f8459891119d816e1c6e6369f0fba4b549bb2` | `1d9fe40725f88ca524e49826efdb8e160e3a7b2bfea9a7ed429253be6e748a14` |
| Snapshot ID | `f42091ee442583229f11a2deea6b56ec736acb0c094ac4f4f01ab36b72192ba8` | `b7c14c3cf6b8864973c14d5cfa773334e9e502013e5c8fae70b3afc9506be04d` |
| Content digest | `268755897ac449e24243b3111242e5efbb3b998aaec676f807385d3e127e43ac` | `52f5d2b32f91cee47c12898fb3b274914581d4d9a232c1c5a6bf984f9daa8898` |
| Generated at (UTC) | `2026-10-13T10:29:53.291196097Z` | `2026-10-14T01:27:53.585464282Z` |
| Positive source registrations | 224 (208 direct property-family sources) | 204 (196 direct property-family sources) |

Final conservative analyzer hardening removed 20 optional source fields whose
flows it no longer proves. It added no new positive source or replacement source.
World/YawPitch retain explicit parsed input 0. Registration IDs and all existing
per-registration fields agree after comparing Types `before`/`after` as sets and
accounting for observed registry order. Fresh-JVM `registrationOrder` changes in
Conditions/Effects/Events/Expressions/Structures and generated Types set order
explain the remaining data churn; these raw orders remain intact. The manifest
records the final producer hash, generation time, digest and snapshot ID.

The seven original raw graphs are byte-equal after serializing the new capture;
the 25 additional raw observations agree field-for-field. Only their snapshot
and producer pins change. The 383 upstream checks, 1,355 control assertions and
16 exact ClassInfo difference fields retain their previous values and coverage.

The projection deliberately excludes pointer/arena IDs, opaque effects and
timings. Native conversion paths, upstream simplified implementation classes,
opaque public data/values and unimplemented capture adapters are not certified.
Unsupported capture diagnostics/attachments/kinds cause an explicit runner
failure rather than being silently discarded. Existing upstream `selected`
trace includes successful retries and is **not a final-object graph**.

## Partial upstream checks

`upstream-graphs.json` records seven validations against the same modified
Skript JAR as the new snapshot. These are outer `ScriptLoader.loadItems` returned
object lists retained and inspected after `validateScript` returned, not
`ParseTrace.selected` entries. Root object identity selects its accepted init
captures, preserving null slots; physical field edges are recorded separately.
The long teleport has 37 observed nodes. The observer's child adapters read
metadata, function bindings and stored numeric literals; the adapters themselves
do not evaluate Effects or game values. Wrapper/leaf adapters that were not
observed remain explicit unknowns.

The separate `validateScript` call runs `postLoad` and can execute `on load`
triggers. The final 2026-10-14 `server-run.log` records `EffMessage.execute` and
a runtime `NullPointerException` in the `location(...)` call while unboxing a
null `Float` from `FunctionArguments.getOrDefault` (`DefaultFunctions.java:493`).
That input still has response `valid=true` and graph `validationException=null`.
Those fields describe validation and returned-object observations; they do not
certify error-free runtime execution. The saved execution log retains the NPE.
The comparisons cover observed parse selection, captures, types/cardinality and
returned objects, without certifying runtime function/game values. The observer's
read-only behavior does not make the validation wrapper nonexecuting.

`upstream.json` adds 383 independent observed checks to the same native run.
It verifies root class/pattern, capture positions/default/null, nested function
names and declaration-ordered parameters, omitted/list argument child ranges,
observed return types/cardinality and small integer source ownership. Expression
samples were wrapped in `send ... to console` under `on load`; these selected
builtin samples are context independent. This does not authorize that wrapper
for arbitrary context-sensitive expressions.

Normalization follows actual `conversionSource` edges before comparing the
selected source Expression. Physical consumed type/path is not represented by
the current native model and remains unknown. Function list values are compared
as parameter-owned values; an upstream default `EmptyExpression` means a native
omitted parameter with zero supplied children. IDs and runtime wrapper classes
are not required to have identical implementations.

There are 16 exact known difference fields, **not 16 compatibility passes**:
16 repeated return/possible-type observations use runtime `PlayerClassInfo`
where native Type literals expose the `ClassInfo` abstraction. The three long
teleport properties (`world`, `yaw`, `pitch`) now require observed `Single` and
no longer have a cardinality discrepancy exemption. Each remaining difference's
case/path/value/reason is fixed individually. A
new difference fails; agreement improvements are reported and require native
baseline review. No broad syntax/diagnostic ignore is used. Whole graph agreement,
registration identity lost during simplify, literal metadata implementation
type normalization and missing Direction/leaf adapters are not certified.

`normalize_upstream.py` is an explicit stdlib-only maintenance tool that derives
the checks from the checked-in actual graphs and curated source cases. It never
rewrites raw graphs. It permits only the documented ClassInfo abstraction
discrepancy family; an unexpected mismatch stops generation.
Review every generated case/path/value before accepting the output.

`cardinality-upstream.json` preserves 25 additional raw validations from the
same boot/export, together with 1,355 partial assertions and native negative
guards. It covers single/AND/OR inputs, nested location functions, scalar/list
function arguments and three rejected plural scalar arguments. Selected
registration/class/pattern, capture positions, nested cardinality and function
argument ownership are checked where observable. Four deliberately changed
results demonstrate that these assertions reject cardinality/identity/ownership
regressions. Grouping is normalized to the native grouped child; ExpressionList
conjunction and child order are compared. The upstream argument API represents
these positional values as NAMED, so syntactic suppliedName is not certified.

The omitted-world control still has native Both versus observed Single; explicit
source evidence does not certify defaults. Lore has observed Multiple and native
unknown, preventing an unconditional Single fallback. ClassInfo implementation
normalization and missing adapters remain unknown. See
`docs/expression-cardinality.md` for the propagation contract. Fresh JVM
registration order changed between exports; IDs and per-registration metadata
were compared, and the observed order was retained without hand rewriting.

## Running and updating

Build the bundled CoreLibrary using the normal workspace instructions, then:

```sh
cargo test --locked -p skript-repl --lib session::corpus
```

`snapshot_and_case_provenance` loads all complete snapshots with `ssg::load`;
digest/inventory/reference failures are fatal. The mutation test demonstrates
that twelve changes in actual native selected nodes alter the projection.
The regression loop aggregates failures and proceeds to subsequent fresh
sessions. Missing expected cases and execution/component/setup failures fail.

The ignored `record_baselines` test is an explicit maintenance tool:

```sh
cargo test --locked -p skript-repl --lib session::corpus::record_baselines -- --ignored
```

It writes only after every case returns without an execution/projection failure.
Review the complete diff, source and coverage before accepting a new baseline;
never bless a changed result solely because it is the current parser output.
Generated SSG snapshots must be replaced as complete exports, not hand patched.

The trusted fixed inputs use deterministic parser budgets. The scoped worker's
`join` is not a hard wall timeout and does not recover an abort. A full/untrusted
corpus needs a separately reviewed isolation/cancellation mechanism before
claiming per-case timeout/abort continuation. Multiline classification,
document/section cases, exhaustive Type cases and full upstream graph coverage
remain follow-up scope.

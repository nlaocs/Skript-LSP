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
- The new `skript-2.16.2-java25` snapshot is a complete, unchanged export from
  the paired SSG #9 verification. Source `ab9fb059...` and merged main
  `1b6a796d...` have identical trees. SSG JAR SHA-256 is
  `03b9591d40bf786af0a7f60eb66980131503d1156f92544fc8dea38883c3dedb`;
  modified Skript 2.16.2 JAR SHA-256 is
  `8ed7254a69f6fc2a9b665b9d45233480719c31ac1483b5834ee14936e2a10ef3`.
  Paper 26.2-121 / Java 25.0.4.1 is an experimental integration environment.
  Its prior direct registry/export comparison found 50/50 equal from/to/flags/
  order (89 registry entries include 39 derived chains). That collection
  observation and the new loader/digest/reference/owner tests are separate
  evidence. Neither establishes every domain's runtime collection completeness.

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
The long teleport has 37 observed nodes. Known child adapters read metadata,
function bindings and stored numeric literals; no Effect or game value is
evaluated. Wrapper/leaf adapters that were not observed remain explicit unknowns.

`upstream.json` adds 380 independent observed checks to the same native run.
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

There are 19 exact known difference fields, **not 19 compatibility passes**:
16 repeated return/possible-type observations use runtime `PlayerClassInfo`
where native Type literals expose the `ClassInfo` abstraction, and three long
teleport properties (`world`, `yaw`, `pitch`) remain native `Both` where upstream
`isSingle()` is true. Their case/path/value/reason are fixed individually. A
new difference fails; agreement improvements are reported and require native
baseline review. No broad syntax/diagnostic ignore is used. Whole graph agreement,
registration identity lost during simplify, literal metadata implementation
type normalization and missing Direction/leaf adapters are not certified.

`normalize_upstream.py` is an explicit stdlib-only maintenance tool that derives
the checks from the checked-in actual graphs and curated source cases. It never
rewrites raw graphs. It permits only the documented ClassInfo abstraction and
Both/Single discrepancy families; an unexpected mismatch stops generation.
Review every generated case/path/value before accepting the output.

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

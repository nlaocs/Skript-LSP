# core-library

[日本語](README.ja.md)

`core-library` is the mandatory WebAssembly Component loaded by every
`parser-wasm::ParserHost`. It is the stable home for Skript's built-in parsing
behavior that must use the same addon ABI as third-party parser addons.

## Current Behavior

The component currently provides the integration foundation:

- component ID `nlaocs.core-library`
- WIT package `nlaocs:skript-parser-addon@0.32.0` and ABI `14.0`
- ABI and capability negotiation during `addon.initialize`
- retention of the accepted WIT `RuntimeProfile`, including Skript/Minecraft
  versions and the enabled plugin list
- nine registered subscriptions: a Document health check, ParseStage
  Expression candidates, registered-Expression and Type semantics, Condition,
  Effect, Section, and Structure semantics, plus a low-priority Tree-phase
  options preprocessor
- typed exports for hooks and text, tree, and AST macro interfaces

The manifest requires `parser.hooks`, the five syntax-parser capabilities,
Tree macros, and `parser.state-store`. It optionally consumes
`parser.dynamic-syntax` and `parser.catalog-data` version 2; Text and AST macro
capabilities are not requirements.

The health hook validates its target, phase, and payload, then continues
without modifying the document.

The Expression hook recognizes braced variables, quoted string literals,
finite signed integer/decimal literals, booleans, SSG-supplied finite type literals,
entity-data literals, and generated
`ClassInfo` literals at legal split points. It also resolves built-in dynamic
semantics including `ExprAllBannedEntries`, `ExprAnyOf`, `ExprDefaultValue`,
`ExprCustomModelData`, `ExprElement`, `ExprEntities`, `ExprFromUUID`,
`ExprInput`, `ExprInventoryInfo`, `ExprInventorySlot`,
`ExprJoinSplit`, `ExprParse`, `ExprRandom`, `ExprRandomCharacter`,
`ExprRandomNumber`, `ExprReversedList`, `ExprSets`, `ExprShuffledList`,
`ExprSortedList`, `ExprTernary`, `ExprWhether`,
and the standard `PropExprAmount`, `PropExprCustomName`, `PropExprName`,
`PropExprNumber`, `PropExprScale`, `PropExprSize`, `PropExprValueOf`, and
`PropExprWXYZ` classes. Property handlers are selected from SSG metadata by
the closest assignable source class, matching Skript's property initialization.
It preserves the
host's expected type/plurality contract and returns typed leaf candidates to
the recursive native parser. Registered Expression matching, recursion, and
ranking remain Rust host responsibilities; CoreLibrary owns only the built-in
semantics that cannot be recovered from SSG registration data alone.

The Type hook handles every built-in Type parser through registrations declared
with `kind: Type`, using the same per-registration dispatch available to third-party
addons. The current modules cover string, number, boolean, ItemType, ItemStack,
EntityData, EntityType, EnchantmentType, Timespan, Time, TimePeriod, Experience,
Color, Particle, ClassInfo, and snapshot-backed finite literals. Each handler owns
its registration and receives the active Type's source
record, addon identity, parser class, parse order, and `before`/`after` relations.
`types/entity_type.rs` implements Skript's amount-bearing `EntityType` literal.
`3 creepers` is one
Literal with return class `ch.njol.skript.entity.EntityType` and `Single`
multiplicity, not three Expression nodes. Its metadata records the effective
`entity-type-amount`, the original `entity-type-raw-amount` (`-1` when omitted),
the Type definition/registration IDs, and an `entity-data` JSON object retaining
the nested EntityData metadata. Entity names and plural forms come from the
snapshot. Finite supplier values cover default entity forms, while SSG's ordered
`registeredParserPatterns` preserve runtime EntityData variants such as age or
powered state without a version-specific hardcoded name table. Patterns containing
typed or regex captures are deferred until a provider can evaluate those captures.
The host namespaces these metadata keys with `nlaocs.core-library/`; keys inside
the nested `entity-data` JSON retain their original names. Type-produced literals
are considered after registered Expressions by default. Quoted and interpolated
strings explicitly request the earlier phase used by Skript's VariableString parser.
This does not implement default argument resolution or environment-backed parsers
such as live Minecraft registries.
Snapshots generated before `registeredParserPatterns` cannot recover non-supplier
EntityData variants. When snapshot data is otherwise insufficient, a Type parser reports a structured
unresolved result with the missing provider instead of guessing or rejecting
the input as invalid.

Quoted strings and variables containing `%expression%` issue generic
`host.expression` parse requests. The host parses those ranges transactionally
and invokes CoreLibrary again with result graphs. CoreLibrary references the
host-issued result tokens from its leaf candidate, so the selected roots become
native child AST nodes with rebased spans instead of opaque metadata.

The built-in variable parser publishes `public_data` separately from
owner-protected `metadata`. Its schema is `nlaocs.skript.variable` version `1`:

```json
{"scope":"local","name":[{"kind":"text","text":"money"},{"kind":"expression","childIndex":0}]}
```

`scope` is `local` or `global`. `name` is a source-name template. Text parts
preserve source spelling, including escaped `%%`, while expression parts refer
to existing children of the originating semantic Expression node through
`childIndex`; they do not duplicate a child's return type or multiplicity.
The data is node-local, so a `Grouped` wrapper does not copy the child's
records. The source-name text is semantic information: changing it does not
rewrite the original source, and the CLI/report must keep it on its own node.

The host validates only the public-data envelope (unique schema ID per list,
schema version at least `1`, and a JSON object). It does not validate
VariableData semantic consistency or derive type/multiplicity from its JSON.
Editors and addons must keep the name template, child indexes, return type,
and multiplicity consistent; changing a list shape requires updating the
standard multiplicity field. This is parse-time semantic data, not a runtime
variable value or a shared `StateStore` entry. Variable type tracking and
server-side variable value mutation are not implemented, and public-data
changes do not retroactively edit a whole AST. Variables intentionally remain a
ParseStage Expression provider rather than pretending to be a registered Type parser.

The Effect and Section hooks provide class-specific semantics including
`EffChange`, `EffDoIf`, `EffSort`, `EffTransform`, `EffSecShoot`, `EffSecSpawn`,
`SecConditional`, `SecFilter`, `SecLoop`, `SecWhile`, and `SecCatchErrors`, as
well as version, platform, and event-context guards. Sort and transform mapping captures are parsed as nested
Expressions with an InputSource context that is visible only inside the
mapping. Property Expressions
publish an owned `change-contract` assembled from `Properties.json` and, when
Skript requires change-in-place propagation, the already parsed source
Expression's contract. `EffChange` consumes that metadata first and falls back
to raw `Expressions.json` or `EventValues.json` records. It validates
`acceptChange(SET)` types and multiplicity without parsing either child twice.
Unresolved SSG contracts produce a warning instead of a guessed error;
missing EventValue changer data is unresolved as well. The metadata envelope is
schema-versioned and bound to its Expression identity. Property candidates keep
their SSG registration, owner, handler, type, and source identities. An earlier
addon hook may select candidate indexes; CoreLibrary refuses an ambiguity with
no explicit selection instead of merging unrelated addons. Raw changer lookups
are bounded by record/byte limits and a bounded cache. Variable type history
remains intentionally deferred.

The Structure hook implements `StructEvent`, `StructFunction`, and
`StructCommand`. It claims semantic captures through registered handler IDs,
selects Trigger or EntryValidator body parsing, and derives Event context from
the captured SSG Event data. `StructFunction` publishes a
`document-function` declaration and may request host Expression parses for
default values; its body uses `FunctionEvent` context. `StructCommand` uses
`ScriptCommandEvent` context for command defaults. Structure matching,
`NodeType`, EntryValidator, and RawTree traversal remain native parser
responsibilities. Third-party addons can implement their own Structure
internals through the same hook without a CoreLibrary change.

Text and AST macro exports currently return `unsupported-capability`. The Tree
macro export is implemented only for the low-priority CoreLibrary options
preprocessor: it performs one-pass `{@...}` replacement on Simple and Section
nodes, preserves replaced Section children, and emits undefined-option
diagnostics. Generated nodes re-enter the Tree phase; this is not a general
purpose CoreLibrary tree-macro API. Function-call matching remains in the
native parser, while `StructFunction` only contributes the document-function
declaration. Version-gated legacy Structure registrations are installed when
the optional dynamic-syntax capability is available.

Initialization requires a non-empty, parseable `runtime.skript-version`. When
`ParserHost::new` receives an SSG-backed `syntax_catalog`, it automatically
fills missing RuntimeProfile fields from that Catalog before initialization;
callers do not need to duplicate the Skript version. With neither a source
Catalog nor an explicit profile version, the default configuration therefore
fails CoreLibrary initialization. Version-sensitive handlers may reject
unsupported syntax or return unresolved diagnostics.

## Why It Is a WASM Component

CoreLibrary is required, but it deliberately uses the same WIT world as addon
components. This keeps core parsing behavior and addon overrides on one
dispatch model:

- the same typed payloads
- the same capability negotiation
- the same resource limits and trap handling
- the same transactional StateStore
- the same dynamic syntax registration API
- the same complete read-only SSG Catalog API

The host treats the component ID specially: startup fails when CoreLibrary is
missing or has the wrong ID, and `ParserHost::unload_addon` refuses to unload
it.

## Source Layout

`src/lib.rs` generates guest bindings from `../parser-wasm/wit` and implements
all interfaces exported by the `parser-addon` world. Built-in syntax behavior
is grouped by syntax kind under `src/expressions`, `src/effects`,
`src/sections`, and `src/structures`. Candidate-end iteration and common candidate construction live in
`src/expression_candidates.rs`. Parser primitives live under `src/primitives`,
while ClassInfo-backed and catalog-backed type literals live under `src/types`.
Each class-specific implementation keeps the Skript Java class name in snake case,
for example `PropExprWXYZ.java` maps to
`expressions/prop_expr_wxyz.rs`; that file owns both handler registration and
semantic resolution. Each directory's `mod.rs` only dispatches handlers and
contains behavior genuinely shared by multiple classes.

The crate uses two crate types:

- `cdylib` produces the core Wasm module.
- `rlib` allows native unit tests for manifest and hook behavior.

Guest builds depend on `parser-wasm` with `default-features = false`. This
reuses ABI constants and compatibility validation without compiling Wasmtime
into the component.

## Build Pipeline

Do not publish the raw core Wasm module directly. The workspace task builds it,
embeds WIT metadata, converts it to a Component Model artifact, validates its
five exported interfaces, and writes the file consumed by the root crate:

```sh
rustup target add wasm32-unknown-unknown
cargo run -p xtask --locked -- build-core-library
```

Output:

```text
artifacts/core-library.wasm
```

The artifact is generated locally and is not committed.

## Testing

Native contract tests:

```sh
cargo test -p core-library --locked
```

Host integration requires the built component:

```sh
cargo run -p xtask --locked -- build-core-library
cargo test -p parser-wasm --test host --locked
cargo test -p skript-lsp --locked
```

When adding built-in behavior, keep the manifest capability list, WIT
subscription, host capability advertisement, and integration tests in sync.

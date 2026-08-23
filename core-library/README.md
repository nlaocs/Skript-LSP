# core-library

[日本語](README.ja.md)

`core-library` is the mandatory WebAssembly Component loaded by every
`parser-wasm::ParserHost`. It is the stable home for Skript's built-in parsing
behavior that must use the same addon ABI as third-party parser addons.

## Current Behavior

The component currently provides the integration foundation:

- component ID `nlaocs.core-library`
- ABI and capability negotiation during `addon.initialize`
- retention of the accepted WIT `RuntimeProfile`, including Skript/Minecraft
  versions and the enabled plugin list
- one core.health-check subscription at the Document phase
- one core.expression-candidates Transform subscription for primitive and registered Expression semantics
- Effect, Section, and Structure subscriptions for class-specific semantics
- typed exports for hooks and text, tree, and AST macro interfaces

The health hook validates its target, phase, and payload, then continues
without modifying the document.

The Expression hook recognizes braced variables, quoted string literals,
finite signed integer/decimal literals, booleans, SSG-supplied finite type literals,
entity-data literals, and generated
`ClassInfo` literals at legal split points. It also resolves the built-in
dynamic semantics of `ExprAllBannedEntries`, `ExprAnyOf`, `ExprDefaultValue`,
`ExprCustomModelData`, `ExprElement`, `ExprEntities`, `ExprFromUUID`,
`ExprInventoryInfo`, `ExprInventorySlot`,
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

Quoted strings and variables containing `%expression%` issue generic
`host.expression` parse requests. The host parses those ranges transactionally
and invokes CoreLibrary again with result graphs. CoreLibrary references the
host-issued result tokens from its leaf candidate, so the selected roots become
native child AST nodes with rebased spans instead of opaque metadata.

The Effect and Section hooks provide the class-specific semantics for
`EffChange`, `EffDoIf`, `SecConditional`, and `SecWhile`. `EffChange` uses the
already parsed child summaries to reject assigning an always-multiple value to
a single variable, matching Skript's `acceptChange(SET)` check without parsing
the child twice.

The Structure hook implements `StructEvent`, `StructFunction`, and
`StructCommand`. It claims semantic captures through registered handler IDs,
selects Trigger or EntryValidator body parsing, and derives Event context from
the captured SSG Event data. Structure matching, `NodeType`, EntryValidator,
and RawTree traversal remain native parser responsibilities. Third-party addons
can implement their own Structure internals through the same hook without a
CoreLibrary change.

Text, tree, and AST macro exports currently return `unsupported-capability`.
Function-call matching remains in the native parser; legacy-specific semantics
are not implemented.

## Why It Is a WASM Component

CoreLibrary is required, but it deliberately uses the same WIT world as addon
components. This keeps core parsing behavior and addon overrides on one
dispatch model:

- the same typed payloads
- the same capability negotiation
- the same resource limits and trap handling
- the same transactional StateStore
- the same dynamic syntax registration API

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

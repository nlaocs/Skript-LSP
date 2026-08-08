# core-library

[日本語](README.ja.md)

`core-library` is the mandatory WebAssembly Component loaded by every
`parser-wasm::ParserHost`. It is the stable home for Skript's built-in parsing
behavior that must use the same addon ABI as third-party parser addons.

## Current Behavior

The component currently provides the integration foundation:

- component ID `nlaocs.core-library`
- ABI and capability negotiation during `addon.initialize`
- one core.health-check subscription at the Document phase
- one core.expression-leaves Transform subscription for leaves and registered Expression semantics
- typed exports for hooks and text, tree, and AST macro interfaces

The health hook validates its target, phase, and payload, then continues
without modifying the document.

The Expression hook recognizes braced variables, quoted string literals,
finite signed integer/decimal literals, and generated `ClassInfo` literals at
legal split points. It also resolves context-dependent return metadata for
`PropExprSize` and both forms of `ExprParse`. It preserves the
host's expected type/plurality contract and returns typed leaf candidates to
the recursive native parser. Registered Expression matching, recursion, and
ranking remain Rust host responsibilities; CoreLibrary owns only the built-in
semantics that cannot be recovered from SSG registration data alone.

Text, tree, and AST macro exports currently return `unsupported-capability`.
CoreLibrary does not yet implement Function calls, Condition, Section,
Structure, or legacy parsing semantics.

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
all interfaces exported by the `parser-addon` world.

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

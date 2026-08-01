# xtask

[日本語](README.ja.md)

`xtask` contains repository build operations that are easier to express in
Rust than in platform-specific shell scripts. It is a developer and CI utility,
not a runtime dependency of the LSP.

## Commands

### build-core-library

```sh
cargo run -p xtask --locked -- build-core-library
```

Builds the mandatory [`core-library`](../core-library/) guest and writes:

```text
artifacts/core-library.wasm
```

The root `skript-lsp` crate embeds this exact path with `include_bytes!`, so the
artifact must exist before compiling that package.

### build-test-components

```sh
cargo run -p xtask --locked -- build-test-components
```

Builds test-only guest components, currently:

```text
artifacts/dynamic-syntax-addon.wasm
artifacts/effect-addon.wasm
artifacts/matching-addon.wasm
artifacts/text-macro-addon.wasm
artifacts/tree-macro-addon.wasm
```

Parser host integration tests embed these artifacts.

## Component Build Pipeline

Both commands use the same `ComponentSpec` pipeline:

1. run Cargo for `wasm32-unknown-unknown`
2. use the optimized `core-library` workspace profile
3. place intermediate files in a component-specific target directory
4. read the raw core Wasm module
5. use `wit-component::ComponentEncoder` to embed component metadata
6. validate the encoded Component
7. require exactly the parser-addon exports
8. atomically replace the generated artifact through a temporary file

The expected exports are:

- `addon`
- `hooks`
- `text-macro`
- `tree-macro`
- `ast-macro`

Build failure, missing metadata, an invalid component, or a different export
set fails the task before an artifact is published.

## Prerequisites

Install the guest target once:

```sh
rustup target add wasm32-unknown-unknown
```

Generated artifacts are intentionally ignored by Git. CI rebuilds them before
running workspace tests.

## Adding a Test Component

When another real guest fixture is needed:

1. add its crate to the workspace
2. generate bindings from `parser-wasm/wit`
3. depend on `parser-wasm` with `default-features = false`
4. add a `ComponentSpec` with package, module, artifact, and display names
5. include it in the appropriate build command
6. add integration coverage that embeds the artifact
7. keep the CI build step before `cargo test`

Do not bypass `validate_component`; native guest tests alone cannot prove that
the produced file is a valid Component with the expected world exports.

## Testing

```sh
cargo test -p xtask --locked
cargo run -p xtask --locked -- build-core-library
cargo run -p xtask --locked -- build-test-components
```

The build commands themselves are the important integration checks. The full
workspace test verifies that every consumer can embed and instantiate their
outputs.

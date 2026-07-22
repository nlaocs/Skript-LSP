# Skript-LSP

## Build

The LSP binary contains the mandatory CoreLibrary WebAssembly component. Build
the component before compiling or testing the complete workspace:

```sh
rustup target add wasm32-unknown-unknown
cargo run -p xtask --locked -- build-core-library
cargo test --workspace --all-features --locked
```

The build task compiles `core-library`, converts its core Wasm module to a
Component Model artifact, validates its exported parser-addon interfaces, and
writes `artifacts/core-library.wasm`. This generated file is embedded into
the LSP at compile time and is not committed.

A missing artifact is a build error because the parser is not supported without
CoreLibrary.

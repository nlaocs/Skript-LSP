# dynamic-syntax-addon

[日本語](README.ja.md)

`dynamic-syntax-addon` is a test-only WebAssembly Component for
`parser-wasm`. It is not a production addon and is not embedded in the LSP
binary.

The fixture provides an end-to-end guest implementation for behavior that
cannot be adequately tested by calling the Rust registry alone.

## Exercised Behavior

During `addon.initialize`, the component:

- requires `parser.hooks` and `parser.dynamic-syntax`
- registers a dynamic Effect named `initial-effect`
- overrides the legacy fixture's static Delay Effect by definition ID

During its Document prepass hook, the component:

- replaces any previous `prepass-effect`
- registers a document-specific dynamic Effect
- places it after `initial-effect`
- returns a typed rejection when the document text is exactly `reject`

The rejection path verifies that dynamic registry writes roll back together
with the parser transaction. Host integration tests also verify freeze and
component unload behavior.

The Delay definition ID is intentionally tied to the checked-in Skript 2.6.4
on Minecraft 1.12.2 SSG fixture. It is test data, not a stable ID applications
should copy.

## ABI Shape

The crate generates guest bindings directly from `../../parser-wasm/wit` and
implements every export required by the `parser-addon` world.

Hook behavior is implemented. Text, tree, and AST macro exports return
`unsupported-capability`, because this fixture targets dynamic syntax rather
than macro execution.

Like CoreLibrary, it depends on `parser-wasm` with
`default-features = false` so the guest reuses compatibility constants without
linking Wasmtime.

## Building

Use the workspace task:

```sh
rustup target add wasm32-unknown-unknown
cargo run -p xtask --locked -- build-test-components
```

Output:

```text
artifacts/dynamic-syntax-addon.wasm
```

The generated component is not committed. `xtask` validates that it exports
the complete parser-addon interface set before publishing the artifact.

## Testing

Native compilation checks the guest bindings:

```sh
cargo test -p dynamic-syntax-addon --locked
```

The meaningful lifecycle assertions run in the native Wasmtime host:

```sh
cargo run -p xtask --locked -- build-core-library
cargo run -p xtask --locked -- build-test-components
cargo test -p parser-wasm --test dynamic_syntax --locked
```

When the WIT world changes, rebuild both CoreLibrary and this fixture before
running workspace tests.

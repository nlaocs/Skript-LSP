# Effect Hook Test Addon

[日本語](README.ja.md)

`effect-addon` is a test-only WebAssembly Component for the typed Effect
lifecycle ABI. It subscribes to the Effect category and two exact DummyAddon
registrations. One hook replaces candidate metadata, while the other rejects a
matched Effect with a diagnostic.

Every path writes Parse-scoped private state. Host integration tests assert
that selected replacement state remains, while rejected or unknown Effect
state is restored together with nested Expression work.

Build it with all other fixtures:

```sh
cargo run -p xtask --locked -- build-test-components
```

The generated `artifacts/effect-addon.wasm` file is not committed.
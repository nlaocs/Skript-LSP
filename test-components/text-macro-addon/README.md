# Text Macro Test Addon

[日本語](README.ja.md)

`text-macro-addon` is a test-only WebAssembly component for the
`parser-wasm` Text macro pipeline. It is not an example of a production addon
and is not loaded by the LSP executable.

## Behaviors

The component declares two `ParseStage` / `Preprocess` / `Transform`
subscriptions with different priorities. Together they exercise:

- deterministic multi-stage expansion from `alpha` to `stage-one` to `二段目`
- StateStore writes and per-call read/write-set reporting
- an edit whose range is inside a multibyte UTF-8 character
- diagnostics mapped through a prior expansion, including related spans
- parse-request spans mapped through a prior expansion
- whole-pipeline rejection with an EOF diagnostic and opening-site relation
- late rejection without orphaned call or diagnostic expansion references
- rejection discarding context updates and parse requests from earlier calls
- rollback of a diagnostic whose range splits a UTF-8 character
- rollback of a parse request whose range splits a UTF-8 character
- a guest trap
- generated text with an explicit anchor

The host integration tests use these trigger strings deliberately. Changing
them requires updating `parser-wasm/tests/text_macro.rs`.

## Build

Build the Component through `xtask`:

```sh
cargo run -p xtask --locked -- build-test-components
```

The output is:

```text
artifacts/text-macro-addon.wasm
```

The raw `wasm32-unknown-unknown` module is converted to a Component, validated,
and checked for the complete parser-addon export set before publication.

## Testing

```sh
cargo test -p text-macro-addon --locked
cargo test -p parser-wasm --test text_macro --locked
```

The first command checks the manifest in native Rust. The second instantiates
the actual generated Component and verifies ordering, SourceMap composition,
ExpansionGraph provenance, primary and related diagnostic and parse-request
mapping, UTF-8 rejection, transactional effect rollback, StateStore rollback,
quotas, anchors, and trap handling.

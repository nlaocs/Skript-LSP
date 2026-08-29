# Catalog Data Test Addon

`catalog-data-addon` is a test-only WebAssembly Component that calls the full
`parser.catalog-data` import from guest code. Its document hook selects a
scenario from the document text and reports the assertions as a diagnostic.

The integration tests cover source metadata, paged document and ID indexes,
bounded document/record chunks, unknown JSON fields, duplicate IDs, class
relations, converter queries, missing source handling, capability advertisement,
and response quotas.

The quota scenarios deliberately keep the 8-byte page rejection and add a
64-byte case where the guest reconstructs both a complete document and an
indexed record from multiple chunks.

Build it with the other fixtures:

```sh
cargo run -p xtask --locked -- build-test-components
```

The generated `artifacts/catalog-data-addon.wasm` is intentionally ignored by
Git.

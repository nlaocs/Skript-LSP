# Catalog Data テストAddon

`catalog-data-addon` は、guest側のWASM componentから
`parser.catalog-data` importを実際に呼び出すテスト専用componentです。
document hookに渡された文字列でシナリオを選び、検証結果をdiagnosticとして返します。

source metadata、documentとID indexのpage、document/recordのchunk、未知のJSON field、
重複ID、class関係、converter query、source未提供、capability広告、response quotaを検証します。
8 byteでpageが拒否されるケースは残し、64 byteではdocumentとindexed recordを複数chunkから
guest側で最後まで再構成できることも検証します。

他のfixtureと同じ方法でビルドします。

```sh
cargo run -p xtask --locked -- build-test-components
```

生成される `artifacts/catalog-data-addon.wasm` はGit管理対象外です。

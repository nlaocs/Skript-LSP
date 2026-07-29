# Text Macro Test Addon

[English](README.md)

`text-macro-addon`は、`parser-wasm`のText macro pipeline専用のtest WebAssembly
componentです。production addonの実装例ではなく、LSP executableからも読み込みません。

## 動作

異なるpriorityを持つ2つの`ParseStage` / `Preprocess` / `Transform` subscriptionを宣言し、
次の動作をまとめて検証します。

- `alpha`から`stage-one`、さらに`二段目`へ進む決定的なmulti-stage展開
- StateStore writeとcallごとのread/write set
- multibyte UTF-8 characterの途中を指す不正なedit range
- related spanを含む、前段展開を経由したdiagnostic mapping
- 前段展開を経由したparse request spanのmapping
- EOF diagnosticとopening位置の関連情報を持つpipeline全体のReject
- 後段Reject後にcallやdiagnosticへ孤立したexpansion参照が残らないこと
- Reject時に前段callのcontext updateとparse requestが破棄されること
- UTF-8 character途中を指すdiagnosticのrollback
- UTF-8 character途中を指すparse requestのrollback
- guest trap
- 明示的なanchorを持つ生成text

host integration testはこれらのtrigger文字列を意図的に使用します。変更する場合は
`parser-wasm/tests/text_macro.rs`も更新してください。

## Build

`xtask`経由でComponentをbuildします。

```sh
cargo run -p xtask --locked -- build-test-components
```

出力:

```text
artifacts/text-macro-addon.wasm
```

raw `wasm32-unknown-unknown` moduleをComponentへ変換し、完全なparser-addon export setを
持つことまで検証してから配置します。

## テスト

```sh
cargo test -p text-macro-addon --locked
cargo test -p parser-wasm --test text_macro --locked
```

1つ目はnative Rust上でmanifestを検証します。2つ目は実際の生成Componentをinstantiateし、
実行順、SourceMap合成、ExpansionGraphの出典、primary/related diagnostic mapping、
parse request mapping、UTF-8拒否、transactional effect rollback、StateStore rollback、
quota、anchor、trap処理を検証します。

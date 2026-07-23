# xtask

[English](README.md)

`xtask`は、platform固有shell scriptよりRustで表現しやすいrepositoryのbuild操作を
まとめます。developer/CI用utilityであり、LSPのruntime dependencyではありません。

## Command

### build-core-library

```sh
cargo run -p xtask --locked -- build-core-library
```

必須の[`core-library`](../core-library/README.ja.md) guestをbuildし、次へ出力します。

```text
artifacts/core-library.wasm
```

ルートの`skript-lsp` crateは`include_bytes!`でこのpathを埋め込むため、packageをcompile
する前にartifactが存在しなければなりません。

### build-test-components

```sh
cargo run -p xtask --locked -- build-test-components
```

test専用guest componentをbuildします。現在の出力は次のとおりです。

```text
artifacts/dynamic-syntax-addon.wasm
```

parser hostのintegration testがこのartifactを埋め込みます。

## Component build pipeline

両commandは共通の`ComponentSpec` pipelineを使います。

1. `wasm32-unknown-unknown`向けにCargoを実行する
2. workspaceの最適化済み`core-library` profileを使用する
3. component固有のtarget directoryへ中間fileを配置する
4. raw core Wasm moduleを読む
5. `wit-component::ComponentEncoder`でcomponent metadataを埋め込む
6. encode済みComponentを検証する
7. parser-addonのexportだけが正確に存在することを要求する
8. temporary fileを経由して生成artifactをatomicに置き換える

必要なexport:

- `addon`
- `hooks`
- `text-macro`
- `tree-macro`
- `ast-macro`

build失敗、metadata欠落、不正Component、異なるexport setがある場合、artifactを配置する前に
taskが失敗します。

## 前提

guest targetを一度installします。

```sh
rustup target add wasm32-unknown-unknown
```

生成artifactは意図的にGit管理外です。CIはworkspace testより前に再buildします。

## Test componentの追加

実guest fixtureが必要になった場合:

1. crateをworkspaceへ追加する
2. `parser-wasm/wit`からbindingを生成する
3. `parser-wasm`へ`default-features = false`で依存する
4. package、module、artifact、display nameを持つ`ComponentSpec`を追加する
5. 適切なbuild commandへ追加する
6. artifactを埋め込むintegration coverageを追加する
7. CIのbuild stepを`cargo test`より前に置く

`validate_component`を迂回しないでください。native guest testだけでは、生成fileが正しい
Componentであり、期待したworld exportを持つことを証明できません。

## テスト

```sh
cargo test -p xtask --locked
cargo run -p xtask --locked -- build-core-library
cargo run -p xtask --locked -- build-test-components
```

build command自体が重要なintegration checkです。workspace全体のtestでは、すべてのconsumerが
出力artifactを埋め込み、instantiateできることを確認します。

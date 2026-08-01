# core-library

[English](README.md)

`core-library`は、すべての`parser-wasm::ParserHost`が読み込む必須のWebAssembly
Componentです。third-party parser addonと同じABIを使う必要がある、Skript標準の
解析処理を実装するための安定した配置場所です。

## 現在の動作

現在は統合の基礎として次を提供します。

- component ID `nlaocs.core-library`
- `addon.initialize`におけるABIとcapabilityのnegotiation
- Document phaseのcore.health-check subscription 1件
- variable・string・number用のcore.expression-leaves Transform subscription 1件
- hook、text macro、tree macro、AST macro interfaceの型付きexport

health hookはtarget、phase、payloadを検証したあと、documentを変更せず処理を継続します。

Expression hookは合法split位置にあるbrace付きvariable、quoted string literal、有限の符号付き
integer/decimal literalを認識します。hostから渡されたexpected type/plural contractを維持し、
再帰native parserへ型付きleaf候補を返します。登録Expressionの照合、再帰、順位付けはRust hostの
責務です。

text、tree、AST macroのexportは、現時点では`unsupported-capability`を返します。CoreLibraryは、
Function call、Condition、Section、Structure、legacy解析の意味処理をまだ実装していません。

## WASM Componentである理由

CoreLibraryは必須ですが、意図的にaddon componentと同じWIT worldを使用します。これにより、
標準の解析処理とaddonによるoverrideを、1つのdispatch model上で扱えます。

- 同じ型付きpayload
- 同じcapability negotiation
- 同じresource limitとtrap処理
- 同じtransactional StateStore
- 同じdynamic syntax registration API

hostはcomponent IDを特別に扱います。CoreLibraryがない場合やIDが異なる場合は起動に失敗し、
`ParserHost::unload_addon`によるunloadも拒否します。

## Source構成

`src/lib.rs`が`../parser-wasm/wit`からguest bindingを生成し、`parser-addon` worldがexport
するすべてのinterfaceを実装します。

crate typeは2種類あります。

- `cdylib`: core Wasm moduleを生成します。
- `rlib`: manifestとhook動作をnative unit testで検証できるようにします。

guest buildでは、`parser-wasm`へ`default-features = false`で依存します。これにより、
Wasm componentへWasmtimeをcompileせず、ABI定数と互換性検証だけを再利用します。

## Build pipeline

raw core Wasm moduleを直接配布しないでください。workspace taskはmoduleをbuildし、WIT
metadataを埋め込み、Component Model artifactへ変換し、5つのexport interfaceを検証して
ルートcrateが使用するfileへ出力します。

```sh
rustup target add wasm32-unknown-unknown
cargo run -p xtask --locked -- build-core-library
```

出力:

```text
artifacts/core-library.wasm
```

artifactはlocalで生成され、commitされません。

## テスト

native contract test:

```sh
cargo test -p core-library --locked
```

host integrationにはbuild済みcomponentが必要です。

```sh
cargo run -p xtask --locked -- build-core-library
cargo test -p parser-wasm --test host --locked
cargo test -p skript-lsp --locked
```

標準処理を追加するときは、manifestのcapability一覧、WIT subscription、hostのcapability
advertisement、integration testを同期させてください。

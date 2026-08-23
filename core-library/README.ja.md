# core-library

[English](README.md)

`core-library`は、すべての`parser-wasm::ParserHost`が読み込む必須のWebAssembly
Componentです。third-party parser addonと同じABIを使う必要がある、Skript標準の
解析処理を実装するための安定した配置場所です。

## 現在の動作

現在は統合の基礎として次を提供します。

- component ID `nlaocs.core-library`
- `addon.initialize`におけるABIとcapabilityのnegotiation
- Skript/Minecraft versionと有効plugin一覧を含むWIT `RuntimeProfile`の保持
- Document phaseのcore.health-check subscription 1件
- primitive候補と登録Expressionの意味解析用core.expression-candidates Transform subscription 1件
- class固有の意味処理を行うEffect、Section、Structure subscription
- hook、text macro、tree macro、AST macro interfaceの型付きexport

health hookはtarget、phase、payloadを検証したあと、documentを変更せず処理を継続します。

Expression hookは合法split位置にあるbrace付きvariable、quoted string literal、有限の符号付き
integer/decimal literal、boolean、SSG由来の有限type literal、entity-data literal、生成された
`ClassInfo` literalを認識します。
また、`ExprAllBannedEntries`、`ExprAnyOf`、`ExprCustomModelData`、`ExprDefaultValue`、
`ExprElement`、`ExprEntities`、`ExprFromUUID`、`ExprInventoryInfo`、
`ExprInventorySlot`、`ExprJoinSplit`、`ExprParse`、`ExprRandom`、
`ExprRandomCharacter`、`ExprRandomNumber`、`ExprReversedList`、`ExprSets`、
`ExprShuffledList`、`ExprSortedList`、`ExprTernary`、`ExprWhether`と、標準の
`PropExprAmount`、`PropExprCustomName`、`PropExprName`、`PropExprNumber`、
`PropExprScale`、`PropExprSize`、`PropExprValueOf`、`PropExprWXYZ`について、
動的な意味と返値metadataを解決します。property handlerはSSG metadataからsource classに
最も近いassignable classを選び、Skriptのproperty初期化規則に合わせます。hostから渡された
expected type/plural contractを維持し、
再帰native parserへ型付きleaf候補を返します。登録Expressionの照合、再帰、順位付けはRust hostの
責務です。CoreLibraryはSSGの登録dataだけから復元できない標準の意味処理だけを所有します。

`%expression%`を含むquoted stringとvariableは、汎用`host.expression` parse requestを返します。
hostは対象rangeをtransaction内で解析し、result graph付きでCoreLibraryを再度呼び出します。CoreLibraryは
hostが発行したresult tokenをleaf候補から参照するため、選択されたrootはopaque metadataではなく、
外側へspanを再配置したnative child ASTになります。

EffectとSection hookは`EffChange`、`EffDoIf`、`SecConditional`、`SecWhile`固有の意味処理を提供します。
`EffChange`は解析済みchild summaryを使い、常にMultipleな値をsingle variableへ設定する処理を、
子を再解析せずSkriptの`acceptChange(SET)`判定に合わせて拒否します。

Structure hookは`StructEvent`、`StructFunction`、`StructCommand`を実装します。登録handler IDを通じて
意味付きcaptureを取得し、TriggerまたはEntryValidator body解析を選び、取得したSSG Event dataから
Event contextを派生します。Structure照合、`NodeType`、EntryValidator、RawTree走査はnative parserの
責務です。third-party addonも同じhookを使って独自Structureの内部を実装でき、CoreLibraryの変更は
不要です。

text、tree、AST macroのexportは、現時点では`unsupported-capability`を返します。
Function callの照合はnative parserが担当し、legacy固有の意味処理は未実装です。

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
するすべてのinterfaceを実装します。標準構文の処理はsyntax kind別に`src/expressions`、
`src/effects`、`src/sections`、`src/structures`へ配置します。候補終端の反復と候補生成の共通処理は
`src/expression_candidates.rs`に置き、parser primitiveは`src/primitives`、ClassInfoと
catalog由来のtype literalは`src/types`に置きます。クラス固有実装はSkriptのJava class名をsnake caseに
したfileへ置きます。例えば`PropExprWXYZ.java`は`expressions/prop_expr_wxyz.rs`に対応し、
そのfileがhandler登録と意味解決の両方を所有します。各directoryの`mod.rs`はdispatchと、
複数classで本当に共有する処理だけを持ちます。

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

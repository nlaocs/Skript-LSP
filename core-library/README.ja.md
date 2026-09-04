# core-library

[English](README.md)

`core-library`は、すべての`parser-wasm::ParserHost`が読み込む必須のWebAssembly
Componentです。third-party parser addonと同じABIを使う必要がある、Skript標準の
解析処理を実装するための安定した配置場所です。

## 現在の動作

現在は統合の基礎として次を提供します。

- component ID `nlaocs.core-library`
- WIT package `nlaocs:skript-parser-addon@0.30.0`とABI `12.0`
- `addon.initialize`におけるABIとcapabilityのnegotiation
- Skript/Minecraft versionと有効plugin一覧を含むWIT `RuntimeProfile`の保持
- Document health check、ParseStageのExpression候補、登録ExpressionとType、Condition、Effect、Section、
  Structureの意味処理、およびlow-priority Tree phaseのoptions preprocessorからなる9件のsubscription
- hook、text macro、tree macro、AST macro interfaceの型付きexport

manifestは`parser.hooks`、5つのsyntax parser capability、Tree macro、`parser.state-store`を必須とし、
`parser.dynamic-syntax`とversion 2の`parser.catalog-data`を任意で利用します。TextとAST macroは必須ではありません。

health hookはtarget、phase、payloadを検証したあと、documentを変更せず処理を継続します。

Expression hookは合法split位置にあるbrace付きvariable、quoted string literal、有限の符号付き
integer/decimal literal、boolean、SSG由来の有限type literal、entity-data literal、生成された
`ClassInfo` literalを認識します。
また、`ExprAllBannedEntries`、`ExprAnyOf`、`ExprCustomModelData`、`ExprDefaultValue`、
`ExprElement`、`ExprEntities`、`ExprFromUUID`、`ExprInventoryInfo`、
`ExprInput`、`ExprInventorySlot`、`ExprJoinSplit`、`ExprParse`、`ExprRandom`、
`ExprRandomCharacter`、`ExprRandomNumber`、`ExprReversedList`、`ExprSets`、
`ExprShuffledList`、`ExprSortedList`、`ExprTernary`、`ExprWhether`と、標準の
`PropExprAmount`、`PropExprCustomName`、`PropExprName`、`PropExprNumber`、
`PropExprScale`、`PropExprSize`、`PropExprValueOf`、`PropExprWXYZ`などのbuilt-in dynamic semanticsと
返値metadataを解決します。property handlerはSSG metadataからsource classに
最も近いassignable classを選び、Skriptのproperty初期化規則に合わせます。hostから渡された
expected type/plural contractを維持し、
再帰native parserへ型付きleaf候補を返します。登録Expressionの照合、再帰、順位付けはRust hostの
責務です。CoreLibraryはSSGの登録dataだけから復元できない標準の意味処理だけを所有します。

Type hookは、標準Type parserをすべて`kind: Type`の登録として処理し、third-party addonと
同じregistration単位のdispatchを使います。現在のmoduleはstring、number、boolean、ItemType、
EntityData、EntityType、EnchantmentType、Timespan、ClassInfo、Snapshot由来の有限literalを扱います。
各handlerは登録を所有し、active Typeのsource record、addon identity、parser class、parse order、
`before`/`after`関係を受け取ります。`types/entity_type.rs`は数量付き`EntityType`の解析を所有します。
`3 creepers`は`ch.njol.skript.entity.EntityType`を返す1個のLiteralで、多重度は`Single`です。
metadataには実効数量`entity-type-amount`、元の数量`entity-type-raw-amount`（省略時は`-1`）、
Typeのdefinition/registration ID、および内包するEntityDataのsupplier metadataを保持した
`entity-data` JSONを記録します。種類名と複数形はSnapshotから解決し、古いSnapshotでは既存の
version-gatedな互換経路を使用します。hostはmetadataキーに`nlaocs.core-library/`を付けますが、
内部の`entity-data` JSONのキー名は変更しません。Type由来literalは既定で登録Expressionの後に
評価されます。quoted/interpolated stringだけは、SkriptのVariableString parserと同じ早い段階を
明示的に要求します。省略引数の補完やlive Minecraft registryへ依存するparserまで実装したことは意味しません。
有限なSnapshot情報だけでは確定できない場合、Type parserは入力を推測または不正扱いせず、
不足するproviderを含む構造化unresolved結果を返します。

`%expression%`を含むquoted stringとvariableは、汎用`host.expression` parse requestを返します。
hostは対象rangeをtransaction内で解析し、result graph付きでCoreLibraryを再度呼び出します。CoreLibraryは
hostが発行したresult tokenをleaf候補から参照するため、選択されたrootはopaque metadataではなく、
外側へspanを再配置したnative child ASTになります。

標準variable parserはowner-protectedな`metadata`とは別に`public_data`を公開します。
schemaは`nlaocs.skript.variable` version `1`です。

```json
{"scope":"local","name":[{"kind":"text","text":"money"},{"kind":"expression","childIndex":0}]}
```

`scope`は`local`または`global`です。`name`はsource-name templateで、text partは
escaped `%%`を含むsource spellingを保持します。expression partの`childIndex`は
元になったsemantic Expression node自身の既存childを参照し、childのreturn typeや
multiplicityを重複して持ちません。dataはnode-localなので、`Grouped` wrapperはchildの
recordをcopyしません。このsource-nameを編集することはsemantic変更ですが、original
source textは書き換えません。

hostが検証するのはpublic-data envelopeだけです。list内のschema IDは一意で、schema
versionは`1`以上、`json`はJSON objectでなければなりません。VariableDataのsemantic
consistencyは検証せず、JSONからreturn typeやmultiplicityを導出もしません。editorとaddonは
name template、child index、nodeのreturn type、multiplicityを整合させる必要があります。
list shapeを変更する場合は標準のmultiplicity fieldも更新してください。現在のcandidateに
対して許可されたTransform/Override hookはpublic dataを置換・削除でき、caller orderに
従って後続hookが先行hookの変更を受け取ります。

これはparse時のsemantic dataであり、runtime variable valueでもshared `StateStore`の
entryでもありません。variable type trackingとserver側のvariable value mutationは実装
されていません。変更はsource/spanや他nodeを遡及して書き換えず、whole ASTの編集にも
なりません。variableは登録Type parserを装うのではなく、意図的にParseStage Expression providerとして残します。

EffectとSection hookは、`EffChange`、`EffDoIf`、`EffSort`、`EffTransform`、`EffSecShoot`、`EffSecSpawn`、
`SecConditional`、`SecFilter`、`SecLoop`、`SecWhile`、`SecCatchErrors`などのclass固有の意味処理を提供し、
version、platform、event contextのguardも扱います。sortとtransformのmapping captureは、そのmapping内だけで
見えるInputSource contextを使ってnested Expressionとして解析します。
Property Expressionは`Properties.json`と、Skriptがchange-in-placeの書き戻しを要求する場合は解析済みsource
Expressionのcontractからowned `change-contract`を公開します。`EffChange`はこのmetadataを優先し、なければ
`Expressions.json`または`EventValues.json`の生recordへfallbackします。子を再解析せず
`acceptChange(SET)`の型とmultiplicityを検証し、SSG contractがunresolvedなら推測で拒否せずwarningを返します。
EventValueのchanger情報が欠けている場合もunresolvedです。metadata envelopeはschema versionを持ち、
対象Expression identityへ結び付きます。Property候補はSSGの登録、owner、handler、type、source identityを保持し、
先行Addon hookが候補indexを選択できます。明示選択のない複数Property登録は、無関係なAddonを合成せず拒否します。生changer lookupには
record数・byte数上限とbounded cacheがあります。variableの型履歴は意図的に後続実装へ残しています。

Structure hookは`StructEvent`、`StructFunction`、`StructCommand`を実装します。登録handler IDを通じて
意味付きcaptureを取得し、TriggerまたはEntryValidator body解析を選び、取得したSSG Event dataから
Event contextを派生します。`StructFunction`は`document-function` declarationを公開し、default valueの
ためにhost Expression parseを要求することがあり、bodyでは`FunctionEvent` contextを使います。
`StructCommand`のdefaultは`ScriptCommandEvent` contextで解析します。Structure照合、`NodeType`、
EntryValidator、RawTree走査はnative parserの責務です。third-party addonも同じhookを使って独自Structureの
内部を実装でき、CoreLibraryの変更は不要です。

textとAST macroのexportは、現時点では`unsupported-capability`を返します。Tree macro exportは、
low-priorityのCoreLibrary options preprocessor専用に実装されています。SimpleとSection nodeの`{@...}`を
一回だけ置換し、置換したSectionのchildrenを保持し、undefined option diagnosticを出します。生成nodeは
Tree phaseへ再入しますが、これは汎用CoreLibrary tree-macro APIではありません。Function callの照合はnative
parserが担当し、`StructFunction`はdocument-function declarationだけを提供します。optionalな
dynamic-syntax capabilityが使える場合は、version-gatedなlegacy Structure registrationも追加されます。

初期化には空でないparse可能な`runtime.skript-version`が必要です。`ParserHost::new`がSSG由来の
`syntax_catalog`を受け取ると、初期化前にCatalogから未指定のRuntimeProfile fieldを自動補完するため、callerが
Skript versionを重複指定する必要はありません。source Catalogも明示的なprofile versionもないdefault configは
CoreLibrary初期化に失敗します。version依存のhandlerは未対応syntaxを拒否するか、unresolved diagnosticを返す
ことがあります。

## WASM Componentである理由

CoreLibraryは必須ですが、意図的にaddon componentと同じWIT worldを使用します。これにより、
標準の解析処理とaddonによるoverrideを、1つのdispatch model上で扱えます。

- 同じ型付きpayload
- 同じcapability negotiation
- 同じresource limitとtrap処理
- 同じtransactional StateStore
- 同じdynamic syntax registration API
- 同じ完全なread-only SSG Catalog API

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

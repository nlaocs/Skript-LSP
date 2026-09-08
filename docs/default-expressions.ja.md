# 省略された型付き引数

[English](default-expressions.md)

`skript-parser`は、選択されなかったoptional/choice branchを含め、登録patternの
型付きslotを保持します。`[]`はbranchの省略を、`-` flagは引数のnullを許可します。
この2つは異なる意味を持ちます。

| Capture | 結果 |
| --- | --- |
| 明示入力 | `TypeCaptureState::Explicit`。default providerを呼ばない |
| `%-type%`の省略 | `Null`。子Expressionを作らずproviderも呼ばない |
| `%type%`の省略、provider成功 | `Default`。`ExpressionNodeKind::Default`を子に追加 |
| providerが文脈やcontractを拒否 | `DefaultExpressionFailureKind::Rejected`の候補失敗 |
| providerなし、Catalog不足、未対応の意味論 | `Unresolved`の候補失敗 |

単独の構造pattern matcherは`Omitted` slotを保持します。`ExpressionSession`による
意味解析は、後続syntax hookより前に補完します。構造を認識できてもdefaultが無効または
未解決なら、検証済みのmatchにはしません。

## 共通parser API

`ExpressionParseEnvironment::provide_default_expression`は借用した
`DefaultExpressionRequest`を受け取ります。syntaxのdefinition/registrationとpatternの
identity、capture indexとpattern span、Typeと要求多重度、flags/time、補完位置のmapped
anchor、Event/Section contextを渡します。返却値は
`DefaultExpressionDecision::{Resolved, Rejected, Unresolved}`です。parserは返却型、
多重度、literal/expression flags、timeを検証して子を採用します。通常のbranch transactionが
補完した子にも適用されます。
補完失敗時はそのbranchのcheckpointへ戻します。後続hookが拒否した場合は、既存のenvironment
scope transactionが明示・補完両方の子を破棄します。scopeをrollbackした後でparserが
より新しいbranch checkpointへ戻し、破棄済みの状態を復活させることはしません。

`DefaultExpressionInfo`は要求Type identity、provider/component、理由、Event class、
Section scope ID、Catalog参照、mapped anchorを保持します。子nodeは解決後の型、
多重度、public data、namespace付きmetadataを保持します。
`ParsedCaptureSemanticSummary`と`SectionScopeCapture`にも同じimplicit provenanceが
あります。LSPや将来の複数行REPLは、表示文字列からdefaultを再構成せず共通結果を辿れます。

implicit nodeのsource rangeは空です。macro由来の補完位置でもすべてのoriginと
expansion IDを保持し、各originを`Anchored`にします。
存在しない`to player`などを`MappedSource`へ挿入しません。

## WASM Addon contract

WIT `nlaocs:skript-parser-addon@0.35.0`、ABI `17.0`を使用します。
capability `parser.default-expression` version 1を要求し、既存の`HookSubscription`、
target、selectorでphase `default-expression`へ登録します。dispatch対象は要求された
**Type**のregistrationです。親syntaxとcaptureのidentityは
`default-expression-payload`に別途入ります。Typeのdefinition/registration IDやType
selectorを利用でき、registered handler bindingでは正確なparser classから安定した
Catalog registration IDを解決できます。`RegisteredSyntaxHandler`の`phase`にはdefault
providerなら`default-expression`、明示入力のType parserなら`expression`を指定します。
default専用の対応が通常のType解析や診断を変更しないための区別です。

順序は既存と同じくtargetの具体性、priority昇順、component読込順、subscription宣言順です。
複数Addonが同じ結果を参照・変更できます。`NotApplicable`は結果を維持します。
新しいreplacementは`component-id: none`で返し、hostが所有者を設定します。
metadataも通常のcomponent namespaceと所有規則に従い、後続hookから参照できます。

`outcome`へ`resolved(default-expression-resolution)`または`unresolved(reason)`を設定し、
文脈が無効なら既存の`HookDecision::Reject`を使います。identity、要求context、anchorは
read-onlyです。providerの副作用は子の採用まで保留されます。拒否、不正な返却値、trap、
cancel、後続の候補拒否では投機的なmetadata、diagnostic、StateStore変更をrollbackします。
拒否理由に属するdiagnosticは失敗候補へ保持します。
provider失敗時に途中の結果を検証済み成功として残しません。

`RegisteredExpressionChild.default-expression`と`ParseSummary.default-expression`から
後続syntax hookがimplicit provenanceを取得できます。payloadにはCatalog全体ではなく
小さいsource参照を渡します。既存の`catalog-data` queryと、索引化した正確なClassInfo検索
`type-for-class`を利用できます。必要なSSG原文はサイズ制限付き`read-record`で取得します。
subscription routeはphase/targetで索引化し、明示入力とnull許可captureではWASM default
providerを呼びません。

## CoreLibraryの標準provider

`core.default-expression.skript`は、標準の`SimpleLiteral`、`EventValueExpression`、
`ExprDamageCause`実装を扱う最終fallbackです。SSGは実装class、literal flag、return class、
確定した`isSingle()`をimmutable descriptorとして渡します。shapeが欠けた場合は推測せず、
Typeを所有するaddon名にも依存しないため、addon TypeがSkript標準実装を再利用できます。

`SimpleLiteral` defaultはcontext不要のliteralで、0以外のtime stateを拒否します。
`EventValueExpression` defaultはdescriptorのreturn classを対象に、共通Catalogの継承、変換、
除外、曖昧性規則で解決します。AudienceもType classではなく実際の`CommandSender`を対象にします。
`ExprDamageCause`は、past/present表記を許可しfutureを拒否しつつ、present EventValueを参照する
Skript固有の挙動を維持します。

標準providerは、より具体的なaddon/providerの後に実行します。Sectionなどの
`DefaultValueData` override、独自`DefaultExpression` subclass、未知のvalidator、静的shapeが
不足したdescriptorは、所有addonが意味論を提供するまでunresolvedのままです。
`effectcommandcli`のreport schema 7は同じ共通結果を描画します。captureの`state`は
`explicit`、`omitted`、`null`、`default`で、implicit Expressionは`defaultExpression`、空の`source`、
ゼロ幅anchorを持ちます。失敗時にも認識したEffect/patternと、capture index、要求型、
rejected/unresolved状態を持つ`defaultExpression`理由を表示します。sessionは入力間で
SnapshotとWASM hostを保持し、`parseDurationNs`からSnapshot読込とreport描画を除外します。

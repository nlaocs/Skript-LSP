# syntaxes

[English](README.md)

`syntaxes`は、Skriptとaddonが登録したすべての情報に対する、保存形式に依存しないruntime
modelです。serialized SSG dataと、parser/LSP consumerの境界になります。

このcrateはJSONを読み込まず、dataを生成したSkript versionも意識しません。それらは
[`ssg`](../ssg/README.ja.md)が担当し、`Catalog`を構築します。

## Domain model

`Syntax`は、projectで定めた次の8 categoryをこの順序で保持します。

1. Event
2. Condition
3. Effect
4. Expression
5. Type
6. Function
7. Section
8. Structure

`CommonSyntax`はregistration order、documentation、parse済みpattern、addon ownership、
definition ID、registration ID、priority metadata、experimental requirement、
supported event、任意のreturn handlerを保持します。

category固有modelは、Expressionのmultiplicity/changer、Section flag、Structure validator、
EventValue、Function parameter、Type parser metadataなどを追加します。

構文以外にも次をmodel化します。

- Java class hierarchyとclass kind
- converterとcomparator
- propertyとproperty handler
- arithmetic operator、operation、difference
- aliasと正規化済みalias target
- 生成されたplural rule

nullable fieldと`ResolutionState`は別の意味を持ちます。値がない場合は取得不能を示すことが
あり、空collectionはregistryを確認した結果entryがなかったことを示す場合があります。

## Catalog

`Catalog`は正規化済みdataを所有し、parserで頻繁に使う検索用indexを構築します。

- registration IDとcategoryによるsyntax検索
- code nameによるtype検索
- nameによるfunction overload検索
- event class hierarchyから継承されるEventValue
- source/destination classによるconverter検索
- Java classとtypeのassignability
- alias、comparator、property、arithmetic、plural rule

classとEventValueの走査はcycle-safeです。EventValueは生成済みresolution orderに従い、
exclusion classを考慮します。

`CatalogParts`は明示的なconstructor inputです。`ssg`のようなconverterや、小さなisolated
testで利用できます。

## ID

static syntaxには2種類のidentityがあります。

- `DefinitionId`: semanticなsyntax definitionをまとめます。
- `RegistrationId`: 1件の具体的なregistrationを識別します。

1 definitionが複数registrationを持つことがあります。overrideは目的に応じ、全形式を
置き換えるdefinitionか、1件だけを置き換えるregistrationをtargetにできます。

## Dynamic Syntax Registry

`DynamicSyntaxRegistry`は、immutableな`Arc<Catalog>`へWASM提供の構文をoverlayします。

dynamic IDは次の形式でnamespace化されます。

```text
dynamic:<component-id>/<local-id>
```

componentはpattern、kind、return metadata、handler、free-form metadata、numeric priority、
`before`/`after` constraintを持つdefinitionを登録できます。static definitionまたは
registrationのoverrideも登録できます。

### Lifecycle

- initialization時の更新が、将来のdocumentのbaselineになる
- document revisionごとにbaselineをcloneする
- Document/Preprocess hookがdocument固有の更新をstageできる
- savepointによってparser候補のrollbackができる
- `freeze`が参照とcycleを検証し、immutableな`DynamicSyntaxSnapshot`を返す
- component削除はbaselineとmutable documentへ反映するが、frozen snapshotは変更しない

registration操作はtransactionalです。`DynamicSyntaxUpdate`は`commit`されるまでregistry
stateを変更しません。

### 順序

frozen candidate listはstaticとdynamic syntaxを結合します。明示的constraintはgraphを
構成し、同じ`SyntaxKind`内だけで使用できます。決定的なtopological sortは、kind順、
numeric priority、static/dynamic class、component load順、declaration順、IDを安定した
tie breakerとして使用します。

未知参照、異なるkindへのconstraint、priority cycleはtyped errorです。static candidateへ
付けられたoverrideはpriority、load順、declaration順、dynamic IDでsortされます。

### Quota

現在、1 componentにつき256 item、1 dynamic syntaxにつき64 pattern、pattern text合計
64 KiB、metadata 64 entryに制限しています。patternはCatalogから得た生成済みplural ruleで
登録時に解析します。

## Source構成

| Module | 役割 |
| --- | --- |
| `model` | 正規化済みsyntaxとregistry data structure |
| `catalog` | indexとsemantic query |
| `dynamic` | dynamic registration、override、ranking、snapshot、rollback |

公開model typeはcrate rootからre-exportされます。

## テスト

```sh
cargo test -p syntaxes --locked
```

Catalog testはclass/type assignability、EventValue inheritance、overload index、
converter、aliasをcoverします。Dynamic testは不正pattern、ID重複、決定的ordering、
override target、cycle、freeze、savepoint、stale revision、unload動作をcoverします。

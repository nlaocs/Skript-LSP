# syntaxes

[English](README.md)

`syntaxes`は、Skriptとaddonが登録したすべての情報に対する、保存形式に依存しないruntime
modelです。serialized SSG dataと、parser/LSP consumerの境界になります。

このcrateはSSG snapshot formatをloadまたはvalidateせず、正規化済みmodelもdataを生成した
Skript versionを意識しません。それらは[`ssg`](../ssg/README.ja.md)が担当し、`Catalog`を
構築します。任意の`CatalogSource::from_json_documents` helperはsource retentionとindexingの
ためにcaller提供のJSONをparseしますが、SSG digestはvalidateしません。

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

- Java class hierarchy、class kind、optionalなdeclared method metadata
- converterとcomparator
- propertyとproperty handler
- arithmetic operator、operation、difference
- aliasと正規化済みalias target
- 生成されたplural rule
- effective runtime language key/value entry

`Option`と`ResolutionState`は、正規化済みmodelが公開する範囲でavailabilityの違いを保持します。
SSG raw dataではoptionalなlistが省略またはJSON `null`なら`None`、存在する空arrayなら空collection
として保持されます。convert時には複数のoptional collectionがempty vectorへ正規化され、legacy
expression metadataにはunresolved stateが使用されます。source viewはunknown JSON fieldを含む
元のbyteを保持しますが、typed enum valueのunknown値は`ssg`のdeserializeで拒否されます。

## Catalog

`Catalog`は正規化済みdataを所有し、parserで頻繁に使うoperation用のindexとqueryを提供します。

- registration IDとcategory iteratorによるsyntax検索
- code nameによるtype検索
- nameによるfunction overload検索
- event class hierarchyから継承されるEventValue
- source/destination classによるconverter検索
- Java classとtypeのassignability
- exactなdeclared Java method signature
- alias、comparator、property、arithmetic、plural rule
- case-sensitiveなlanguage検索とdeterministicなlanguage entry iteration

classとEventValueの走査はcycle-safeです。EventValueは生成済みresolution orderに従い、
exclusion classを考慮します。

`CatalogParts`は明示的なconstructor inputです。`ssg`のようなloaderや、小さなisolated testで
利用できます。

`CatalogSource`はsource documentのexact byteを保持し、top-level JSON objectを
`registrationId`と`definitionId`でindexします。`CatalogSource::from_json_documents`は渡された
documentをparseしてsource digestを計算しますが、SSG manifestやcontent digestをverifyしません。
`ssg::load`が返すCatalogでは`Catalog::source()`が設定されます。

class method metadataでは、`Class.methods == None`はmetadataが利用できないこと（例: 古いSSG
schema）を示し、`Some(empty)`はmetadataが利用でき、declared methodがないことを示します。
`declared_method_exists`はclassまたはmethod metadataが利用できない場合に`None`を返し、それ以外
ではexactなparameterとoptional return signatureが存在するかを返します。

language entryは`language_value`と`language_entries`で取得できます。存在しないkeyは`None`、
空valueは`Some("")`で、iterationはkey順でdeterministicです。

## ID

static syntaxには2種類のidentityがあります。

- `DefinitionId`: semanticなsyntax definitionをまとめます。
- `RegistrationId`: 1件の具体的なregistrationを識別します。

1 definitionが複数registrationを持つことがあります。overrideは目的に応じ、全形式を
置き換えるdefinitionか、1件だけを置き換えるregistrationをtargetにできます。

## Dynamic Syntax Registry

`DynamicSyntaxRegistry`は、immutableな`Arc<Catalog>`へcomponent提供の構文をoverlayします。
`parser-wasm` hostはWIT adapterを通してこのoverlayをWASM componentへ公開します。

dynamic IDは次の形式でnamespace化されます。

```text
dynamic:<component-id>/<local-id>
```

componentはpattern、kind、return metadata、handler、free-form metadata、numeric priority、
`before`/`after` constraintを持つdefinitionを登録できます。static definitionまたは
registrationのoverrideも登録できます。

### Lifecycle

- initialization時の更新が、将来のdocumentのbaselineになる
- 新しく開始したdocument revisionごとにbaselineをcloneする
- parser hostのDocument/Preprocess hook処理がdocument固有の更新をstageできる
- savepointと`rollback_to`でparser候補stateをrestoreできる
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

Catalog testはclass/type assignability、common type選択、EventValue inheritance、overload
index、converter、alias、language lookup、literal matching、source retention、difference、
declared method probeをcoverします。Dynamic testは不正pattern、ID重複、決定的ordering、
override target、cycle、freeze、Structure metadata、savepoint、stale revision、unload動作を
coverします。

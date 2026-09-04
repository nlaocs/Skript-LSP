# Parser WASM ABI

[English](README.md)

`parser-wasm`は、Rust parser、必須のCoreLibrary、parser addon componentをつなぐ
WebAssembly Component Model境界の両側を所有します。共通ABI modelと、任意で有効になる
native Wasmtime hostを含みます。

Skriptの構文patternや`.sk` sourceそのものは解析しません。型付きparser dataの転送、
hook呼び出し順序、resource limit、採用されたcomponent side effectのtransaction管理を
担当します。

## Feature mode

defaultの`host` featureは、Wasmtime、永続化、URL処理、LSPで使用する`syntaxes`統合を
有効にします。

```toml
parser-wasm = { path = "../parser-wasm" }
```

WASM guest crateではdefault featureを無効にします。

```toml
parser-wasm = { path = "../parser-wasm", default-features = false }
```

このmodeでは、native hostをlinkせず、ABI version、capability ID、互換性検証を利用できます。

## WIT contract

WIT packageは`nlaocs:skript-parser-addon@0.29.0`です。`parser-addon` worldはhost serviceを
importし、guest実装をexportします。ここでいうWIT package versionはRust crateやcomponentの
versionとは別です。workspaceの両crateは現在`0.1.0`で、CoreLibraryの`component-version`には
crateの`CARGO_PKG_VERSION`が使われます。

Guest export:

- `addon`: static manifestとhost profileのnegotiation
- `hooks`: parser stageの観測、変換、override
- `text-macro`: virtual UTF-8 source textに対するedit
- `tree-macro`: losslessなindentation-based RawTreeに対する対象指定edit
- `ast-macro`: parse済みAST arenaの置換

Host import:

- `catalog-data`: SSGの全read-only document、ID lookup、型関係query、Skript互換の継承距離
- `state-store`: compare-and-swapとprefix scanを備えたscope付きkey/value storage
- `dynamic-syntax-registry`: syntax definitionの追加、override、削除

parser payloadはWITのrecordとvariantです。ABIを通るJSONは`catalog-data`がopaque bytesとして返す
SSG sourceだけです。RawTreeとASTはnode ID arenaを使い、Component Model上で再帰しない値として表現します。

## 互換性

各manifestはdiagnostic用のcomponent IDとcomponent versionを公開します。

package versionはWITのshapeを示します。Text editのanchor追加で0.1.0から0.2.0へ、
lossless RawTreeと対象指定TreeEditの追加で0.3.0へ、型付きpattern matching scope、path、
status、spanの追加で0.4.0、Expression leaf request/candidateの追加で0.5.0、型付きEffect
lifecycle candidate/failureの追加で0.6.0、登録Expressionのmatch後の意味解決追加で0.7.0、
componentが解決する登録Expression classの宣言追加で0.8.0へ変わりました。
汎用registered syntax handler、意味付きCondition/Effect capture、Section lifecycleの追加で
0.9.0へ変わり、登録propertyのcomponent axis情報追加で0.10.0、有限type literal候補の追加で
0.11.0へ、構造化type literal metadataで0.12.0へ、SSG supplier metadataのExpression type option追加で
0.13.0へ変わりました。runtime profileとopen parser result graphで0.15.0、leaf候補から解析済みchild rootを
参照するhost tokenで0.16.0、child node kindとparser IDの明示で0.17.0、SSG ID hook target、
PatternRef routing、宣言的selector、`NotApplicable`で0.18.0へ変わりました。Structure lifecycle、
EntryValidator結果、Structure scoped context、body RawTree、SSGの完全なread-only source access、
host側の型関係queryの追加で0.19.0、Skript互換のJava共通型query追加で0.20.0、typed dynamic Structure登録の追加で0.21.0へ変わりました。
登録semantic handlerの複数targetとdynamic handler照合の追加で0.22.0へ変わりました。
汎用semantic payloadとSSG由来contractの拡張で0.24.0まで進み、host側の継承距離query追加で0.25.0、
正規化済みexperiment catalogへの直接アクセス追加で0.27.0へ、schema version付きpublic
Expression dataと編集可能なsemantic envelopeの追加で0.28.0へ、providerが指定するExpression leaf timing、
完全なactive Type metadata、parser-class targetの追加で0.29.0へ変わりました。
manifestの現在の`abi`値は11.0で、
runtime handshakeとして`major.minor`の完全一致が必要です。

capabilityはclosed enumではなく、安定した文字列IDと独立した整数versionで表します。
新しいcomponentが未知のcapabilityを記述しても、古いhostがmanifestをliftできます。

- 必須capabilityが存在しない、またはversionが古い場合、初期化を拒否します。
- 任意capabilityが存在しない、またはversionが古い場合、無視します。
- capability IDが空、または重複しているmanifestは不正です。
- hostとguestは同じnegotiation ruleを使います。hostがcomponent manifestを検証したあと、
  guestが`addon.initialize`でhost profileを検証します。

hostはText macroとTree macroをadvertiseし、実行します。AST macroはcontractだけが存在し、
まだadvertiseされません。CoreLibraryのmanifestは`parser.hooks`、5つのsyntax parser capability、
Tree macro、`parser.state-store`を必須とし、`parser.dynamic-syntax`とversion 2の
`parser.catalog-data`を任意で利用します。TextとAST macroは必須ではありません。

`addon.initialize`には、読み込んだSSG manifestから作った`RuntimeProfile`も渡します。snapshot、server、
Skript、Minecraft、Javaのversion、language、有効pluginのload orderが含まれます。`ParserHost::new`は
validation前に`HostConfig::inherit_catalog_runtime`を呼びます。`syntax_catalog`がSSG sourceを持つ場合、
profileで未指定のfield（Skript versionやsnapshot identityを含む）はsourceから自動補完されるため、callerが
同じversionを重複指定する必要はありません。source Catalogも明示的なSkript versionもないdefault configは
CoreLibrary初期化で拒否され、明示したsnapshot identityはsource Catalogと一致する必要があります。
componentは、Java classやparse markの意味がversion間で変わる構文を、特定のSkript releaseを暗黙の標準にせず
処理できます。

## Open parser request

capture parserは閉じたenumではなく文字列IDを使います。標準routeは`host.expression`、
`host.condition`、`host.effect`で、addon manifestは独自の`parser(...)` targetへsubscribeできます。
hookが`parse-request`を返すと、hostは解析完了後、対応する`parse-result` graph付きで同じhookを再度呼びます。
graph nodeは意味summary、child、mapped span、diagnostic、metadata、version付きopaque addon attachmentを保持します。
Expression summaryにはschema version付きのpublic dataも含められ、各nodeが自分のlistだけを保持します。
完了したresultにはhost所有のtokenが付きます。Expression leaf候補はtokenとroot IDを参照し、再解析や
metadata keyへの依存なしに、そのExpressionをnative child ASTとして所有できます。
continuationにはそれ以前の全roundで得たresultを累積して渡すため、後続requestは複数の先行parseへ依存できます。

nested処理にはround、request、result node、call、recursion quotaがあります。現在実行中と同じrequest keyは
cycle failureになります。request中のwriteは外側候補が採用された場合だけcommitされ、Reject、trap、cancel、
不正outputではcontinuation全体をrollbackします。

## Expression解析

`ParserHost::parse_expression_in_parse`はdocumentのdynamic syntax registryをfreezeし、1つの
transactional WASM environmentで`skript-parser`を実行します。Expression subscriptionは
`parser.expression` capabilityを宣言し、`ParseStage` / `Expression` phase / `Transform` modeを
使用します。型付きpayloadにはvirtual source全体、remaining rangeとmapped span、expected Java
classとplural、合法split位置、literal/expression flag、time state、depth、一致した有限type literal候補、
蓄積leaf候補が含まれます。hostはSSGのtype metadataとaliasから有限literal indexを一度だけ作り、
現在の合法split位置に一致する候補だけをWASMへ渡すため、registry全体を毎回copyしません。

CoreLibraryとaddonはVariable、Literal、Function、Customのleaf候補を追加し、open parser protocolから
返された解析済みchild rootを所有できます。登録
Expressionと型付きの子Expressionが一致した後、hostはparse tag、子Expression、generic parsed capture、既知の返値候補、
適用可能なproperty情報と対応component axisを含む2段目のpayloadを送ります。
CoreLibraryまたはaddonは実効Java返値型と
Multiplicityを確定するか、候補をrejectできます。componentはsemantic handlerごとに安定したhandler IDを付け、
`registered-syntax-handlers`へ1つ以上のtargetを宣言します。targetは`definition`、`registration`、`parser-class`、
`class-suffix`、`super-class`、`dynamic-handler`のいずれかです。`parser-class`は`kind: Type`だけで使用でき、
SSGが出力した完全一致のparser classへ照合します。複数targetはORとして扱われるため、1つのhandlerで複数のstatic登録とdynamic
definitionを処理できます。hostはstatic targetを読み込み時に一度だけcatalogへ照合し、解決したdefinitionIdと
registrationIdを`HostProfile`でcomponentへ渡します。`dynamic-handler`はparse時にdynamic syntax definitionが宣言した
opaqueなhandler IDへ照合され、catalog lookupは行いません。このtargetでもcapture parserと名前付きcontext requirementを
提供できます。実行時のsemantic選択はJava class suffixへ依存しません。

handlerには`kind: Type`も指定できます。解決済みのDefinition、Registration、parser-class、class-suffix、
super-class bindingは、型単位の`Expression` phase呼び出しの対象になります。Snapshotの有限literalも、component
handlerを要求せず所有Typeを候補へ加えます。hostは要求返値型との互換性で候補を絞り、直接assign可能なTypeを
converter経由のTypeより先に置いた上で、各groupを`typeParseOrder`順に並べます。payloadの`active-type`は
対象SSG Typeのsource record、addon、parser class、parse order、`before`/`after`関係を示します。componentはType
targetへsubscribeし、自身が担当するactive registrationだけを処理します。Type parserが文字列を解釈し、その値を
Literalとして返す構造です。Typeごとに候補listは分離され、rejectまたは候補なしの呼び出しだけstateとeffectsを
rollbackするため、別Typeが生成済みの候補は失われません。
複合Typeが内部で使用するEntityDataのsupplier情報などを必要とする場合は、既存の
`expression.type-options.all` context requirementを指定できます。hostは該当する呼び出しにだけ
全Type optionを渡します。
Javaのparser自体を実行したり、`usage`から文法を推測したりする機能ではありません。

各leaf候補は`before-registered`または`after-registered`のtimingを宣言します。native parserはCoreLibrary固有の
parser IDを識別せず、このfieldだけで登録Expressionとの前後関係を決めます。third-party parserもVariableStringの
ような早期parserを再現でき、通常のType literalは登録Expressionの後に維持できます。

handlerは名前付きhost contextも要求できます。また、`pattern-indices`、完全一致する`pattern-sources`、
必須・禁止parse tag、集約ParseMarkでtargetを
さらに絞れます。各list内はOR、空でないpredicate group同士はANDです。これにより特定構文をhostへ
hard-codeせず、Javaの`init`内の分岐をaddonから表現できます。capture parser optionの
`context.event-classes`（`;`区切りのJava class）と`context.value.<key>`はnested host parserのcontextだけを
一時的に上書きし、候補の成功・失敗にかかわらず外側contextへ復元します。
`host.expression`では、`context.value-from-child.<key>`によってそれ以前の型付きExpression captureから
`0.return-type`、`0.possible-return-types`、`0.multiplicity`、`0.metadata.<key>`のようなselectorで
context値を導出できます。`host.expression`は`parse.mode`（`all`、`expressions-only`、`literals-only`）、
`expression.expected-types`、`expression.time-state`にも対応します。capture indexはpattern内の
すべてのcaptureを出現順に数えるため、`<.+>`より前の`%type%` captureもindexへ含まれます。
`expression.type-options.all`は`ExprParse`のような構文へSSGの全Type optionを渡します。host側はJava class名を
知る必要がありません。各childにはnative node kindと任意のparser IDも含まれるため、componentはsource文字列を
推測せずliteral、variable、functionなどを区別できます。native parserは有効なcomponentが宣言した
場合だけ、通常は期待型と互換しないdynamic登録を候補へ含めます。これにより、未解決登録をすべての
型探索へ混ぜません。hostはnative
parserでstatic/dynamic登録Expressionと順位付けする前に、変更不可request field、UTF-8 range、
parser ID、return type/Multiplicity、metadataを検証します。nativeのrange、type、Multiplicity
検証で全leafが除外された場合、そのdispatchのstateとeffectsをsavepointへ戻します。再帰matcher
呼び出しはそれぞれcandidate frameを持つため、子候補の選択が親のselected stateを上書きしません。
no-matchとparser failureでは開始時のStateStore savepointへ戻します。parse overlay revisionは
native memo keyに含まれ、candidate rollback時にはrevision自体も復元されます。

### Public semantic data

Expression semantic payloadの`public-data`（native Rustでは`public_data`）は、
owner-protectedな`metadata`とは別の公開dataです。各recordは一意な`schema-id`、
`1`以上の`schema-version`、およびJSON objectでなければならない`json` stringを
持ちます。hostが検証するのはこのenvelopeだけで、object内部のaddon固有fieldは検証
しません。schemaを定義したaddonがその意味を解釈します。特にhostはVariableDataの
semantic consistencyを検証しません。editorまたはaddonはname template、そこからの
`childIndex`参照、nodeのreturn typeとmultiplicityを整合させる必要があります。
list shapeを変更する場合は標準のmultiplicity fieldも変更してください。hostがJSONから
multiplicityを推測することはありません。

public dataはnode-localです。listは元になったExpression node自身に属し、schema内の
`childIndex`はそのnode自身の`children`を参照します。外側のEffect capture listや
`Grouped` wrapperのchild listを参照するものではありません。nativeのGrouped wrapperは
`public_data`を空のままにし、元のchildがrecordを保持します。reportとCLI outputも
nodeごとのlistだけを表示し、node identityをまたいでflattenしません。variable nameの
textはescaped `%%`を含むsource spellingを保持し、評価済みruntime keyではありません。
このsource-name dataを変更することはsemantic interpretationの変更ですが、元のsource
textを書き換えません。

`observe` hookはrecordを読めます。現在のcandidateに対して実行を許可された任意の
`Transform`または`Override` hookはrecordを置換・削除できます。caller orderが
適用されるため、後続hookは先行hookが採用したreplacementを見ます。schema IDはpublic
contractでありowner markerではありません。public dataはparse時のsemantic情報であり、
runtime variable valueでもshared `StateStore`のentryでもありません。variable typeの
trackingとserver側のvariable value mutationは実装されていません。変更してもinput
sourceやspanは変わらず、sibling/parent/child nodeを遡及編集せず、whole ASTを一括編集
することもありません。

既存の`hooks::Guest::invoke`では、crateがすでに公開しているWIT fieldを使って
CoreLibraryのvariable leafをExpression payload内で変換できます。次の例はpublic dataと
leafのreturn type/multiplicityを同じhookで変更します。

```rust,ignore
use exports::nlaocs::skript_parser_addon::hooks;
use nlaocs::skript_parser_addon::types::{
    AddonError, DynamicMultiplicity, HookDecision, HookEffects, HookInvocation,
    HookOutput, HookPayload,
};

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
enum VariableScope {
    Local,
    Global,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum VariableNamePart {
    Text { text: String },
    Expression {
        #[serde(rename = "childIndex")]
        child_index: u32,
    },
}

#[derive(serde::Deserialize, serde::Serialize)]
struct VariableData {
    scope: VariableScope,
    name: Vec<VariableNamePart>,
}

impl hooks::Guest for MyAddon {
    fn invoke(input: HookInvocation) -> Result<HookOutput, AddonError> {
        let not_applicable = || HookOutput {
            decision: HookDecision::NotApplicable,
            replacement: None,
            effects: HookEffects {
                diagnostics: Vec::new(),
                context_updates: Vec::new(),
                parse_requests: Vec::new(),
                parse_results: Vec::new(),
            },
        };
        let HookPayload::Expression(mut payload) = input.payload else {
            return Ok(not_applicable());
        };
        let Some(candidate) = payload.candidates.iter_mut().find(|candidate| {
            candidate.public_data.iter().any(|record| {
                record.schema_id == "nlaocs.skript.variable" && record.schema_version == 1
            }) && candidate.multiplicity == Some(DynamicMultiplicity::Single)
        }) else {
            return Ok(not_applicable());
        };

        {
            let Some(record) = candidate.public_data.iter_mut().find(|record| {
                record.schema_id == "nlaocs.skript.variable" && record.schema_version == 1
            }) else {
                return Ok(not_applicable());
            };
            let Ok(mut data) = serde_json::from_str::<VariableData>(&record.json) else {
                return Ok(not_applicable());
            };
            for part in &mut data.name {
                if let VariableNamePart::Text { text } = part {
                    *text = text.replace("price", "money");
                }
            }
            let Ok(json) = serde_json::to_string(&data) else {
                return Ok(not_applicable());
            };
            record.json = json;
        }
        candidate.return_type = Some("java.lang.String".to_owned());
        candidate.multiplicity = Some(DynamicMultiplicity::Single);

        Ok(HookOutput {
            decision: HookDecision::ContinueProcessing,
            replacement: Some(HookPayload::Expression(payload)),
            effects: HookEffects {
                diagnostics: Vec::new(),
                context_updates: Vec::new(),
                parse_requests: Vec::new(),
                parse_results: Vec::new(),
            },
        })
    }
}
```

この例は`Expression` leaf candidateを対象にしています。leafが持つ可変な型fieldは
`return_type`と`multiplicity`です。`possible_return_types`はleaf candidateのfieldではなく、
後段のregistered-Expression semantic payloadに属します。

CoreLibraryが提供するvariable schemaは`nlaocs.skript.variable` version `1`です。
JSON shapeは次のとおりです。

```json
{"scope":"local","name":[{"kind":"text","text":"money"},{"kind":"expression","childIndex":0}]}
```

`scope`は`local`または`global`です。`name`はsource-name templateであり、
expression partは同じsemantic nodeの既存childを指します。childのreturn typeや
multiplicityを重複して格納しません。

## Effect解析

`ParserHost::parse_effect_in_parse`はlosslessな`RawNodeKind::Simple` nodeを受け取り、indentationと
行末commentを除いた正確な`code_span`を照合します。static SSG Effectとfrozen dynamic登録は
同じresolved orderを使います。`%type%` captureは再帰Expression sessionへ入り直すため、採用
Effectと子Expressionのstateは1つのtransaction階層で管理されます。

Effect subscriptionは`parser.effect` capabilityと`Effect` phaseを使います。hostはnative照合前に
category-levelの`SyntaxKind::Effect` hookを呼びます。exact pattern/registrationのsemantic hookは候補照合中に
実行され、外側のafter dispatchはunknownまたはnear-matchのdiagnostic用です。Skript互換の照合では、catalogの
`EffectSection` registrationを通常のEffectより先に試します。通常のEffectを採用した場合は`Effect` syntax-kind
targetを使いますが、one-line EffectSectionを採用した場合は`Section` syntax-kind targetを保ったまま`Effect`
phaseを使います。WITの`effect-candidate`は安定したidentityとcaptureを公開し、`effect-section` identityは
追加WIT flagではなく、host側dispatch targetの`Section` kindとcatalog lookupで保持されます。WITのpattern reference
自体はdefinition/registration identityとpattern indexを持ちます。typed payloadはdefinition/registration ID、element
class、pattern index、capture span、parse tag、XOR mark、解析済みConditionまたはnested Effect capture、dynamic
handler metadata、alternative、最遠failureを保持します。置換できるのは採用候補のhandlerとmetadataだけで、
registration identity、capture、alternative、spanはhostが固定します。

unknown、Reject、不正output、host failureではEffect入口のStateStore savepointへ戻します。
unknown nodeは正確なsource、mapped span、最遠failureを保持します。Reject hookのstateは破棄
されますが、そのdiagnosticは結果に残ります。

## ConditionとSection解析

`ParserHost::parse_condition_in_parse`は、完全な外側の括弧を繰り返し外す挙動を含め、Skriptの
registration順でConditionを照合します。再帰Expression sessionを共有するため、Condition patternは
型付きExpressionを含められ、登録Expression、Effect、Sectionの意味付きregex captureとしても
再利用できます。

`ParserHost::parse_section_in_parse`はlosslessな`RawNodeKind::Section`を受け取ります。headerは通常の
Section、EffectSection、SectionExpression、frozen dynamic Sectionをまとめて照合し、採用候補には
3種類のmetadata flagを保持します。子SectionとEffectを再帰解析する前後で、hostは`parser.section`を使い、
`Section` phaseのexact registrationをdispatchします。block-formのEffectSectionはこのSection lifecycleを使います。
一方、one-lineの`RawNodeKind::Simple` EffectSectionは上記のEffect pathを使い、`Section` syntax kindを保ったまま
`Effect` phaseで処理されます。enter phaseのcontext updateはそのSection bodyと子孫だけへ適用されます。
未取得または複数取得されたbody nodeも、diagnostic付きpartial treeとして保持します。

CoreLibraryはSkript標準のconditional、filter、loop、while、error-catching Section、`ExprWhether`、`ExprTernary`、
`EffChange`、`EffDoIf`、`EffSecShoot`、`EffSecSpawn`などのsemantic handlerを宣言します。addonも同じmanifest
宣言を使い、独自のraw、Condition、nested Effect captureを処理できます。

### Dynamic Structure登録

`dynamic-syntax-registry`では、static SSG Structureと同じparser向けmetadataを持つStructureを登録できます。
`structure-node-type`は`simple`、`section`、`both`から選び、`structure-body-mode`は`none`、`raw`、
`entries`、`trigger`から選びます。`entry-validator`には完全な宣言型`EntryData` treeを指定します。

これらのfieldはStructure専用です。別のsyntax kindで設定するとhostが登録を拒否します。`Simple` Structureは
`none`だけを使え、`entries` bodyにはvalidatorが必須で、validatorは`entries`との組み合わせでだけ使えます。
optional fieldを省略した場合は既存のdynamic登録との互換性を保ち、node typeは`both`、body modeはvalidatorの
有無に応じて`raw`または`entries`になります。

Component Modelではrecursive recordを直接表現できないため、validatorはflatな`entry-data` listとして渡します。
root entryの`parent-entry-index`は省略し、nested entryは親containerの0始まりindexを指定します。
`nested-validator-present`により、空のnested validatorとvalidatorなしを区別します。hostはindex、cycle、到達不能な
entry、重複key、fieldの組み合わせを登録前に検証します。

`default-value`は文字列化した簡易値ではなく、元のJSON documentです。JSONの`null`、array、object、number、
boolean、stringを失わずに渡せます。ABI層は値を解釈せず、native parserがlosslessな`serde_json::Value`へ変換します。
これによりStructure固有のdefault値をparserへ渡しつつ、WITはformat-neutralに保てます。

dynamic Structure candidateはheader照合前に宣言node typeでfilterされます。その後、宣言body modeとvalidatorが
通常のbody parserで使われるため、dynamic登録でもstatic Structureと同じcandidate/EntryValidator経路になります。

## Structure解析

`ParserHost::parse_structures_in_parse`はtop-level RawTree rootを二段階で解析します。native Rustは
Structure順序、`NodeType`、宣言的`EntryValidator`、body走査、transaction境界を担当します。
WIT `structure-payload`は安定したdefinition/registration ID、capture、解析済みentry、
Structure scoped context、候補をrootとするread-only subtreeを公開します。不変fieldは各hookの
直後に検証されるため、改変されたpayloadが次のaddonへ渡ることはありません。

`parser.structure` capabilityはexact registration hookを`enter-body`と`exit-body`でdispatchします。
enter hookは候補のreject、body context更新、`none`/`raw`/`entries`/`trigger`の選択、所有者付き
metadata追加を行えます。context updateはhook順で合成され、次のaddonから参照できます。
CoreLibraryの`StructEvent`、`StructFunction`、`StructCommand`も同じ公開ABIだけで実装されており、
addon固有Structureのためにnative parserを変更する必要はありません。

## Text macro

Text macroは`Preprocess` phaseの`ParseStage`へ`Transform` modeでsubscribeします。一致した
subscriptionは決定的なpriority順で実行され、その時点のvirtual UTF-8 sourceを受け取ります。

各outputはbyte rangeのeditを返します。hostはeditをsortし、overlap、不正なUTF-8 boundary、
曖昧な同位置insert、不正なanchorを拒否してから、output全体をatomicに適用します。置換textは
既定で置換対象のcall-siteへ対応し、任意の`anchor`を指定すると生成textを明示的なzero-width
位置へ対応付けます。複数macroのoutputではSourceMapを順次合成し、親子関係を持つText
expansionをExpansionGraphへ追加します。複数editのoutputや、過去の複数mappingをまたぐ置換は
全originを保持し、最初のcall-site以外を捨てずにexpansion DAGを構築します。

effects、Reject、addon errorのdiagnosticとparse requestのspanは、そのmacroへ入力された
virtual sourceに対するrangeとして解釈します。hostはguestが指定したoriginsを信用せず、
primary spanとrelated spanの両方を現在のMappedSourceから再構築します。これにより、後段
macroのeffectもそれ以前のすべての展開を辿ってoriginal documentへ対応します。不正な
diagnosticまたはparse requestのbyte rangeは、そのcallの不正出力として扱い、textとstate
変更をrollbackします。

pipeline全体をRejectした場合は、すべてのcallを未採用へ変更し、callのexpansion IDと、
復元後source graphに存在しないdiagnosticのexpansion参照を削除します。context updateと
parse requestも、Rejectされた変換と一緒に破棄します。Reject diagnosticの
`virtual-range`はRejectしたmacroの入力snapshotを指し続け、再構築したoriginがoriginal
document上の位置を示します。editor diagnosticにはoriginを使用してください。これにより、
返却metadataがrollback済みexpansionを参照することはありません。

各callにはStateStore invocation transactionがあります。addon error、trap、不正edit、
pipelineのRejectでは、対応するtextとstate変更を破棄します。成功したcallはread/write setを
公開し、将来のincremental parseがstate dependencyを追跡できるようにします。

`HostConfig`はexpansion数、生成replacement byte数、virtual source全体のbyte数を制限します。
pipeline全体のquotaを超えた場合は、元sourceとStateStore savepointへ戻します。

## Tree macro

Tree macroは`Tree` phaseの`ParseStage`へ`Transform` modeでsubscribeします。hostはlossless
RawTreeをpre-orderで走査します。各callには、その時点のtree全体、対象node ID、生成nodeの
depthを渡します。raw line、trivia、invalid reason、diagnostic、indentation metadata、
span、syntax contextもWIT payloadに含まれます。

`TreeEdit`の対象は現在のnodeです。対象nodeを0個、1個、複数の生成nodeへ置換する、Sectionの
bodyだけを置換する、元Sectionの子を生成Sectionの子の前後へ保持する、という操作ができます。
生成fragmentはlocal IDを使い、hostがIDの一意性、到達可能性、cycle、node kind、text、
親子関係を検証します。最終的なRawNode ID、ExpansionId、call-site span、SyntaxContextIdは
hostだけが割り当てます。

生成rootと生成Sectionの子は同じTree macro stageへ再投入されます。構造上のnestingとmacroの
再投入は独立したquotaで制限し、`max_raw_tree_depth`の既定値は256、
`max_tree_macro_expansion_depth`の既定値は64です。総node数、hook call数、output byte数にも
別のpipeline quotaがあります。さらにmacro identity、入力origin、subtree内容を組み合わせ、
直接・間接cycleを検出します。cycle時は現在のnodeを保持し、component failureと
`tree-macro-cycle` diagnosticを返します。

各候補はStateStore invocation transaction内で実行します。TreeEdit検証とstate採用はatomic
です。addon error、trap、不正edit、cycleでは現在のnodeを保持し、その候補の書き込みを
rollbackします。型付きRejectまたはpipeline quota errorでは、元tree、source provenance、
parse StateStore savepointを復元します。成功したeditはExpansionGraphへTree entryを追加し、
再帰的に生成されたnodeから完全なcall-site backtraceを辿れます。

## Pattern matching hook

`ParserHost::match_patterns_in_parse`は他のparser stageと同じparse transaction上でnative
matcherを実行します。`MatchingPayload`は入力と任意pattern、definition/registration ID、
pattern index、nested element/branch path、pattern source range、local input range、editor向け
mapped span、scope、timing、status、failure reasonを公開します。

matching hookはdefinition、registration、pattern、element scopeのbefore/afterで実行されます。
handled overrideは`matched`または`failed`を返す必要があります。matched elementは検証済みprefixを
消費でき、definition、registration、patternの広域matchはtrim済み入力全体を消費する必要が
あります。hook replacementでidentityとprovenance fieldは変更できません。

各syntax候補は同じparse StateStore savepointから開始します。失敗候補と非採用alternativeの
書き込みはrollbackされ、採用候補のStateStore書き込みとHookEffectsだけがparse結果へ残ります。hook callと
component failureはdiagnostic/tracing用に保持します。
## Hook rule

subscriptionはtarget、phase、signed priority、modeを指定します。

- `observe`: payloadを読み取れますが、置き換えてはいけません。
- `transform`: 後続hookへ渡すreplacement payloadを返せます。
- `override`: targetの通常処理に代わって処理します。

targetはparse stage、parser ID、syntax kind、definitionId、registrationId、または正確な
`registrationId + patternIndex`を指定できます。宣言的selectorでは現在のpattern、mark、tag、解析済みcapture、
実効return type、Multiplicity、metadataをAND条件で絞れます。return-type selectorは`exact`、`assignable`、
`convertible`のrelationを使います。catalog relationが不明な場合は`NoMatch`ではなく内部的に`Unknown`として
扱われるため、WASMが呼ばれ、component自身が最終的な適用可否を判断できます。

`NotApplicable`は「このpayloadは対象外」です。そのhookのreplacement、effects、StateStore write、dynamic syntax変更を
破棄して次へ進みます。`ContinueProcessing`は変更を採用して続行し、`Handled`は採用してchainを停止し、`Reject`は
拒否したcallをrollbackしつつdiagnosticを返します。

manifestの`catalog-annotations`はdefinition、registration、exact patternへ所有者付きmetadataを付けます。hostは
selector評価前にannotationを適用します。後段hookは他componentのmetadataを読めますが変更・削除できず、hookが
新規作成したmetadataにはhostがそのcomponent IDを記録します。

所有者付きmetadataをnative ASTへ移すときは`component-id/key`で表し、WITへ戻すときに構造化されたownerを
復元します。そのためcomponent ID内の`/`はnamespace separatorとして予約されています。

hostはcomponent登録時に、mode固有の動作、payload variant、subscription、capabilityを
検証します。runtime limitとtrap処理はWasmtime hostの責務です。

subscriptionの順序は決定的です。

1. parser、exact pattern、registration、definition、syntax-kind、parse-stageのtarget specificity
2. signed subscription priority
3. component load順
4. component manifest内の宣言順

overrideがhandledを返すと、後続の一致するhookを停止します。addon errorはcomponent failure
として報告されます。trap、timeout、fuel枯渇、resource-limit違反が起きたcomponentは
無効化されます。

## SSG Catalog data

hostがSSG由来の`Catalog`を持つ場合、`parser.catalog-data`をadvertiseします。各hookは全JSONを
payloadへ複製せず、`catalog-data` importから保持されたsnapshot全体をread-onlyで参照できます。

- `source`はformat、schema version、generatorのsnapshot IDに加え、保持した全filenameとbyte列を覆う
  正確な`source-digest`を返します。未知のManifest fieldだけが変わった場合もdigestは変化します。
- `documents`は`Manifest.json`を含む全source fileをpage単位で列挙します。`read-document`は
  保持された原文をrange単位で読み、大きなfileも最後まで取得できます。
- `records-by-registration-id`と`records-by-definition-id`は一致する全top-level JSON objectを、
  document名とarray indexの参照としてpage単位で返します。`read-record`は各objectをrange単位で
  読みます。重複IDも意図的に保持し、どの候補を使うかはaddonが判断します。
- `class-known`、`is-class-assignable`、`hierarchy-distance`、`can-convert`はhostの正規化済みclass・converter indexを使い、
  非互換とsource data不足を区別できます。型関係queryは`compatible`、`incompatible`、`unknown`を返し、
  `class-known = false`はsnapshotに未収録という意味であり、runtime classpathからの不在は証明しません。
  class不足を確定的な非互換として返しません。継承距離はassignability確認後にSkriptと同じ
  concrete superclass chainで計算します。
- `declared-method-exists`は収録済みclassへ`Class.getDeclaredMethod`相当の検索を行います。
  schema 5のmethod metadataがあれば`Some(false)`まで確定でき、旧snapshotや未収録classは`None`で
  unresolvedを維持します。
- SSG schema 5は`Language.json`も保持します。schema 3と4のsnapshotには保持されたLanguage entryが
  ありません。`catalog-data::language-value`は正確なcase-sensitive key lookupを行い、missing keyまたは
  旧snapshotでは`none`を返します。`language-pattern-matches`は保持されたregexがあればそれを使い、なければ
  callerのfallbackを使います。入力全体にmatchするようanchorされ、regexのcompileはguest fuelの外で行われます。

未知fieldも生JSONから利用できます。索引されたrecordは正しいJSONですが、空白やobject key順序は保証しません。
各page/chunkは既定32 MiBの`HostConfig::max_catalog_response_bytes`で制限されますが、paginationと
range readによりsource全体へ到達できます。このviewは不変なSSG source snapshotであり、dynamic
syntaxとtransactional runtime情報はそれぞれ専用APIを使います。
信頼する構築経路は`ssg::load`です。手動構築した`CatalogSource`を埋め込むhostは、事前にbytesを検証する
責任を持ちます。`RuntimeProfile`とsource Catalogが両方ある場合、schema/snapshot identityが一致しなければ
hostの構築に失敗します。

候補のstable IDから、型付きhook payloadにないfieldを取得できます。

```rust,ignore
fn read_record(record: &types::CatalogRecordRef) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut offset = 0;
    while offset < record.byte_length {
        let chunk = catalog_data::read_record(
            &record.source_digest,
            &record.snapshot_id,
            &record.document,
            record.index,
            offset,
            u32::MAX,
        )?.expect("不変なCatalogのrecordは読み取り中に消えない");
        if chunk.offset != offset || chunk.total_length != record.byte_length {
            anyhow::bail!("読み取り中にCatalog recordが変化した");
        }
        if chunk.bytes.is_empty() {
            anyhow::bail!("Catalog recordの読み取りが進まなかった");
        }
        bytes.extend_from_slice(&chunk.bytes);
        offset = offset.checked_add(chunk.bytes.len() as u64)
            .ok_or_else(|| anyhow::anyhow!("Catalog record offsetがoverflowした"))?;
    }
    anyhow::ensure!(offset == record.byte_length, "Catalog recordがdescriptorを超えた");
    Ok(bytes)
}

let mut offset = 0;
loop {
    let page = catalog_data::records_by_registration_id(
        &candidate.registration_id,
        offset,
        64,
    )?;
    for record in &page.items {
        if record.document != "Expressions.json" {
            continue;
        }
        let expression: serde_json::Value = serde_json::from_slice(&read_record(record)?)?;
        let accepted = &expression["acceptedChangers"];
        // addon/versionも調べます。IDは検索keyであり、一意なrow keyではありません。
    }
    let Some(next) = page.next_offset else { break };
    offset = next;
}
```

`catalog-record-ref`は正確な`source-digest`とgeneratorの`snapshot-id`の両方へ結び付いています。
保持byte列が異なるhostへ渡すと、偶然同じdocument/indexを読むのではなく拒否されます。

Expression source recordでchanger型の末尾に付く`[]`はJava配列classではなく、そのelement classの
複数値をchangerへ渡せることを表します。例えば`java.lang.String[]`は「複数の
`java.lang.String`を受け付ける」という意味です。`acceptedChangersState`も必ず確認してください。
`unresolved`の場合、SSGがcontractを確定できなかっただけなので、modeの欠如を「非対応」と断定できません。

Typeとliteral optionは正確な`Types.json` source recordを持ち、構造化supplier literalはnested literal indexも
持ちます。登録Expressionの子要素にはstableなdefinition/registration IDとpattern indexが含まれ、metadataには
`target-type`などのopenなsemantic roleも載せられます。Property optionには正確なsource recordとpayload上のindex、
一致理由、type code/element class、Property登録ID、Propertyのowner/handler、related typeのhandler/provider、
`acceptedChangers`、解決状態、`requiresSourceExpressionChange`も含まれます。同じ入力Java classでも、
異なる登録やhandlerは潰しません。これによりsemantic addonはJava class suffixやpattern indexを
ハードコードせず、Skript runtimeの検査を再現できます。複数登録が一致した場合、先行hookは
`selected-property-option-indices`へ採用候補を書き、hostのindex検証後にCoreLibraryが選択候補だけを評価します。
その他のSSG fieldも生source APIから取得できます。

Addonは解析済みExpressionの有効なchanger contractを、keyが`change-contract`のowned metadataとして公開できます。
値はschema versionと、所有するExpressionのregistration/parser identityへ結び付いたenvelopeです。

```json
{"schemaVersion":1,"subjectId":"expression:addon:registration","contract":{"state":"resolved","modes":{"SET":[{"className":"java.lang.String","multiple":false}]}}}
{"schemaVersion":1,"subjectId":"expression:addon:registration","contract":{"state":"unresolved"}}
```

CoreLibraryもProperty ExpressionとEffChangeの連携に同じcontractを使います。独自Expressionを提供するAddonも
このcontractを公開できます。複数providerが矛盾するcontractを公開した場合は、黙って1つを選ばずunknownとして扱います。
schemaまたはsubject identityが異なるcontractは、別Expressionへ誤適用せず拒否します。

## StateStore

StateStoreはhookとmacro呼び出しから使えるhost importです。

| Scope | Lifetime |
| --- | --- |
| `invocation` | 1回のcomponent call |
| `parse` | 1回のparse transaction |
| `document` | commitされたdocument revision |
| `project` | 1 project内のdocumentとaddon |
| `persistent-project` | LSP再起動後も保持するproject state |

namespaceは1 component専用にするか、明示的なschema declarationによって共有します。
shared declarationはschema ID、schema version、reader、writerを指定します。valueはraw、
CBOR、JSON encodingのいずれかを宣言しますが、hostはbytesの内容を解釈しません。

component callごとにinvocation overlayを作成します。reject、trap、不正なcallではrollback
します。採用されたcallだけをparse overlayへmergeします。document revisionが最新で、
project revision conflictがない場合に限りparseをcommitします。persistent project stateは
OSのapplication data directory配下の`redb`へ保存し、canonical project URIごとに分離します。

## Dynamic syntax

`HostConfig::syntax_catalog`にSSG由来の`Arc<syntaxes::Catalog>`が設定されていると、
hostは`parser.dynamic-syntax`をadvertiseします。componentは次の操作を行えます。

- component/local IDでnamespace化された新規syntaxの登録
- definition IDまたはregistration IDによるstatic syntaxのoverride
- staticまたはdynamic syntaxに対する順序制約の追加
- 自身が登録したdynamic entryの削除

更新できるのはcomponent初期化中とDocument/Preprocess prepass中です。後続のparser phase
より前にregistryをfreezeします。immutable snapshotはstatic候補とdynamic候補を決定的な
topological orderで結合します。rejectやhost failureでは、StateStoreと同時にdynamic更新も
rollbackします。componentをunloadすると将来のsnapshotからentryが消えますが、すでに
freezeされたdocument snapshotは変更しません。

Catalog未接続時は、このcapabilityを意図的に利用できません。

## Native host API

主なentry pointは次のとおりです。

- `ParserHost::new`: 必須CoreLibraryをinstantiateする
- `load_addon` / `unload_addon`: component lifecycleを管理する
- `begin_parse`: multi-phase parse transactionを作る
- `dispatch_in_parse`: 一致するhook subscriptionを呼ぶ
- `expand_text_in_parse`: 既存parse transaction内でText macroを実行する
- `expand_text`: 1 pipeline分のparse transactionを作るconvenience API
- `expand_tree_in_parse`: 既存parse transaction内でTree macroを再帰実行する
- `expand_tree`: 1 tree pipeline分のparse transactionを作るconvenience API
- `dynamic_syntax_snapshot`: 候補をfreezeし、順位付きsnapshotを取得する
- `match_patterns_in_parse`: transactional WASM hook付きで順位済み候補を照合する
- `parse_expression_in_parse`: 型付き再帰Expressionを解析する
- `parse_condition_in_parse`: registration順でConditionを解析する
- `parse_effect_in_parse`: simple RawTree nodeをEffectとして解析する
- `parse_section_in_parse`: Sectionとそのbodyを再帰解析する
- `parse_structures_in_parse`: top-level Structureと選択されたbodyを解析する
- `dispatch`: 1回のdispatch transaction用convenience API

`HostConfig`はcall fuel、epoch timeout、Wasmtimeのmemory/table/instance limit、dispatch
output quota、Text macro/Tree macro quota、StateStore設定、任意のsyntax Catalogを管理します。

## Source構成

| Path | 役割 |
| --- | --- |
| `wit/` | Component Model package、world、record、variant、host import |
| `src/bindings.rs` | WITから生成されるWasmtime binding |
| `src/host.rs` | component lifecycle、subscription、dispatch、limit、dynamic syntax bridge |
| `src/state/mod.rs` | namespace registryとin-memory transactional StateStore |
| `src/state/persistent.rs` | `redb` persistent-project backend |
| `tests/contract.rs` | host/guest binding contract |
| `tests/host.rs` | CoreLibrary lifecycleとWasmtime動作 |
| `tests/state.rs` | scope、permission、conflict、quota、persistence |
| `tests/dynamic_syntax.rs` | SSG fixtureに対する実WASM dynamic registration |
| `tests/text_macro.rs` | 順序付き実WASM展開、diagnostic mapping、rollback、quota、trap |
| `tests/tree_macro.rs` | 実WASM node/body edit、再帰provenance、cycle、rollback、quota、trap |
| `tests/pattern_match.rs` | 実WASM element overrideと採用候補だけを残すStateStore rollback |
| `tests/structure.rs` | 実CoreLibrary Structure lifecycle、Event capture、EntryValidator、未知addon entry |

## テスト

integration testより先に埋め込みcomponentをbuildします。

```sh
cargo run -p xtask --locked -- build-core-library
cargo run -p xtask --locked -- build-test-components
cargo test -p parser-wasm --locked
```

workspace全体のcheckでは、host専用dependencyがguest componentへ誤って必要になっていない
ことも確認します。
`1`以上の`schema-version`、およびJSON objectでなければならない`json` stringを
持ちます。hostが検証するのはこのenvelopeだけで、object内部のaddon固有fieldは検証
しません。schemaを定義したaddonがその意味を解釈します。

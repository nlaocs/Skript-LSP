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

WIT packageは`nlaocs:skript-parser-addon@0.3.0`です。`parser-addon` worldはhost serviceを
importし、guest実装をexportします。

Guest export:

- `addon`: static manifestとhost profileのnegotiation
- `hooks`: parser stageの観測、変換、override
- `text-macro`: virtual UTF-8 source textに対するedit
- `tree-macro`: losslessなindentation-based RawTreeに対する対象指定edit
- `ast-macro`: parse済みAST arenaの置換

Host import:

- `state-store`: compare-and-swapとprefix scanを備えたscope付きkey/value storage
- `dynamic-syntax-registry`: syntax definitionの追加、override、削除

parser payloadはすべてWITのrecordとvariantです。JSONはABIに含まれません。RawTreeとASTは
node ID arenaを使い、Component Model上で再帰しない値として表現します。

## 互換性

各manifestはdiagnostic用のcomponent IDとcomponent versionを公開します。

package versionはWITのshapeを示します。Text editのanchor追加で0.1.0から0.2.0へ、
lossless RawTreeと対象指定TreeEditの追加で0.3.0へ変わりました。manifestの現在の`abi`値は
1.2で、runtime handshakeとして`major.minor`の完全一致が必要です。

capabilityはclosed enumではなく、安定した文字列IDと独立した整数versionで表します。
新しいcomponentが未知のcapabilityを記述しても、古いhostがmanifestをliftできます。

- 必須capabilityが存在しない、またはversionが古い場合、初期化を拒否します。
- 任意capabilityが存在しない、またはversionが古い場合、無視します。
- capability IDが空、または重複しているmanifestは不正です。
- hostとguestは同じnegotiation ruleを使います。hostがcomponent manifestを検証したあと、
  guestが`addon.initialize`でhost profileを検証します。

hostはText macroとTree macroをadvertiseし、実行します。AST macroはcontractだけが存在し、
まだadvertiseされません。

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

生成rootと生成Sectionの子は同じTree macro stageへ再投入されます。再帰はdepth、総node数、
hook call数、output byte数のquotaで制限します。さらにmacro identity、入力origin、subtree
内容を組み合わせ、直接・間接cycleを検出します。cycle時は現在のnodeを保持し、component
failureと`tree-macro-cycle` diagnosticを返します。

各候補はStateStore invocation transaction内で実行します。TreeEdit検証とstate採用はatomic
です。addon error、trap、不正edit、cycleでは現在のnodeを保持し、その候補の書き込みを
rollbackします。型付きRejectまたはpipeline quota errorでは、元tree、source provenance、
parse StateStore savepointを復元します。成功したeditはExpansionGraphへTree entryを追加し、
再帰的に生成されたnodeから完全なcall-site backtraceを辿れます。

## Hook rule

subscriptionはtarget、phase、signed priority、modeを指定します。

- `observe`: payloadを読み取れますが、置き換えてはいけません。
- `transform`: 後続hookへ渡すreplacement payloadを返せます。
- `override`: targetの通常処理に代わって処理します。

hostはcomponent登録時に、mode固有の動作、payload variant、subscription、capabilityを
検証します。runtime limitとtrap処理はWasmtime hostの責務です。

subscriptionの順序は決定的です。

1. exact registration target
2. syntax-kind target
3. parse-stage target
4. signed subscription priority
5. component load順
6. component manifest内の宣言順

実際の比較では、最初の3つをtarget specificityとして比較したあと、残りを順に比較します。
overrideがhandledを返すと、後続の一致するhookを停止します。addon errorはcomponent failure
として報告されます。trap、timeout、fuel枯渇、resource-limit違反が起きたcomponentは
無効化されます。

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

## テスト

integration testより先に埋め込みcomponentをbuildします。

```sh
cargo run -p xtask --locked -- build-core-library
cargo run -p xtask --locked -- build-test-components
cargo test -p parser-wasm --locked
```

workspace全体のcheckでは、host専用dependencyがguest componentへ誤って必要になっていない
ことも確認します。

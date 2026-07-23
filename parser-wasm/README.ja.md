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

WIT packageは`nlaocs:skript-parser-addon@0.1.0`です。`parser-addon` worldはhost serviceを
importし、guest実装をexportします。

Guest export:

- `addon`: static manifestとhost profileのnegotiation
- `hooks`: parser stageの観測、変換、override
- `text-macro`: virtual UTF-8 source textに対するedit
- `tree-macro`: indentation-based RawTreeの置換
- `ast-macro`: parse済みAST arenaの置換

Host import:

- `state-store`: compare-and-swapとprefix scanを備えたscope付きkey/value storage
- `dynamic-syntax-registry`: syntax definitionの追加、override、削除

parser payloadはすべてWITのrecordとvariantです。JSONはABIに含まれません。RawTreeとASTは
node ID arenaを使い、Component Model上で再帰しない値として表現します。

## 互換性

各manifestはdiagnostic用のcomponent IDとcomponent versionを公開します。

package versionはWITのshapeを示します。manifestの`abi` fieldはruntime handshakeで、
現在は`major.minor`の完全一致が必要です。

capabilityはclosed enumではなく、安定した文字列IDと独立した整数versionで表します。
新しいcomponentが未知のcapabilityを記述しても、古いhostがmanifestをliftできます。

- 必須capabilityが存在しない、またはversionが古い場合、初期化を拒否します。
- 任意capabilityが存在しない、またはversionが古い場合、無視します。
- capability IDが空、または重複しているmanifestは不正です。
- hostとguestは同じnegotiation ruleを使います。hostがcomponent manifestを検証したあと、
  guestが`addon.initialize`でhost profileを検証します。

WIT contractにはtext、tree、AST macro capabilityが定義されていますが、現在のhostは
これらのmacro pipelineをadvertiseも実行もしません。ABIを段階的に実装してもcapability
IDが変わらないよう、定数だけ先に定義されています。

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

StateStoreはhookと将来のmacro呼び出しから使えるhost importです。

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
- `dynamic_syntax_snapshot`: 候補をfreezeし、順位付きsnapshotを取得する
- `dispatch`: 1回のdispatch transaction用convenience API

`HostConfig`はcall fuel、epoch timeout、Wasmtimeのmemory/table/instance limit、dispatch
output quota、StateStore設定、任意のsyntax Catalogを管理します。

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

## テスト

integration testより先に埋め込みcomponentをbuildします。

```sh
cargo run -p xtask --locked -- build-core-library
cargo run -p xtask --locked -- build-test-components
cargo test -p parser-wasm --locked
```

workspace全体のcheckでは、host専用dependencyがguest componentへ誤って必要になっていない
ことも確認します。

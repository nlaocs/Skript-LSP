# Skript REPL

[English](README.md)

`skript-repl`は、SkriptSyntaxGenerator（SSG）の正確なsnapshotと、
Skript-LSPと同じWASM parser hostを利用する独立したparser consoleです。
一つのEffectと、確定した複数行Skript documentの両方を解析できます。
Skriptを実行せず、Minecraft serverへの接続も行いません。

REPLは入力間で、読み込んだCatalog、CoreLibrary、任意のparser addon、
commit済みStateStoreを再利用します。Effect専用parserではなく、共通parserの
小さな対話frontendという位置付けです。

## ビルド

CoreLibraryは実行ファイルへ組み込まれるため、先にComponentを作成します。

```console
rustup target add wasm32-unknown-unknown
cargo run -p xtask --locked -- build-core-library
cargo build -p skript-repl --locked
```

Windowsの実行ファイルは`target/debug/skript-repl.exe`です。

## SnapshotとAddon

SSG出力directory、またはその`Manifest.json`を指定します。

```console
skript-repl.exe --snapshot C:\server\plugins\SkriptSyntaxGenerator --repl
```

`--snapshot`を省略した場合は`SKRIPT_REPL_SNAPSHOT`、次にcurrent directoryを
使用します。CoreLibrary起動前にsnapshot全体を検証します。SSG schema 3から6に
対応し、schema 5と6では`Language.json`が必須、schema 3と4では不要です。

追加のparser addon Componentは、command lineの順序で読み込めます。

```console
skript-repl.exe --snapshot C:\snapshot \
  --addon C:\addons\reflect-parser.wasm \
  --addon C:\addons\project-rules.wasm \
  --repl
```

`--addon`は複数指定できます。AddonはCoreLibraryと同じruntime profile、
SSG Catalog参照、hook、dynamic syntax registry、transactional StateStoreを
利用できます。

## 単発Effect

Effectを引数として渡すと、一つのsimple lineを解析して終了します。

```console
skript-repl.exe "send 1 to console"
skript-repl.exe --json "broadcast \"hello\""
skript-repl.exe --event "on join:" "send player"
skript-repl.exe --section "loop all players:" "continue"
```

これは以前のEffectCommandCLIの契約を維持するmodeです。選択されたEffectまたは
EffectSection、pattern、capture、再帰Expression、type、multiplicity、addon、
default、diagnostic、parse時間を表示します。成功したhookの書き込みを含め、
report作成後にparse transactionを破棄します。

終了codeは次のとおりです。

| Code | 意味 |
| ---: | --- |
| `0` | Effectが一致した。 |
| `1` | 入力は有効だがEffectが一致しなかった。 |
| `2` | command line引数が不正。 |
| `3` | snapshot、host、parser、addon、I/Oの準備に失敗した。 |

## 対話REPL

Effectを省略するか、`--repl`を指定します。

```console
skript-repl.exe --snapshot C:\snapshot --repl
```

単純な一行は今までどおり即座にEffectとして解析します。

```console
skript> send 1 to console
```

StructureまたはSection headerを入力するとdocument draftが始まります。terminalは
次のindentを提案しますが、利用者が自由に変更できます。

```console
skript> on join:
......>     send "hello" to console
......>     loop all players:
......>         send loop-player's name to console
......>     send "done" to console
......>
```

空行または`:submit`で確定します。`:cancel`とCtrl+Cは現在のdraftだけを破棄します。
EOF時は空でないdraftを確定してから終了します。terminalへの複数行pasteは、途中の
空行やblock commentを含めて一つのdraftとして保持します。不正な入力を通すために
元の空白や改行を書き換えることはありません。

確定したdocumentは`ParserHost::parse_document`へそのまま渡されます。Text/Tree
macro、RawTree、二段階Structure parse、Event/Section scope、再帰的な
Effect/Condition/Expression、AST macro、diagnostic、transaction commitまで、LSPと
共有する経路を使います。回復可能な失敗ではpartial treeを出力し、REPLは終了しません。

成功したdocument入力はparser addonのStateStore書き込みをcommitします。REPLで
成功した一行Effectも、採用branchの書き込みだけを保持します。rejectまたは
incomplete候補の書き込みはrollbackします。これにより将来、addonが`import ...`の
ようなparse時宣言を実装し、REPL本体にaddon固有処理を追加せず後続入力へ公開できます。
cancelしたdraftと未確定draftはparserへ入らないため、stateを変更しません。

## REPL command

| Command | 動作 |
| --- | --- |
| `:help` | command一覧を表示する。 |
| `:reload` | snapshotと全`--addon` Componentを再読込する。 |
| `:event <header>` | Event文脈を選択する。末尾の`:`は任意。 |
| `:event off` | EventとSection文脈を消去する。 |
| `:events` | Catalogおよび動的登録Eventを一覧表示する。 |
| `:section <header>` | 解析済みSection文脈をpushする。末尾の`:`は任意。 |
| `:section pop` | 最内側のSection文脈をpopする。 |
| `:section clear` | 全Section文脈を消去する。 |
| `:context` | 現在の手動文脈を表示する。 |
| `:json on`, `:json off` | report形式を切り替える。 |
| `:submit` | 現在のdocument draftを確定する。 |
| `:cancel` | 現在のdocument draftを破棄する。 |
| `:quit`, `:exit` | 終了する。 |

入力中のdocumentへcommandを混入させません。閉じていない`###` block commentの
入力中もREPL meta commandは有効なので、`:submit`で診断でき、`:cancel`で脱出できます。
Skript source自体へ`:submit`のような行を入れる場合は、複数行blockとしてpasteします。
`:event`と`:section`は、一行Effectの
仮想body contextだけに適用します。複数行documentは手動文脈を継承せず、記述どおりに
解析します。手動選択は保持され、次の一行Effectで再び利用されます。

## Report

通常のdocument出力は、行番号付き原文と
`Structure -> Event/Section -> Effect/Condition -> Expression/Type` treeを表示します。
解決済みnodeにはregistration identity、実装class、addon、pattern、return type、
multiplicity、default provider、metadata、addon定義public dataを保持できます。
diagnosticとrecoveryのspanは、確定したsource全体を参照します。
色表示に対応したterminalでは、行番号付きsourceも一行Effectと同じ構文カテゴリ色で
表示します。redirectされた出力とJSONには色を含めません。
補間文字列では引用符と本文を白、区切りの`%`を薄いグレーで表示し、内側のExpressionは
通常の意味色を維持します。

JSONが機械読み取り用の契約です。

- 単発および一行Effect reportは`schemaVersion: 7`。
- 複数行document reportは`schemaVersion: 1`。
- document nodeはarena形式で、`roots`、`children`、captureが数値node IDを参照する。
- mapped spanは全original originとmacro expansion IDを保持する。
- `state`はdocument revisionでcommitされたread/writeを要約する。

通常表示は読みやすさのため変更される可能性があります。consumerはJSONを使い、
report schemaとSSG snapshot schemaを別々に確認してください。

## 現在の境界

利用できる構文はsnapshot、CoreLibrary、読み込んだparser addonに依存します。
Minecraft上での実行ではなく静的解析です。一つの確定document内では二段階parseにより
Function宣言を登録して利用できますが、別々の入力をまたぐproject-wide symbol indexへ
宣言を蓄積する機能はまだありません。複数fileのvariable、symbol、serverへ問い合わせる
意味解析は、今後のLSP実装範囲です。

## テスト

```console
cargo test -p skript-repl --locked
```

単発互換、Event/Section文脈、複数行buffer、indent、paste、明示submit、cancel、EOF、
block comment、partial tree、入れ子document、default Expression、JSON、modern/legacy
SSG snapshotを検証します。

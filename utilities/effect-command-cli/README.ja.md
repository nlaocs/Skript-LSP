# Effect Command CLI

[English](README.md)

`effectcommandcli`は、1つのSkript EffectをSkriptSyntaxGenerator（SSG）の
schema 3 / 4 snapshotに対して解析する独立した確認用utilityです。Effectは実行しません。
`ssg`、`syntaxes`、`skript-parser`、`parser-wasm`、必須CoreLibraryを接続する
小さな実例としても利用できます。

## Build

CoreLibraryを実行ファイルへ埋め込むため、先にComponent artifactを生成します。

```console
cargo run -p xtask --locked -- build-core-library
cargo build -p effect-command-cli --locked
```

Windowsの成果物は`target/debug/effectcommandcli.exe`です。

## Snapshot

SSGの出力directoryまたは`Manifest.json`を指定できます。

```console
effectcommandcli.exe --snapshot C:\server\plugins\SkriptSyntaxGenerator "send 1"
```

`--snapshot`を省略した場合は`EFFECT_COMMAND_CLI_SNAPSHOT`、次にcurrent directoryを
使用します。CoreLibraryの起動前にsnapshot全体を検証するため、未対応schema、digestの
不一致、file不足、参照不整合があるsnapshotでは解析を開始しません。

## 単発モード

Effect引数を渡すと1行だけ解析して終了します。

```console
effectcommandcli.exe "send 1"
effectcommandcli.exe --json "broadcast \"hello\""
```

人間向け出力では、採用Effect、addon、実装class、登録pattern、pattern AST、capture、
期待されるSkript type、解決されたJava return type、multiplicity、再帰Expression、
parse tag、parse mark、代替候補、最遠failureを表示します。JSON reportには
`schemaVersion: 1`を持たせ、SSG schemaとは独立してreaderをversion管理できます。

`patternElements`は、選択されなかったbranchも含む登録pattern全体のASTです。
`elements`には、実際の照合へ参加したregexと型付きExpression captureだけを格納します。

addonによっては意図的なcatch-all Effectを登録します。例えばskript-reflectのexpression
statementは`[1:await] <.+>`であるため、このaddonを含むsnapshotでは空でない任意入力が
正しいEffectになり得ます。その場合、CLIはunknownを作らず、採用されたcatch-allを表示します。

終了codeは固定します。

| Code | 意味 |
| ---: | --- |
| `0` | 登録Effectに一致した。 |
| `1` | 入力は有効だが一致するEffectがない。 |
| `2` | CLI引数が不正。 |
| `3` | snapshot、host、parser、streamの準備に失敗した。 |

## REPLモード

Effectを省略するか`--repl`を渡すと、読み込んだsnapshot、catalog、parser hostを
再利用して連続解析します。

```console
effectcommandcli.exe --snapshot C:\server\plugins\SkriptSyntaxGenerator

effect> send 1
effect> broadcast "hello"
effect> :json on
effect> :reload
effect> :quit
```

利用可能なcommandは`:help`、`:reload`、`:json on`、`:json off`、`:quit`、
`:exit`です。構文不一致や不正な1行があってもREPLは終了しません。EOFでは正常終了し、
入力のinterrupt後はpromptへ戻ります。

## 現在の境界

登録Expression、variable、literal、custom leaf、期待type、解決return class、再帰的な
Expression captureは現段階で表示します。data modelはgenericなFunction leafを表現できますが、
CoreLibraryはまだSkript標準Function呼び出しやoverload・引数treeを解析しません。
Function対応の完了は[Issue #79](https://github.com/nlaocs/Skript-LSP/issues/79)で
引き続き追跡し、genericな`parserId`だけを完全に解決されたFunctionとして扱いません。

このutilityが解析するのはトップレベルのEffect 1行だけです。`.sk` file全体の解析、
Text/Tree macroの実行、Minecraft上の処理実行は行いません。

## Test

```console
cargo test -p effect-command-cli --locked
```

integration testでは、repositoryに含まれるSkript 2.15.4のmulti-addon snapshotと、
Skript 2.6.4/Minecraft 1.12.2のlegacy schema 3 snapshotを使用します。単発JSON、
不明Effect、再帰要素、REPL継続、表示切替、snapshot reloadを検証します。

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
effectcommandcli.exe "send sin(abs(-1))"
```

人間向け出力では、採用Effect、addon、実装class、登録pattern、pattern AST、capture、
期待されるSkript type、解決されたJava return type、multiplicity、再帰Expression、
parse tag、parse mark、代替候補、最遠failureを表示します。JSON reportには
`schemaVersion: 3`を持たせ、SSG schemaとは独立してreaderをversion管理できます。
人間向け出力の`parseTime`は、1 millisecond以上なら`ms`、それ未満なら`ns`で表示します。
JSONでは同じ時間を整数nanosecondの`parseDurationNs`として出力します。この時間には
parse処理だけを含み、SSG snapshotの読み込みとindex構築は含みません。

`patternElements`は、選択されなかったbranchも含む登録pattern全体のASTです。
`elements`には、実際の照合へ参加したregexと型付きExpression captureだけを格納します。

addonによっては意図的なcatch-all Effectを登録します。例えばskript-reflectのexpression
statementは`[1:await] <.+>`であるため、このaddonを含むsnapshotでは空でない任意入力が
正しいEffectになり得ます。その場合、CLIはunknownを作らず、採用されたcatch-allを表示します。

登録Effectの意味のあるprefixまで一致し、型付きcaptureだけが失敗した場合は`incomplete`を返します。
Effect identityと失敗captureのspanを保持し、候補自体を認識できない入力だけを`unknown`にします。

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

SSGに登録されたSkript/addon Functionは構造化Expression nodeとして解析します。Function名、
definition/registration ID、addon、return type、multiplicity、宣言parameter名、named binding、
省略optional parameter、再帰解析済みargument Expressionを表示します。opaqueなWASM Function
leafは`structured: false`のまま区別できます。

`.sk`内で宣言されたユーザーFunctionの登録は未接続です。parser側には既に
`lookup_functions`からdocument definitionを受け取る入口があり、Structureを含むfile全体解析後に
宣言収集とproject symbol管理を接続します。残るCLI作業は
[Issue #79](https://github.com/nlaocs/Skript-LSP/issues/79)で引き続き追跡します。

このutilityが解析するのはトップレベルのEffect 1行だけです。`.sk` file全体の解析、
Text/Tree macroの実行、Minecraft上の処理実行は行いません。

## Test

```console
cargo test -p effect-command-cli --locked
```

integration testでは、repositoryに含まれるSkript 2.15.4のmulti-addon snapshotと、
Skript 2.6.4/Minecraft 1.12.2のlegacy schema 3 snapshotを使用します。単発JSON、
不明Effect、再帰Function/Expression、REPL継続、表示切替、snapshot reloadを検証します。

# Effect Command CLI

[English](README.md)

schema 7はimplicit/defaultの子Expressionと、省略引数のrejected/unresolvedを通常表示・
JSONの両方へ表示します。例えば`send stone`には`--event "on join"`などCommandSenderを
提供するEventが必要です。`send stone to console`は送信先を明示しています。原文は維持します。
[共通DefaultExpressionモデル](../../docs/default-expressions.ja.md)を参照してください。

`effectcommandcli`は、1つのSkript EffectをSkriptSyntaxGenerator（SSG）の
schema 3 / 4 / 5 snapshotに対して解析する独立した確認用utilityです。Effectは実行しません。
`ssg`、`syntaxes`、`skript-parser`、`parser-wasm`、必須CoreLibraryを接続する
小さな実例としても利用できます。

## Build

CoreLibraryを実行ファイルへ埋め込むため、先にComponent artifactを生成します。

```console
rustup target add wasm32-unknown-unknown
cargo run -p xtask --locked -- build-core-library
cargo build -p effect-command-cli --locked
```

Windowsの成果物は`target/debug/effectcommandcli.exe`です。

## Snapshot

SSGの出力directoryまたは`Manifest.json`を指定できます。

```console
effectcommandcli.exe --snapshot C:\server\plugins\SkriptSyntaxGenerator "send 1 to console"
```

`--snapshot`を省略した場合は`EFFECT_COMMAND_CLI_SNAPSHOT`、次にcurrent directoryを
使用します。CoreLibraryの起動前にsnapshot全体を検証するため、未対応schema、digestの
不一致、file不足、参照不整合があるsnapshotでは解析を開始しません。

schema 5では`Language.json`が必須ですが、schema 3と4では不要です。必要なfile一覧は
[`ssg`のformat説明](../../ssg/README.ja.md)を参照してください。

## 単発モード

Effect引数を渡すと1行だけ解析して終了します。

```console
effectcommandcli.exe "send 1 to console"
effectcommandcli.exe --json "broadcast \"hello\""
effectcommandcli.exe "send sin(abs(-1)) to console"
effectcommandcli.exe --event "on join:" "send join message"
effectcommandcli.exe --section "loop all players:" "continue"
effectcommandcli.exe --section "loop all players" --section "if loop-player is online:" "exit 2 sections"
```

`--event <HEADER>`を指定すると、選択したSkript Eventの内部としてEffectを解析します。
Event headerはStructEventとsnapshotのEvent catalogを通して照合します。末尾の`:`は任意で、
`on join`、`on join:`、`join`は同じEventを選択します。人間向け出力には解決したEvent classと
EventValue件数を表示します。JSON reportは各EventValueのSSG登録、順序、changer、validator、
除外条件、pattern、addon metadataを保持します。

`--section <HEADER>`を外側から内側の順に繰り返すと、人工的なSection stack内でEffectを解析できます。
各headerは自由形式のlabelとして保存せず、通常のSection parserで解析します。末尾の`:`は任意です。
選択したregistration identity、addon、flag、capture、return type、multiplicity、addon metadataは、
parser所有のread-only scope stackを通してCoreLibraryと他のWASM hookから参照できます。JSON reportでは
同じ情報を`context.sections`に出力します。
Section選択時はrootのexit hookをすぐ実行せず、enter hook後のtransaction stateを保持します。
`pop`と`clear`は保存したtransactionへ戻すため、stateを持つWASM addonから見ても後続Effect解析と
同じscope lifetimeになります。dynamic registrationの`ownerComponentId`はcatalog addon metadataと
分離して出力します。

人間向け出力では、採用Effect、addon、実装class、登録pattern、pattern AST、capture、
期待されるSkript type、解決されたJava return type、multiplicity、再帰Expression、
public semantic data、parse tag、parse mark、代替候補、最遠failureを表示します。
JSON reportには`schemaVersion: 7`を持たせ、SSG schemaとは独立してreaderをversion管理できます。
人間向け出力の`parseTime`は、1 millisecond以上なら`ms`、それ未満なら`ns`で表示します。
JSONでは同じ時間を整数nanosecondの`parseDurationNs`として出力します。この時間には
RawTree解析、parserによる解析、transaction rollbackを含みます。SSG snapshotの読み込み、
index構築、reportの構築と描画は含みません。

解決された各Expressionは、`metadata`とは別にnode-localな`publicData` recordを出力します。
recordは`schemaId`、`schemaVersion`、`json`を持ち、valid JSONはJSON stringへ二重 encodeせず
structured raw valueとして出力するため、大きなintegerやdecimalの桁表記を保持します。
ネストしたchildは自分のrecordだけを持ち、Grouped wrapperのlistは空のままです。
人間向け出力にも同じschema/versionとJSON objectを表示し、表示するsourceは変更しません。

人間向けのparse失敗は`miette`で表示し、最も遠くまで解析できたfailure spanを
source上へ直接示します。人間向けの書式は可読性のため変更される可能性があります。
安定した機械向け契約はJSON出力であり、構造変更時は`schemaVersion`を更新します。

`patternElements`は、選択されなかったbranchも含む登録patternのASTです。
レポート生成には上限があり、pattern ASTの再帰は深さ16、ネストした
Expression情報は深さ8で打ち切られます。`elements`には、実際の照合へ参加した
regexと型付きExpression captureだけを格納します。

SSGの静的なEffectSection registrationは通常のEffectより先に候補として扱われ、
Section syntax identityとともにレポートされます。
通常のSection registrationはEffect候補になりません。JSON reportに独立した
`effectSection` fieldは追加されません。

人間向け出力はstdoutがterminalで、`NO_COLOR`が未設定の場合だけ色付きになります。

addonによっては意図的なcatch-all Effectを登録します。例えばskript-reflectのexpression
statementは`[1:await] <.+>`です。ただし、snapshotに登録されているだけでは不十分です。
読み込んだWASM componentがregex captureに対応し、入力を検証する必要があります。
native側はWASM routeのないregex構文を照合対象から除外するため、広いpatternだからといって
空でない任意入力が成功するわけではありません。

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

effect> send 1 to console
effect> broadcast "hello"
effect> :event on join:
effect> :section loop all players:
effect> send join message
effect> :context
effect> :section pop
effect> :section clear
effect> :event off
effect> :json on
effect> :reload
effect> :quit
```

利用可能なcommandは`:help`、`:reload`、`:event <HEADER>`、`:event off`、`:events`、
`:section <HEADER>`、`:section pop`、`:section clear`（または`off`）、`:context`、`:json on`、
`:json off`、`:quit`、`:exit`です。`:events`はSSG catalog Eventと
WASM addonが動的登録したEventの両方を表示します。Event選択は常に実際のSkript Event headerを
使うため、StructEventとaddon WASM hookへ同じ入力が渡ります。Section commandは選択中Eventを
変えずにparser所有stackをpush、pop、clearします。別のEventを選択した場合は新しいroot contextに
なるためSection stackをclearします。構文不一致や不正な1行があってもREPLは終了しません。
EOFでは正常終了し、入力のinterrupt後はpromptへ戻ります。

## 現在の境界

SSGに登録されたSkript/addon Functionは構造化Expression nodeとして解析します。Function名、
definition/registration ID、addon、return type、multiplicity、宣言parameter名、named binding、
省略optional parameter、再帰解析済みargument Expressionを表示します。opaqueなWASM Function
leafは`structured: false`のまま区別できます。

ライブラリ側では既に2段階のStructure解析でdocumentのFunction宣言を収集し、
`lookup_functions`から参照できます。ただし、この1行解析CLIはその宣言をロードしないため、
セッション内でユーザー定義Functionは使用できません。project全体のsymbol管理も未実装です。
残るCLI作業は
[Issue #79](https://github.com/nlaocs/Skript-LSP/issues/79)で引き続き追跡します。

このutilityが解析するのはトップレベルのEffect 1行だけです。`.sk` file全体の解析、
Text/Tree macroの実行、Minecraft上の処理実行は行いません。

## Test

```console
cargo test -p effect-command-cli --locked
```

integration testでは、repositoryに含まれるSkript 2.15.4のmulti-addon snapshotと、
Skript 2.6.4/Minecraft 1.12.2のlegacy schema 3 snapshotを使用します。単発JSON、
不明Effect、再帰Function/Expression、REPL継続、表示切替、Event文脈の選択、
入れ子Section文脈のpush/pop/clear、loop内Effect/Expression、snapshot reloadを検証します。

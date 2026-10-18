# Neovimで診断・補完・意味情報を確認する

Neovim 0.12 以降の標準 LSP client を使う。`nvim-lspconfig` や追加 plugin は不要で、
この手順は既存の Neovim 設定ファイルを書き換えない。
完成済みの `skript-lsp` executable と、対象サーバーから生成した完全な SSG Snapshot を用意する。

## 接続設定

[起動例](../examples/neovim/skript-lsp.lua) は次の環境変数を受け取り、path を絶対化する。

| 環境変数 | 値 |
| --- | --- |
| `SKRIPT_LSP_EXE` | `skript-lsp` executable の path |
| `SKRIPT_LSP_SNAPSHOT` | 完全な SSG Snapshot directory |
| `SKRIPT_LSP_ROOT` | 今回開く単一 project の root directory |
| `SKRIPT_LSP_ADDON_PATHS` | 任意。追加 WASM component の path を JSON 配列で指定 |

起動 command は `{ exe, "--stdio" }`。
Snapshot と Addon は `init_options = { snapshotPath = ..., addonPaths = ... }` として
initialize request に渡す。`root_dir` は `SKRIPT_LSP_ROOT` に固定する。
この例は `.sk` を filetype `skript` に対応付け、`vim.lsp.config` と `vim.lsp.enable` で接続する。

PowerShell では、repository root から次のように設定する。
`SKRIPT_LSP_EXE` は実際に用意した executable の場所へ変更する。

```powershell
$env:SKRIPT_LSP_EXE = (Resolve-Path .\target\debug\skript-lsp.exe).Path
$env:SKRIPT_LSP_SNAPSHOT = (Resolve-Path .\utilities\skript-repl\tests\data\current-multi-addon-2.15.4).Path
$env:SKRIPT_LSP_ROOT = (Get-Location).Path
# 必要な場合だけ指定する。省略時は追加 Addon なし。
# $env:SKRIPT_LSP_ADDON_PATHS = '["C:/path/to/addon.wasm"]'
```

普段の設定から独立して起動する場合は、`-u` にこの例だけを指定する。
`-i NONE` は ShaDa の読み書き、`-n` は swap file、`--noplugin` は plugin の自動読込を無効にする。

```powershell
nvim -n -i NONE --noplugin -u .\examples\neovim\skript-lsp.lua .\example.sk
```

通常の設定を読み込んだ Neovim から試す場合は、次の command を実行してから `.sk` を開く。
通常設定の plugin や既存 LSP client も同時に動作し得るので、独立起動の確認とは区別する。
既存設定への自動追記や plugin のインストールは行わない。

```vim
:luafile /path/to/checkout/examples/neovim/skript-lsp.lua
:edit /path/to/project/example.sk
:checkhealth vim.lsp
```

## ホバーを表示する

解析が完了してから構文、文字列、変数、関数にカーソルを置き、`K` または
`:lua vim.lsp.buf.hover()` で標準floating windowを開く。
空白・コメント・未知の構文・位置対応が曖昧なマクロ展開では表示を返さない。
編集直後の再解析中もnullになり、完了後に再度要求すると現在の情報を表示する。
変数にはnativeの要求型と、現在の代入履歴から独立に得た`Inferred type`を分けて表示する。
後者は要求型への変換前の保存型であり、根拠がないときはUnknownになる。literal scalarや
安全な代入経路の根拠が必要で、型付きFunction/parameter、listの実値や非空性まで保証しない。
opaqueなconverter/callbackを通る読取では以前の保存型を保持せず、globalのruntime値や別eventの実行順も推測しない。
古いschema 7 snapshotも引き続き読み込めます。cloner/`beforeChange`の確かな情報がない場合、
影響する代入や読取の事実はUnknownのまま保持し、型宣言だけから保証を補いません。

Neovim標準のcapabilitiesではMarkdown表示を選び、説明のHTMLタグを変換して表示する。
定義元Javaクラス、採用pattern、型、Addon、Examplesを確認できる。

同じ隔離環境で `tests/neovim/hover.lua` を実行すると、標準の
`vim.lsp.buf.hover()` が作った実floating bufferの本文と範囲extmarkを確認する。
構文のSSG説明、関数signature、local/global変数、日本語と絵文字のUTF-16、未完成文書の正常な兄弟、
空白/コメント、編集、close/reopenを確認し、実要求と応答をJSONへ記録する。
制御workerによる決定的な旧結果破棄の検証はRustの単体試験が担当する。

`tests/neovim/hover_formatting.lua` はMarkdownを要求し、実SSGのName、Display Name、Player List Nameを
標準floating windowで確認する。HTMLタグと文字参照、色タグ、複数行の例の字下げ・境界、
完全修飾Javaクラス、2回目のhoverによるfocus、scroll、close、編集後の表示更新をJSONへ記録する。
headlessの標準rendererを対象とし、第三者のMarkdown pluginやdesktopの画面描画までは保証しない。

## 入力補完を使う

付属設定はNeovim標準の`vim.lsp.completion`をbufferごとに有効にする。
挿入modeで`Ctrl-Space`を押すと候補menuを開き、`Ctrl-N`/`Ctrl-P`で選択、`Ctrl-Y`で確定する。
`{`の入力でも自動要求する。通常の文字を一つ打つたびに全文解析を追加要求する設定は行わない。
snippet対応時は必須引数・型slotをtab stopにする。Neovim標準のsnippet操作を利用でき、追加pluginは不要。

文頭のCatalogテンプレート、式の関数とExpression、同じ文書で観測した静的変数名を表示する。
候補は型・Addon guard・実行scopeへの適合性を保証しない。local変数の候補名は、同じbody/祖先で
カーソルより前に現れた参照履歴に限定する。解析済みの候補には現在のflow推論の説明を添えられるが、
参照履歴そのものを宣言やruntime値の証明にはしない。
未完成行ではPattern ASTに沿う継続語も示す。`teleport player t`から`to`を選ぶと入力済み引数を保持する。
説明paneには元の全文法を表示する。型・guardは未判定であり、同じ継続語をまとめた場合は代表patternの説明になる。
編集直後でも構文候補は現在の原文から先に返せる。参照履歴や関数の確定には現在文書の解析が必要で、
即時の構文応答の後に同じ要求へ追加送信はしない。検索時間・解析待ち・clientのmenu描画は分けて扱う。
詳しい制約は[LSPの入力候補](lsp.ja.md#入力候補の範囲)を参照する。

`tests/neovim/completion.lua`は標準clientの実応答とmenu・確定後のbufferを確認し、
JSONに要求往復時間、解析・候補検索logを分けて保存する。headlessの標準clientを対象とする。

Blinkでは付属の`examples/neovim/lsp_completion_edit.lua`を設定の`lua/lsp_completion_edit.lua`へ置き、
既存optsの` sources.providers.lsp.transform_items = require("lsp_completion_edit") `を設定できる。
この公開hookはversion 1の`nlaocs.lsp.completion-edit`を持つ項目だけに完全な編集を適用する。
挿入文字列に既存の開き括弧を含めると、Blink自身の括弧重複検査が使われる。
Function kind、既存引数、他の補完項目、通常の自動括弧設定を維持し、filetypeによる無効化は不要になる。
このmetadataはLSP標準fieldの意味を変えるものではなく、任意adapter向けの公開data契約である。
既存のLSP用transformがある場合は、その処理と合成する。

## 引数案内を使う

標準clientでは `:lua vim.lsp.buf.signature_help()` で現在の文法と引数位置を表示できる。
Blinkを使う構成では既存の `opts` に `signature = { enabled = true }` を加えると、空白・`(`・`,`で標準の案内windowを開ける。
既存の補完キー設定を変更する必要はない。この設定はBlink全体の有効化であり、他の対応LSPにも適用される。
文法内の位置は入力支援であり、引数型やAddon guardの成功を示さない。曖昧な場合は文法だけを表示する。

## 定義移動・参照検索・暗黙引数

解析後、変数や関数へカーソルを置き、`:lua vim.lsp.buf.definition()`で定義候補へ移動する。
`:lua vim.lsp.buf.references()`は`includeDeclaration`を含む標準参照要求を使う。
既存の`gd`等のmappingがあればその設定を利用できる。未openのproject関数にも移動でき、複数の
書込候補はclientの一覧へ渡す。動的名や曖昧なmacro位置から一意のruntime bindingを推測しない。
projectの複数文書解析には全componentの`parser.project-stable-semantics` v1宣言が必要である。

bufferのhintを有効にするには、接続後に次を実行できる。

```vim
:lua vim.lsp.inlay_hint.enable(true, { bufnr = 0 })
```

表示の色はthemeの`LspInlayHint`を使う。例えば`vim.api.nvim_set_hl(0, "LspInlayHint",
{ link = "Comment", default = true })`で、既存の設定を保ちながら編集文字と区別できる。
無効化は同じAPIへ`false`を渡す。hintはinline virtual textであり、bufferのsourceやchangedtickを
変えず、通常cursorが移動する実文字を増やさない。コピー用の補完候補や自動修正ではない。

Playerを持つevent内の`send 1`等では、実際に初期化した省略引数を`to <event player>`のように示す。
明示receiverや、Playerを得られない`on load`へ同じ値を補作しない。literal defaultの保存済み実値は
snapshotにないため、`<registered default: 型名>`を示し、tooltipに値が利用できない旨を残す。
根拠不足のdefaultや複雑な省略branchは表示を省略する。

未open `.sk`の変更・削除は、Neovimがdynamic file watcher登録に対応すれば依存文書へ反映する。
clientが対応する標準inlay/token refreshにより、本文versionが同じ依存bufferも再取得する。
watcherを登録できない環境では、次のopen文書の編集または明示的なfile変更通知まで更新されない。

## Hover・Blink説明内の色

serverは採用Pattern・templateを`skript-pattern`、実行例を`skript`のMarkdown fenceへ分ける。
`examples/neovim/skript_documentation.lua`の`setup()`は、nofileのMarkdown/Blink文書buffer内にある
この二種類のfenceだけを着色する。通常のMarkdown fileやSkript sourceへ色を上書きしない。
起動例は同じdirectoryのmoduleを読み込むため、例を別directoryへ移す場合は両fileを配置する。
通常設定の`lua/skript_documentation.lua`へ置く構成では、`require("skript_documentation").setup()`でも利用できる。

patternは固定語・型slot・regex・対応する区切り、例は文字列・数値・変数・補間等の字句上の役割を示す。
特定の構文名やJava classの例外一覧は持たず、着色はparserの受理を保証しない。
bufferを変更せずUTF-8 byte extmarkを付ける処理であり、completionのlabel・snippet・挿入範囲や
既存のBlink provider・keymap・括弧処理を置き換えない。説明形式はcompletionとhoverが別々に交渉する。
PlainTextを選んだclientにはfenceやMarkdown記号を付けない。

## Semantic Tokens の色

付属設定はNeovimの標準capabilitiesへ `skriptEvent`、`skriptCondition`、`skriptEffect`、`skriptExpression`、
`skriptSection`、`skriptStructure`、`skriptLiteral`、`skriptInterpolationDelimiter` を追加し、
`@lsp.type.<分類>.skript` の色を定義する。役割別RGBはREPLのソース構文色と揃える。

| 分類 | RGB |
|---|---|
| Event | `#C55F73` |
| Condition | `#FC6255` |
| Effect | `#58C4DD` |
| Expression | `#83C167` |
| Section | `#5CD0B3` |
| Structure | `#9A72AC` |
| Variable | `#00FFFF` |
| Function | `#A0A0A0` |
| Interpolation delimiter | `#B0B0B0` |

たとえば `on load:` の `load` はEvent、`if 1 is 1:` の `if` はSection、`is` はConditionとして別の色になる。
文字列・変数名の `%…%` はdelimiterをgrayに分け、内側の式・変数・関数は自身の分類を維持する。
literalは既存の `Normal` リンクを維持し、REPLの白を固定せずthemeの前景色に従う。
`default = true` のため、既に定義したhighlight groupを上書きしない。好みのthemeでは同じgroupを設定できる。

Neovimの既定tokenTypesは標準分類だけであり、そのままのclientにはFunction名・変数参照だけを返す。
独自分類は意味の違う `property` / `method` 等へ置き換えない。
標準分類だけならclient設定が少なく済み、独自分類ならSkriptの構文上の役割ごとに色を選べる。
他clientでも同じ独自名への対応表明と色設定が必要になる。

確定した固定語と子の式を別範囲にし、親AST全体を同色で塗らない。
採用したnative captureの位置を最終ASTへ保持し、`if`等の固定部分とcapture全域を分離する。
macro変更後等に位置の根拠を確認できない親、未分類のcapture内、動的構文の一部は対象外になり得る。
詳細は[LSPの制約](lsp.ja.md)を参照する。

同じ隔離環境で `tests/neovim/semantic_tokens.lua` を実行すると、標準decoderのtoken範囲、
標準highlighterの実extmark、役割別RGB、Structure/Section/Conditionの区別、literalのNormalリンク、既存色の維持、
UTF-16、日本語・絵文字、文字列/変数名の補間、未完成文書、編集、同URI再openを検査する。
出力は既存の `SKRIPT_LSP_SMOKE_OUTPUT`、または専用directoryを示す `SKRIPT_LSP_OUTPUT` から指定する。
`SKRIPT_LSP_STANDARD_TOKENS=1` では標準capabilitiesだけの対照試験を行う。
`SKRIPT_LSP_CONFIGURATION` にLua設定のpathを指定すれば、配布用の設定でも同じ項目を検査できる。
これはheadlessの標準clientの検証であり、desktopのfont描画や第三者pluginの検証とは区別する。

## 独立した headless smoke

[smoke script](../tests/neovim/smoke.lua) は UI と通常の設定を読み込まずに使う。
結果 JSON の path を `SKRIPT_LSP_SMOKE_OUTPUT` へ指定する。親 directory は先に作成し、
workspace 内の専用出力先を選ぶ。Neovim の runtime state と log もその directory 下へ分離する。

```powershell
New-Item -ItemType Directory -Force ..\neovim-smoke-output | Out-Null
$env:SKRIPT_LSP_SMOKE_OUTPUT = Join-Path (Resolve-Path ..\neovim-smoke-output).Path 'result.json'
nvim --headless -n -i NONE --noplugin -u NONE -l .\tests\neovim\smoke.lua
```

script は project root 内の一意な `.sk` URI を持つ LF の buffer をメモリ上だけで編集する。
`.sk` file を保存せず、Snapshot と既存 script も変更しない。
確認内容は次のとおり。

- 不正な unclaimed 行を含む文書の診断受信と、正常入力への変更後の空診断。
- Unicode 行の診断末尾が UTF-16 の column に対応すること。日本語と補助平面文字を含む。
- 6 回の連続変更を実際に送信し、最後の version の空診断を受けること。
- 変更後の close が最後の受理 version の空診断を返すこと。
- 同一 URI を新しい buffer/version 0 で reopen し、古い診断を残さないこと。
- shutdown/exit により server が正常終了すること。

正常入力は、この fixture で対応済みの次の文書を使う。

```skript
on load:
    send "hello" to console
```

`LspNotify` で実際に送った URI・version・本文を取得し、`publishDiagnostics` の通知を
標準 handler へ転送する前に記録する。空配列も観測し、表示に残った診断だけで合否を決めない。
各待機は原則 30 秒、全体は 90 秒、終了処理は各 5 秒に制限する。
通知はそれぞれ最大 256 件まで記録する。最新結果・close・reopen 後は 500 ms の追加観測を行う。
この有限時間の smoke は、任意の遅延や全入力で古い結果が絶対に来ないことの証明にはならない。

成功時は exit code 0 と JSON の `status: "passed"`、失敗時は非 0 と
`status: "failed"` を返す。JSON には各確認結果、送信通知、受信診断、server capabilities、
終了結果、経過時間、失敗理由を保存する。出力先自体が不正・書込不能な場合は stderr と exit code を確認する。
終了に失敗した場合の強制停止対象は、この smoke 自身が起動した LSP client だけである。

API と起動引数は対象 Neovim の `:help vim.lsp.config()`、`:help vim.lsp.enable()`、
`:help LspNotify`、`:help --headless`、`:help -u`、`:help -i` で確認できる。

Neovim 0.12 では、同じ環境変数と隔離用引数で `tests/neovim/diagnostics.lua` を実行すると、
標準handlerが作ったvirtual textのextmarkとchunksも確認できる。
単独 `else`、複数captureの不正Effect、複数の子エラー、原因なしfallback、それらの混在を実LSPで検証する。
同位置・同一行の異なる位置・逆順・複数行の表示方式を確認する合成診断は、実LSPケースと分けて記録する。

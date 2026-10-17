# Neovim で文書診断を確認する

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

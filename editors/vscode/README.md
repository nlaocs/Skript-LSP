# Skript LSP for VS Code (prototype)

Existing native `skript-lsp` serverをstdioで接続する薄いクライアントです。`.sk` をSkriptとして登録し、補完、hover、診断、定義・参照、signature help、semantic tokens、inlay hintsを標準LSP経由で提供します。構文・型の解析は既存サーバーが担当します。拡張側に別parserはありません。

VS Code Stableの公開APIのみを使います。最低APIは1.100、型定義も1.100に固定しています。Insiders専用APIやproposed APIは使いません。Web版・virtual workspaceには対応しません。Remote環境では拡張ホスト側OS用の実行ファイルとデータが必要です。

## Build / package

Node.js 22以上、npmを用意し、このディレクトリで実行します。

```sh
npm ci --ignore-scripts
npm run check
npm test
npm run package
```

`skript-lsp-0.1.2.vsix` が生成されます。依存関係はlockfileで固定し、registryはnpm公式に限定しています。`ignore-scripts` でも明示した `npm run` は実行できます。実行ファイル、SSGデータ、WASM addonは同梱・自動取得しません。既存のサーバーbuildと完全なSSG exportディレクトリを別途用意してください（[サーバー仕様](../../docs/lsp.ja.md)）。

## Install / configure

InsidersではExtensionsの「Install from VSIX...」から生成物を選べます。最初は専用profile、または下記の専用data directoryを使ってください。Stableでも同じVSIXを利用できます。

```sh
code-insiders --user-data-dir /path/to/test-profile --extensions-dir /path/to/test-extensions --install-extension /path/to/skript-lsp-0.1.2.vsix
code-insiders --user-data-dir /path/to/test-profile --extensions-dir /path/to/test-extensions --new-window /path/to/project
```

Settingsで次を指定します。Windowsの実行ファイルは `.exe`、JSONでは `C:/...` またはescapeしたbackslashを使います。

```json
{
  "skriptLsp.serverPath": "/path/to/skript-lsp",
  "skriptLsp.snapshotPath": "/path/to/complete-ssg-export",
  "skriptLsp.addonPaths": []
}
```

パスは絶対パス、または下記project rootからの相対パスです。PATH検索・shell展開・`~`・`${workspaceFolder}` 展開はしません。空白を含むパスも一つのパスとして扱います。`snapshotPath` はJSONファイル一個ではなく、完全なexportディレクトリを指定します。データ内容の検証とaddonの解釈はサーバーが行います。

実行ファイル・任意addonを起動するためWorkspace Trustが必要です。フォルダー設定でこれらのパスを渡す場合も、そのworkspaceを信頼したうえで利用してください。試験のためにTrustを無効化する必要はありません。

## Project roots / lifecycle

- 開いているworkspace folderをそのままrootにします。親の `.git` や別のprojectを探しません。
- 通常のmulti-rootは、Skript文書を開いた各folderにつき一つのサーバーを起動します。各接続へ渡すworkspace folderは一つです。resource設定でfolderごとにsnapshot・addonを選べます。
- workspace外の単独ファイルは親ディレクトリをrootにし、そのディレクトリのファイルだけをclientの文書対象にします。近隣ファイルのproject探索は既存サーバーの上限に従います。
- **入れ子のworkspace foldersは試作版の制限です。** 祖先folderの起動をエラーで止めます。workspaceから祖先または子folderを外してください。曖昧なprovider選択や重複したproject解析に頼らないための明示的な制限です。単独ファイルの親ディレクトリ同士が入れ子になる場合は利用できます。
- `file:` のSkript文書だけを扱います。未保存のUntitled文書には接続しません。

Command Paletteの `Skript LSP: Start / Stop / Restart / Show Output` で操作します。最後の対象文書が閉じられるとそのrootのclientを停止します。関連設定やworkspace foldersを変更すると接続を作り直します。明示的にStopした状態は設定変更後も維持します。起動エラーを直したらStart/Restartしてください。

`skriptLsp.trace.server` はwindow単位の設定です。Outputの「Skript LSP」でログレベルをTraceにするとprotocol tracingが有効になり、この設定でmessages / verboseの詳細度を選べます（SDKの仕様によりoffでもTraceレベルではmessages）。Trace以外ではprotocol tracingは無効です。verboseにはソース本文・ローカルパスが含まれるため、共有前に内容を確認してください。

## Colors / editor limits

本文のsemantic tokensは、サーバーが返すSkript独自のroleも交渉し、通常のtheme scopeへ対応付けます。色の選択は利用中のthemeに従います。

hoverや補完説明の `skript` / `skript-pattern` code fenceにはTextMate grammarを提供します。例文の文字列・変数・補間、およびpatternの `%types%` やgroup記号が字句単位で色分けされます。説明欄内でサーバーのASTやsemantic tokensを再解析するものではありません。VS CodeのMarkdown表示とthemeに依存し、Neovimの独自表示と同一の配色は保証しません。

補完はサーバーのprepared/final/revoked/project-changed通知を利用し、現在の文書・版・位置・ticketに一致する候補を更新します。取消・編集・終了後の結果は破棄します。空白でも補完を起動し、右側のsuffixを保つ編集ではfilterTextを入力範囲に合わせます。文字列内の補間はコードのtoken種別に戻します。構文名・Javaクラス名による例外は追加していません。候補がないprepared結果には現サーバーがticketを付けないため、その場合の独自通知更新はありません。既存サーバーの探索上限・解析費用は引き続き適用されます。大量ディレクトリで探索上限に達しても、開いた文書の解析を続けるのはサーバー側の契約です。

## Isolated integration tests

インストール済みのCode executableを指定して実機試験できます。自動downloadはありません。`Code.exe` / `Code - Insiders.exe` 本体を指定し、CLIの `.cmd` は指定しません。

```sh
npm run compile
node scripts/test-host.mjs /path/to/Code /path/to/skript-lsp /path/to/complete-ssg-export
```

毎回新しい一時profile・extensions directory・workspace・`.sk` fixtureを作り、Extension Development Hostで公開APIを試験します。既存profileやユーザーのファイルは変更しません。project探索上限の試験用に空のテキストファイルを50,001個作ります。成果物の一時ディレクトリは表示され、`result.json` とCodeのlogsを残します。試験完了後、必要な証跡を保管してからその一時ディレクトリを削除できます。Trust承認が必要な環境では、作成された試験workspaceだけを承認してください。

自動試験はprovider結果と診断を検証します。画面上の候補リストの操作感、Markdownの実際の配色、各themeでの見え方は別途目視してください。

## Remove / rollback

専用profileで拡張を無効化・uninstallするか、同じdata directoryで次を実行します。

```sh
code-insiders --user-data-dir /path/to/test-profile --extensions-dir /path/to/test-extensions --uninstall-extension nlaocs.skript-lsp
```

導入前のVSIXへ戻す場合は、対象profileへ前のVSIXを再installしてください。サーバー・snapshotは自動更新しないため、この拡張の削除では変更しません。

実装の基礎はMicrosoft公式の [Language Server Extension Guide](https://code.visualstudio.com/api/language-extensions/language-server-extension-guide)、[Extension tests](https://code.visualstudio.com/api/working-with-extensions/testing-extension)、[Semantic highlighting](https://code.visualstudio.com/api/language-extensions/semantic-highlight-guide)、[Workspace Trust](https://code.visualstudio.com/api/extension-guides/workspace-trust) です。

With `skriptLsp.trace.server` set to `messages` or `verbose`, the main output also records completion provider entry, dispatch attempts, response counts and ranges, phase refresh, waits and discarded results. They omit source text, candidate labels and filter strings; URI, version, positions, filter lengths and alignment flags remain. Dispatch means the SDK request callback was invoked, not proof of transport delivery. These metadata logs do not require the channel log level to be Trace. Activation logs identify the loaded extension version and path. They help separate API completion from automatic suggest display; an API response alone does not confirm a visible menu.

# Skript LSP for VS Code（試作版）

既存のnative `skript-lsp`へstdioで接続するclientです。`.sk`をSkriptとして登録し、診断、ホバー、
補完、引数案内、定義・参照、安全なリネーム、呼び出し階層、構造一覧、semantic tokens、inlay hintsを
標準LSP経由で提供します。解析はserverが担当し、拡張側に別のSkript parserはありません。

VS Code Stableの公開APIのみを使い、最低APIと型定義は1.100です。Insiders専用・proposed APIは使いません。
両channelを対象にしていますが、API互換性と各version/themeでの実表示の検証は別です。
Web版・virtual workspace・Untitled文書には対応しません。Remoteでは拡張hostのOS用の実行ファイルとデータが必要です。

## ビルドとパッケージ

Node.js 22以降とnpmを用意し、このdirectoryで実行します。

```sh
npm ci --ignore-scripts
npm run check
npm test
npm run package
```

`skript-lsp-0.1.2.vsix`を生成します。依存はlockfileで固定します。`ignore-scripts`を付けても、
明示した`npm run`は実行できます。server executable、SSGデータ、WASM addonは同梱・自動取得しません。
[repositoryのビルド手順](../../README.ja.md#ビルドと接続)でserverを用意し、対象環境の完全なSSG exportを指定してください。

## インストールと設定

対象のStableまたはInsidersのExtensionsから「Install from VSIX...」でVSIXを選びます。
CLIでは、導入先のchannelに合わせて`code`または`code-insiders`を使えます。

```sh
code-insiders --install-extension /path/to/skript-lsp-0.1.2.vsix
```

Settingsに次を指定します。pathは実際の配置へ変更してください。Windowsの実行ファイルは`.exe`で、
JSON内では`C:/...`またはescapeしたbackslashを使います。

```json
{
  "skriptLsp.serverPath": "/path/to/skript-lsp",
  "skriptLsp.snapshotPath": "/path/to/complete-ssg-export",
  "skriptLsp.addonPaths": []
}
```

絶対path、または選択したproject rootからの相対pathを受け取ります。PATH検索、shell展開、`~`、
`${workspaceFolder}`の展開はしません。`snapshotPath`はJSON file一個ではなくexport directoryです。
snapshotの内容とAddonの契約はserverが検証します。

native executableと任意Addonを起動するためWorkspace Trustが必要です。信頼できる対象folderだけを承認し、
Trustを無効にする必要はありません。関連設定を変えるとclientを再起動するため、稼働中の作業と区別して変更してください。

## Projectの範囲と接続寿命

- workspace folderをそのままrootにし、祖先の`.git`から別projectを選びません。
- 通常のmulti-rootでは、Skript文書を開いた各folderに一つのserverを起動します。各接続には一つのfolderを渡し、
  resource設定でfolderごとにsnapshot・Addonを選べます。
- **入れ子のworkspace foldersは未対応です。** 祖先folderの起動を明示errorにします。
  workspaceから祖先または子folderを外してください。単独fileの親directory同士が入れ子になる場合は利用できます。
- workspace外の単独fileは、その親directoryをrootにします。clientは直接そのdirectoryにあるfileを扱い、
  serverの近隣ソース探索には[LSPのproject上限](../../docs/lsp.ja.md#project内の文書と依存関係)が適用されます。
- 対象は`file:`のSkript文書です。最後の対象文書を閉じると、そのrootのclientを停止します。

Command Paletteの`Skript LSP: Start / Stop / Restart / Show Output`で操作します。
関連設定・workspace foldersの変更は接続を作り直しますが、明示的なStopは設定変更後も維持します。
起動エラーを修正したらStart/Restartしてください。snapshotを読み直す場合も再起動が必要です。

## 操作と対応範囲

editor標準のHover、Suggest、Signature Help、Go to Definition、Find All References、Rename Symbol、
Call Hierarchy、文書内symbol一覧を利用します。inlay hintsとsemantic highlightingの表示はeditor設定にも従います。

| 機能 | 注意点 |
| --- | --- |
| 補完 | 要求slotと候補の型が分かる範囲で不適合を除外します。Unknown/Dynamicと未確定の期待型は保守的に扱い、未検証の別文法continuationを適合済みとは表示しません。 |
| 変数の型 | 宣言型、要求型、現在の保存値の推論を区別します。関数結果の安全な代入経路も扱いますが、任意のruntime値や副作用を実行して確定するものではありません。 |
| リネーム | 静的なASCIIの単一localとソースのある関数に限定します。global/list/parameter/動的名は対象外。全編集先をopenにし、現在版で解析を完了させます。不完全な参照範囲や衝突は拒否します。 |
| 呼び出し階層 | ソースのある関数本文同士が対象です。Event/Command入口やCatalogだけの関数、未解決callを補作しません。 |
| 構造一覧 | providerがmetadataと安全な原文範囲を提供するStructure/Sectionを表示します。全構文をheaderの綴りから拾う機能ではありません。 |
| Inlay hints | 実引数名と対応する初期化済み暗黙引数を表示します。変数型のhintは未実装です。 |

正確な拒否条件、型flow、探索不完全時の制限は[LSPの仕様](../../docs/lsp.ja.md)を参照してください。
参照検索で読めるclosed fileでも、安全なリネームにはopenが必要です。編集はversion付きWorkspaceEditとして返されます。

## 補完の更新と色

拡張はserverのprepared/final/revoked/project-changed通知を使い、同じ文書・版・位置・ticketの候補を更新します。
取消・編集・終了後の結果を破棄し、空白からの起動と右側suffixを保つ編集にも対応します。
文字列の補間内はcodeのtoken種別に戻します。構文名・Java class名による例外は持ちません。
空のprepared結果には現serverがticketを付けないため、その場合の独自通知更新はありません。
refreshによるmenuのちらつきが残る場合があり、APIの応答だけで実際のmenu表示を確認したとは扱いません。

本文のsemantic tokensはSkript独自のroleも交渉し、themeのscopeへ対応付けます。初回は最終解析を使うため、
大きな文書では準備済み補完より遅れて色が付く場合があります。全文解析の並列化は導入していません。

ホバー・補完説明の`skript`/`skript-pattern` fenceにはTextMate grammarを提供します。例文の文字列・変数・補間、
登録patternの型slotやgroupを字句単位で色分けします。説明内のASTを再解析するものではなく、解析の受理を示しません。
Markdown rendererとthemeによって見え方は異なり、Neovimと同じ配色は保証しません。

## ログと検証

Outputの「Skript LSP」でログを確認できます。`skriptLsp.trace.server`はwindow単位の設定です。
channelのlog levelをTraceにするとprotocol tracingが有効になり、messages/verboseの詳細度を選べます
（SDKの仕様により、Traceではoffでもmessages）。Trace以外ではprotocol tracingは無効です。
verboseにはソース本文・local pathを含み得るため、共有前に確認してください。

`trace.server`がmessages/verboseの場合、Trace level以外でも補完providerの受付・dispatch・応答件数と範囲・
phase更新・待機・破棄を記録します。これらのmetadata logは本文・候補label・filter文字列を出力しませんが、
URI・版・位置・filter長・整列flagは残します。dispatchはSDK callbackを呼んだ意味で、通信配送の証明ではありません。
activation logは読み込んだ拡張のversionとpathを示します。

unit testに加え、インストール済みCode executableを指定した任意のintegration testがあります。
自動downloadはなく、`Code.exe`/`Code - Insiders.exe`本体を指定します（CLIの`.cmd`ではありません）。

```sh
npm run compile
node scripts/test-host.mjs /path/to/Code /path/to/skript-lsp /path/to/complete-ssg-export
```

この試験は一時profile・extensions directory・workspaceを作り、探索上限試験のため空fileを50,001個生成します。
通常のprofileや原本を変更しませんが、時間とdiskを使うため明示的に実行してください。表示された一時directoryに
`result.json`とCode logsを残します。Trust承認が必要なら作成された試験workspaceだけを承認してください。
API/provider/診断の確認と、物理画面のmenu・Markdown配色・themeの確認は別です。

## 削除と復元

導入先のeditorから拡張を無効化・uninstallできます。CLIの場合も同じchannel/profileを選びます。

```sh
code-insiders --uninstall-extension nlaocs.skript-lsp
```

以前のVSIXへ戻すには、その導入先へ旧VSIXを再installします。server・snapshot・外部Addonは自動更新しないため、
拡張の削除では変更しません。serverを差し替える場合は旧exeと対応するAddon・設定を保存し、整合する組み合わせへ戻せるようにします。

実装の基礎はMicrosoft公式の[Language Server Extension Guide](https://code.visualstudio.com/api/language-extensions/language-server-extension-guide)、
[Extension tests](https://code.visualstudio.com/api/working-with-extensions/testing-extension)、
[Semantic highlighting](https://code.visualstudio.com/api/language-extensions/semantic-highlight-guide)、
[Workspace Trust](https://code.visualstudio.com/api/extension-guides/workspace-trust)です。

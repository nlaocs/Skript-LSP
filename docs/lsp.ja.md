# 最小 stdio Language Server

`skript-lsp --stdio` は、設定した一つのprojectとSSG snapshotを使い、開いている文書の診断を更新する。
必須CoreLibraryはexeへ埋め込まれ、通常の解析にJavaやMinecraftサーバーは必要ない。
全構文・全Addonの互換性を保証する完成版ではなく、補完・定義移動・複数ファイル索引・変数型flow・incremental parseは含まない。

## 初期化と対応メソッド

clientは `rootUri` または一件の `workspaceFolders` と、次の `initializationOptions` を渡す。
複数workspace、snapshot指定漏れ、読めないsnapshot、不正なAddonは初期化エラーとして返す。
相対pathはproject rootから解決する。読み込み後のsnapshot変更にはserverの再起動が必要。

```json
{
  "snapshotPath": "C:/path/to/complete-ssg-export",
  "addonPaths": []
}
```

対応する要求は `initialize`、`shutdown`、通知は `initialized`、`exit`、
`textDocument/didOpen`、`didChange`、`didClose`。
同期方式はFullで、変更は文書全体の `text` と増加するclient versionを送る。
位置encodingはUTF-16。診断はversion付きの `publishDiagnostics` で置換し、
修正後に診断がなくなった場合とclose時には空配列を返す。
close時のversionは、そのopenで最後に受理したversion。
空文書も解析対象になる。未知の要求にはmethod-not-foundを返す。
shutdown後は解析・診断公開を止め、exitで終了する。shutdown前のexitは異常終了として扱う。

## 解析と寿命

32 MiB stackのworker一つがParserHostを所有し、既存 `parse_document` を呼ぶ。
通信処理は解析の完了を待たない。待機中の編集は文書ごとに最新一件へまとめ、
新しい編集・close・shutdownで旧解析へ取消を要求する。
内部revisionはserverの寿命中に再利用せず、client versionとは別に採番する。
host内のdocument IDはURIのSHA-256から作る短いopaque IDであり、percent encodingで長くなったURIもhostのID上限へ収める。
clientへの診断URIと原文の位置は変更しない。Addonへ渡すdocument IDはファイルpathとして解釈しない。
公開直前にURI・open世代・revision・client versionを照合し、旧結果を破棄する。

取消は協調的である。実行中のregex/WASMやstage境界までの処理を即時に停止する保証はない。
終了処理もworkerの取消処理と後始末が返るまで待つ。既にcommit済みの解析を、
後から届いた編集通知で取り消す機構ではない。通信上の旧結果の破棄とtransactionのrollbackは別の保証である。

closeではbuffer/jobの参照を外し、worker上で解析終了後にDocument scopeと動的構文layer・revision記録を解放する。
StateStoreのlifetime identityにより、解放前のtransactionは同じ数値revisionで再openしてもcommitできない。
遅延したcloseが新しい内部revisionを解放することも防ぐ。
Project / PersistentProject scopeとAddon instanceは同じproject内で維持する。
呼び出し側が保持する古いtransactionやimmutable snapshotは、所有参照をdropするまで生存する。
StateStoreをLSPのsymbol databaseとして利用する設計ではない。

## 診断の意味と位置

RawTree、Structure/Section、hook、回復解析のprimary/related failure、extension faultを診断へ変換する。
fatal errorを空診断へ変えず、取消・staleの解析結果は公開しない。
`data.executionStatus` の `complete` は拡張障害のない実行であり、全文法が正しいという意味ではない。
`partial` の領域は未確定で、得られた他の診断とともに残す。

同じ子要素に具体的なerrorのfailure traceがある場合、その子のStructure/Sectionのunclaimed要約は公開しない。
対応付けにはRawNodeIdを使い、同じ行や重なる位置という理由だけでは別要素の診断を消さない。
複数のcaptureや子要素の具体診断は残す。原因なし・warningのみの場合、トップレベルStructureのroot/header自体の失敗、
対応する子要素を特定できない要約は残す。有効な `on load` の子が失敗した場合にheaderへ位置を移すこともない。

位置はvirtual byte offsetを直接表示せず、SourceMapのoriginal originから原文の行・UTF-16列へ変換する。
primaryはparserが返した最初のorigin、残りのoriginとrelated spanはrelated informationへ残す。
optionsによる生成文もこの規則でcall-siteへ対応付ける。元位置が不正ならsource-mapping failureを明示し、仮のvirtual位置へ置き換えない。
単独Function呼び出し文等、現parserの未対応部分はそのまま制約として残る。

## 計測と検証

`window/logMessage` にstartupと解析単位のJSONを出力する。
`startupMs` はsnapshot読み込み・WASM host初期化を含むworker準備時間。
`queueMs` は要求の待機、`parseMs` は `parse_document`、`reportMs` は診断変換と結果の後始末、
`analysisMs` はworker処理全体、`cancelToFinishMs` は取消要求からworkerが返るまでの時間を表す。
取消前に終了していた場合や取消されなかった場合はnullになる。
これらはRustのbuild/compile時間や、clientが画面へ診断を描くまでの時間を含まない。
最新版へまとめられて実行されなかった要求の解析時間は計上しない。

```sh
cargo run -p xtask --locked -- build-core-library
cargo run -p xtask --locked -- build-test-components
cargo test -p skript-lsp --all-features --locked --lib --test lsp_stdio
cargo test -p parser-wasm -p syntaxes --all-features --locked --lib
```

決定的な制御worker試験では、解析を止めたまま通信応答、連続編集の集約、旧完了の破棄、
close/versionリセット/reopen、cancel/stale/fatal/partialの区別を確認する。
stdio試験は実CoreLibraryとsnapshot、試験Addonを使い、初期化/終了、原文位置、options、faultを確認する。
[Neovim手順](neovim.ja.md)には独立設定と期限付きheadless確認を用意している。

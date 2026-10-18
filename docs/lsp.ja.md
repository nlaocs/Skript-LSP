# 最小 stdio Language Server

`skript-lsp --stdio` は、設定した一つのprojectとSSG snapshotを使い、開いている文書の診断・ホバー・Semantic Tokensを提供する。
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

対応する要求は `initialize`、`textDocument/hover`、交渉済みの `textDocument/semanticTokens/full`、`shutdown`、
通知は `initialized`、`exit`、`textDocument/didOpen`、`didChange`、`didClose`、`$/cancelRequest`。
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

## ホバーの情報と寿命

`hoverProvider: true` を公開し、現在の文書の最終ASTで採用された要素から説明と原文のUTF-16範囲を返す。
`textDocument.hover.contentFormat` の先頭から対応するMarkdownまたはPlainTextを選ぶ。
指定がない場合はPlainTextを使う。
重なる要素では狭い範囲を優先し、同範囲なら深い子要素を選ぶ。同順位で異なる要素が残る場合はnullを返す。
空白・コメント・未解析の位置・拡張障害の領域・空範囲にはホバーを返さない。
Exact originだけを使い、Replaced/Anchored originの位置は親要素へのfallbackも含めて対象外にする。
マクロ展開内の異なる要素が同じ原文位置に潰れる場合に、確定した意味を推測しないためである。

構文はdefinition ID・registration ID・種類を照合してSSGの名前・採用pattern・Addon・説明と例を表示する。
定義元JavaクラスはSSGの `elementClass` を使い、短いクラス名と完全修飾名を併記する。
情報を持たない関数や型について、戻り値のJava表現から実装クラスを推測しない。
説明のHTMLは強調・段落・箇条書き・コード・安全な外部リンクをMarkdownまたはPlainTextへ変換する。
HTMLを識別した後で文字参照を一段だけ復号し、Skriptの `<red>` 等をコードから削除しない。
通常文中のplaceholder・パス・Markdown記号を含む断片はinline codeで保護する。
Neovimでバックスラッシュのescapeが露出するのを避け、原文にあるバックスラッシュ自体は保持する。
相対リンクやSSG内の参照anchorは表示ラベルを残す。URLの基点は推測しない。
Examples見出しは一つにし、単一行itemのうちsection開始行より深い字下げの継続行だけを結合する。
元から複数行のitem、同じ深さのitem、独立した例の境界と字下げを保持する。
Markdownのコード区切りは内容中のbacktickより長くする。処理入力と出力には上限を設け、
上限に達した場合は例全体を省略するか本文内に省略記号を付け、コード区切りを途中で切らない。
型名はcatalogのcode nameへ変換し、戻り値型・個数・候補型のcomplete/partial/unresolvedを区別する。
関数のsignatureは解決済みIDに一致する文書内宣言、またはSSGの関数定義を使う。
変数名とlocal/global scopeは `nlaocs.skript.variable` version 1のpublic dataだけを解釈する。
動的な名前の式は評価せずtemplateとして表示する。変数の型は解析文脈が要求した型であり、値や代入履歴から推論した型ではない。
未知schemaでは名前の綴りからscopeや値を推測しない。

ホバー用の不変データは解析workerで作る。protocol処理は再解析を行わず、解析待ちにはnullを返す。
受理した編集で直ちに旧キャッシュを捨て、世代・revision・client versionが一致した完了結果だけを保持する。
closeではキャッシュを解放し、同じURIをversion 0で開き直しても旧情報を引き継がない。

## Semantic Tokens の分類と範囲

clientがrelative形式とfull要求への対応を表明した場合に、対応するtoken typeの共通部分だけをlegendとして返す。
fullのみを実装し、range・delta・modifierは公開しない。未対応の独自分類を標準の別の意味へ変換せず省略する。

| token type | 対象 |
| --- | --- |
| `skriptEvent` / `skriptCondition` / `skriptEffect` | 採用された各構文の確定した固定部分 |
| `skriptExpression` / `skriptSection` / `skriptStructure` | 同じくExpression・Section・Structureの固定部分 |
| `skriptLiteral` | 最終結果がliteralと示す値。Type parserが生成した値も含む |
| 標準 `function` | 型付きFunction callの名前だけ |
| 標準 `variable` | version 1の公開schemaで確認できる変数参照の名前部分 |

実効型から変数の固定型やparameter bindingを推測せず、引数値を一律にparameter色にしない。
子の範囲・位置付きcaptureを親から引くため、文字列や変数名の補間内にも子自身の分類を適用する。
未対応の子も親から引き、親色へのfallbackを避ける。各tokenは単一行・非空・UTF-16の半開区間で、重なりを返さない。

最終ASTと原文が一致するExact originだけを使う。コメント・Invalid行・fault領域・非Exactの原文範囲は除外する。
別nodeの原文範囲が競合する部分は順番で選ばず省略する。Function名は単一Exact originと原文一致を確認する。
未完成文書でも独立して確定した兄弟は着色できる。

最終ASTでは一部のcapture位置が失われるため、Text captureを持つ親、CatalogのID・patternと照合できない親、
regexを含むpatternの親を初版では着色しない。EventではTypeExprを含むpatternの親も省略する。
`if` / `while`等の固定語や宣言header、動的構文の親もこの制約に該当し得る。確定した子の着色は残る。
default/group/listのwrapper、型名の宣言位置、引数label等の位置や意味を補作しない。

token用の不変データはworkerで作り、通信threadで再解析しない。
解析中のfull要求は通信処理を止めずに保持し、同じ世代・revisionの解析完了時に返す。
編集・closeは旧要求へContentModified、shutdownと個別取消はRequestCancelledを返す。
個別要求の取消は共有の解析を止めない。fatal・cancel・staleの完了時には空結果で保留を解放し、worker故障時もerrorで終端する。
semantic投影の失敗は専用logと空tokenにし、既存の診断とホバーの生成から分離する。

## 計測と検証

`window/logMessage` にstartupと解析単位のJSONを出力する。
`startupMs` はsnapshot読み込み・WASM host初期化を含むworker準備時間。
`queueMs` は要求の待機、`parseMs` は `parse_document`、`reportMs` は診断・ホバー・token変換と結果の後始末、
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

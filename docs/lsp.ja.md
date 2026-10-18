# 最小 stdio Language Server

`skript-lsp --stdio` は、設定した一つのprojectとSSG snapshotを使い、開いている文書の診断・ホバー・Semantic Tokens・入力候補を提供する。
必須CoreLibraryはexeへ埋め込まれ、通常の解析にJavaやMinecraftサーバーは必要ない。
全構文・全Addonの互換性を保証する完成版ではなく、定義移動・複数ファイル索引・変数型flow・incremental parseは含まない。

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

対応する要求は `initialize`、`textDocument/hover`、`textDocument/completion`、交渉済みの `textDocument/semanticTokens/full`、`shutdown`、
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
| `skriptInterpolationDelimiter` | parserが位置を確定した文字列・変数名補間の `%` |
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

native parserが採用したregex/TypeExpr captureのMappedSpanを、既存WITのSpan captureとして最終ASTへ保持する。
host所有のsealをnodeの採用identityと位置集合に結び付け、macroによるcaptureの削除・縮小・移動後は根拠として使わない。
全capture範囲を子より狭く解釈せず差し引くため、`if 1 is 1:` は固定`if`がSection、`is`がConditionになる。
Text captureは対応する元位置と原文一致も確認し、文字検索で位置を補わない。
sealのないregex親、根拠が変化した親、CatalogのID・patternと照合できない動的構文の親等は省略し、独立した子を残す。
regex capture全体に含まれる未分類の宣言名等を親の固定語として塗らない。
default/group/listのwrapper、型名の宣言位置、引数label等の位置や意味を補作しない。

補間delimiterはCoreLibraryの公開schema `nlaocs.skript.interpolation` version 1が示す、親source内のUTF-8 byte位置を使う。
単一Exact origin、原文の`%`、childIndexと子の包含を確認し、空白をtrimした子の隣接文字からは推測しない。
位置が確定した補間slotは全体を親から引き、macro等で子が狭くなっても残りを親色へ戻さない。
escaped `%%` はdelimiterではない。対応を表明しないclientへ別分類の色で代用しない。
補間schemaを利用できない場合は専用delimiter分類を省略し、独立に確定した親分類と子の範囲を維持する。
現parserが拒否する`{_key% sin(1) %}`等の入力は、診断と無色の範囲を保ち、着色のために受理した扱いへ変えない。

token用の不変データはworkerで作り、通信threadで再解析しない。
解析中のfull要求は通信処理を止めずに保持し、同じ世代・revisionの解析完了時に返す。
編集・closeは旧要求へContentModified、shutdownと個別取消はRequestCancelledを返す。
個別要求の取消は共有の解析を止めない。fatal・cancel・staleの完了時には空結果で保留を解放し、worker故障時もerrorで終端する。
semantic投影の失敗は専用logと空tokenにし、既存の診断とホバーの生成から分離する。

## 入力候補の範囲

`completionProvider` は `{` をtrigger characterとして公開する。任意の位置から手動要求でき、resolve要求は使わない。
起動時のCatalogからPattern ASTを読み、行頭ではStructure/Event、字下げされた文頭ではEffect/Section/Condition、
式の入力位置ではExpressionと戻り値のある関数を候補にする。固定語の続きを入力中は文頭候補も維持する。
SSGに記録されたEventの`on` prefixを使い、headerには`:`を付ける。
文中では同じPattern ASTを有限グラフとしてたどり、入力済みの引数の次に来る固定語も提示する。
例えば`teleport player t`には`to`を提示し、`player`や既存の右suffixを挿入し直さない。
型slotとregexは引用符・括弧・braceが閉じた原文の境界として扱う。引数の型や意味を確定した証拠にはしない。
継続語は一致した固定文字が多いもの、消費したslotが少ないもの、短い固定語、綴り順、Catalog登録順で並べる。
`t`に対する`to`を`towards`より先にする短さの順位は編集の便宜であり、parserの採用順位やAddonの優先権を表さない。
同じ継続語は一つにまとめ、説明には最上位の代表patternの全文法を表示する。
継続候補が残れば通常の代替候補を最大8件に絞り、その後に全体を最大100件にする。
それ以外は文頭固定語、変数履歴、文書関数、Catalog関数、Expressionの順で、各Catalog群内では登録・pattern順を保つ。
pattern sourceは1024 byteまでとし、文頭テンプレートの選択肢をpatternごと最大16通り、深さ32に制限する。同じ登録・先頭固定語の重複をまとめる。
継続語は選択肢の直積を作らず、graphを最大2048 node、探索をpatternごと4096状態・slotごと128境界、
要求全体で100万操作（状態・辺・文字走査）、入力prefixを4096 byteまでに制限する。
切り詰めがあれば`isIncomplete`を返す。各候補のdocumentationには省略しない元patternまたは関数signatureを含める。

これらは入力支援候補であり、採用済みの構文ではない。不完全な行へのexpected typeやAddon guardの判定、
実行scopeへの適合性は確認しない。先頭がplaceholderだけのpattern、動的Addon layer、regexの具体的なマッチ文字列は補作しない。
関数は現在文書の既存registryとCatalogから取り、parserと同じ引数のcomponent type・single・optional変換を使う。
同名・同shapeの文書内関数によるshadowを維持し、戻り値のないshadowからCatalogの式候補を復活させない。
引数型に合うoverloadの選択は行わない。単独Function呼び出し文のparser制約も変わらない。

静的変数名は最終ASTのExpression、公開variable schema、子なし、原文一致のExact originを確認した参照履歴から得る。
localはカーソルより前に同じRawTree bodyまたはその祖先bodyで観測した参照だけを示し、兄弟bodyへ持ち出さない。
これは宣言・binding・型・値の解析ではない。動的名、未知schema、位置が曖昧な生成文、fault領域は履歴へ加えない。
文字列本文・コメントには候補を出さず、補間内では入力支援用の字句状態を使う。字句判定自体はparserの受理を保証しない。

編集は原文のUTF-16範囲で返す。既存の関数引数・変数の閉じbrace・右側の固定語を保持する。
カーソル右側の既存文字を置換する必要があるときは、その文字列が候補末尾と一致する場合だけ前方prefixの編集へ変換する。
末尾が一致しない途中置換は候補から省略する。右側の文字をmenu確定時に置換しないclientでも二重にしないためである。
`snippetSupport`があれば型slotや必須関数引数をtab stopにし、なければ最初のslot直前までの固定語だけを入れる。
解析済みのrevisionではworkerで作った不変データを使い、追加の型推論やWASM解析を要求ごとに行わない。
編集中に構文候補を返せる場合は、現在の原文だけからnative RawTreeを作り、全文WASM解析を待たずに`isIncomplete=true`で返す。
このcontextはrevisionごとに再利用し、256KiBを超える原文では即時経路を使わない。参照scope用の祖先列は即時経路では作らない。
即時応答は文頭構文と継続語を対象とし、その要求には未解析の関数・変数を後送しない。次の補完要求で現在の解析済み候補を得られる。
構文候補を返せない場合は解析完了まで要求を保持する。旧revisionの履歴や文書関数を流用しない。
編集・closeは旧要求へContentModified、個別取消・shutdownはRequestCancelledを返す。
取消・stale・fatalは空結果、worker failureはerrorで終端し、旧revisionの候補を返さない。

## 計測と検証

`window/logMessage` にstartupと解析単位のJSONを出力する。
`startupMs` はsnapshot読み込み・WASM host初期化・Catalog入力候補索引を含むworker準備時間。
`queueMs` は要求の待機、`parseMs` は `parse_document`、`reportMs` は診断・ホバー・token・補完view変換と結果の後始末、
`analysisMs` はworker処理全体、`cancelToFinishMs` は取消要求からworkerが返るまでの時間を表す。
取消前に終了していた場合や取消されなかった場合はnullになる。
これらはRustのbuild/compile時間や、clientが画面へ診断を描くまでの時間を含まない。
最新版へまとめられて実行されなかった要求の解析時間は計上しない。
`skript-lsp completion timing`の`source`は`syntax`または`analysis`を示す。
`queryMs`は候補検索と応答構築で、即時経路では必要なnative context作成も含む。
`waitMs`は受信後から解析済み候補検索を始めるまでの待機で、失敗した即時試行も含む。即時応答では0になる。
通信・client scheduling・menu描画とRust build/compileはこれらに含まれず、それぞれ別に計測する。

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

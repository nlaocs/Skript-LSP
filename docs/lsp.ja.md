# stdio Language Server

`skript-lsp --stdio`は、一つのprojectとSSG snapshotで診断・ホバー・Semantic Tokens・入力候補・引数案内・
定義移動・参照検索・暗黙引数のinlay hintを提供する。採用済みの根拠から保守的な変数型flowも得る。
必須CoreLibraryはexeへ埋め込まれ、通常の解析にJavaやMinecraftサーバーは必要ない。
全構文・全Addonの互換性、runtime値の評価、完全な手続き間解析を保証する完成版ではない。

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

対応要求は`initialize`、`textDocument/hover`、`textDocument/completion`、`textDocument/signatureHelp`、
`textDocument/definition`、`textDocument/references`、`textDocument/inlayHint`、交渉済みの
`textDocument/semanticTokens/full`、`shutdown`。通知は`initialized`、`exit`、`textDocument/didOpen`、
`didChange`、`didClose`、`workspace/didChangeWatchedFiles`、`$/cancelRequest`に対応する。
同期方式はFullで、変更は文書全体の `text` と増加するclient versionを送る。
位置encodingはUTF-16。診断はversion付きの `publishDiagnostics` で置換し、
修正後に診断がなくなった場合とclose時には空配列を返す。
close時のversionは、そのopenで最後に受理したversion。
空文書も解析対象になる。未知の要求にはmethod-not-foundを返す。
shutdown後は解析・診断公開を止め、exitで終了する。shutdown前のexitは異常終了として扱う。

## 解析と寿命

32 MiB stackのworker一つがParserHostを所有し、既存 `parse_document` を呼ぶ。
通信処理は解析の完了を待たない。待機中の編集は文書ごとに最新一件へまとめ、
最新入力を優先しながら古い別文書を無期限に飢餓させない。新しい編集・close・shutdownで旧解析へ取消を要求する。
内部revisionはserverの寿命中に再利用せず、client versionとは別に採番する。
host内のdocument IDはURIのSHA-256から作る短いopaque IDであり、percent encodingで長くなったURIもhostのID上限へ収める。
clientへの診断URIと原文の位置は変更しない。Addonへ渡すdocument IDはファイルpathとして解釈しない。
公開直前にURI・open世代・revision・client versionを照合し、旧結果を破棄する。

取消はstage間・native matching内・commit前で確認する。実行中のregexには既存quotaが残る。
WASMをepoch checkで途中停止するには、componentが`parser.epoch-cancellation-restart-safe` v1を明示し、
中断後にguest Store全体を破棄し、同じ不変HostProfileから決定的に再初期化しても将来の意味を保つと約束する必要がある。
初期化中のStateStore/dynamic呼出は試行を含めて拒否し、不変Catalog照会だけを許す。host側の状態は保持する。
次の呼出前にmanifest全項目とhandler bindingを照合し、復帰故障なら明示的に無効化する。中断した解析は再試行しない。
未宣言componentはfuel・通常deadlineを維持し、上限内の正常return直後に取消を確認する。
既存host方針ではCore IDの通常wall-clock deadlineを`u64::MAX / 2` ticksへ緩める例外が残る。
既定の呼出しfuel上限20Mは残るが、Coreに短いwall-clock取消時間を保証しない。
一律の即時停止や一定ms以内を保証しない。終了処理もworkerの取消処理と後始末を待つ。既にcommit済みの解析を、
後から届いた編集通知で取り消す機構ではない。通信上の旧結果の破棄とtransactionのrollbackは別の保証である。

closeではbuffer/jobの参照を外し、worker上で解析終了後にDocument scopeと動的構文layer・revision記録を解放する。
StateStoreのlifetime identityにより、解放前のtransactionは同じ数値revisionで再openしてもcommitできない。
遅延したcloseが新しい内部revisionを解放することも防ぐ。
Project / PersistentProject scopeとAddon instanceは同じproject内で維持する。
呼び出し側が保持する古いtransactionやimmutable snapshotは、所有参照をdropするまで生存する。
StateStoreをLSPのsymbol databaseとして利用する設計ではない。

## Project内の文書と依存関係

単一root内のUTF-8 `.sk`を読み取り、未openの関数宣言・参照も現在のproject snapshotへ含める。
open bufferは同じpathのdisk内容より優先し、未保存の内容を勝手に書き戻さない。
root外へのpath、symlink/reparse point、重複したopen aliasを安全側で扱い、`.git`を走査しない。
上限は512 source、1 fileあたり1 MiB、合計16 MiB、directory entry 50,000件、深さ64である。
読めないfile、不正UTF-8、予算超過、解析中のclosed file変更は明示errorにする。

複数文書を解析する場合、有効なすべてのcomponentが`parser.project-stable-semantics` v1を
明示する必要がある。未宣言componentがあればproject解析をerrorにし、古い関数をそのまま採用しない。
この契約は初期化済みの不変入力と現在の完全な文書pipelineが意味を決めるという約束であり、
Project/Document等の永続State、I/O、時刻、乱数、隠れたguest履歴、処理順や内部revisionに
後続文書の意味を依存させない。新しい未知のversionをv1と解釈しない。

native registryの採用済み宣言、成功・失敗した関数lookup、原文hash、source generationを用いる。
削除された定義や以前の未解決lookupも再評価し、signatureが一致するだけの古い移動先は返さない。
安定しない宣言集合は上限付き固定点解析でerrorにする。snapshotの更新は再起動が必要である。
clientがdynamic watcher登録に対応する場合、`**/*.sk`のcreate/change/deleteを登録し、
`workspace/didChangeWatchedFiles`でproject世代を更新する。未対応・登録失敗時のclosed file更新は、
次のopen文書の変更または同通知まで反映されない。現在世代を初めて受理した時点で、対応clientへ
標準`workspace/inlayHint/refresh`と`workspace/semanticTokens/refresh`をまとめて要求する。
まだ新しい解析が完了していない文書の問い合わせは、その文書の完了を待つ。

## Incremental処理とAddon契約

公開WITは`nlaocs:skript-parser-addon@0.41.0`、ABIは`23.0`である。旧ABIのcomponentは再buildする。
`parser.incremental-local-syntax` v1とmanifestの`incremental-local-subscriptions`へ
明示したhookだけが再利用候補になる。現在payload/context、物理source行、不変Catalog/runtime/binding、
観測したStateのget/scan値が返却値を完全に決め、guest履歴や未観測の入力へ依存しないという契約である。
hostは入力と依存read certificateを照合し、data-onlyの安全な返却値だけを再利用する。
State変更・変更試行、diagnostic、context変更、AdditionalParse、opaque attachment、登録副作用は再利用しない。
cancel/faultで終わった処理から新しいcache entryを公開しない。

native matching、macro、Structure/Sectionのowner選択、関数lookup、現在のtransaction検証は実行する。
以前のAST・State差分・source IDを移植する仕組みではなく、project parse要求数もゼロにはしない。
projectの安定性契約、個々のhookの再利用契約、epoch取消の安全性契約は独立している。

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

typed captureを拒否したnativeの`parser.type-mismatch`は、証明できたexpected/actual classを
structured dataへ残し、単なるtrailing-inputで隠さない。unionは既知の全候補を扱い、nested childの
不一致は外側の要求型へ置き換えない。未知のalternativeや変換可能な候補がある場合は断定しない。
別に`semantic.variable-type-mismatch` warningは、parserが受理した変数参照について現在到達する
既知型がすべて要求型へ変換できない場合だけ返し、`data.parserAccepted: true`を付ける。
native errorと同じ受理判定ではなく、Unknown・未設定の可能性・不確かな位置では出さない。

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
採用Patternは`skript-pattern`、Examplesは`skript`のMarkdown fenceに分ける。pattern grammarを
実行Skriptとして扱わない。Markdownのコード区切りは内容中のbacktickより長くする。処理入力と出力には上限を設け、
上限に達した場合は例全体を省略するか本文内に省略記号を付け、コード区切りを途中で切らない。
型名はcatalogのcode nameへ変換し、戻り値型・個数・候補型のcomplete/partial/unresolvedを区別する。
関数のsignatureは解決済みIDに一致する現在の文書・project宣言、またはSSGの関数定義を使う。
変数名とlocal/global scopeは `nlaocs.skript.variable` version 1のpublic dataだけを解釈する。
動的な名前の式は評価せずtemplateとして表示する。nativeの要求型表示とは別に、現在の代入履歴から
証明できた型を`Inferred type`として追加する。根拠不足はUnknownとし、要求型を実値の型とは扱わない。
未知schemaでは名前の綴りからscopeや値を推測しない。

ホバー用の不変データは解析workerで作る。protocol処理は再解析を行わず、解析待ちにはnullを返す。
受理した編集で直ちに旧キャッシュを捨て、世代・revision・client versionが一致した完了結果だけを保持する。
closeではキャッシュを解放し、同じURIをversion 0で開き直しても旧情報を引き継がない。

## 変数型flow・定義移動・参照検索

`textDocument/definition`と`textDocument/references`は、現在の採用済みnative AST、provider所有の
公開変数identity・変更・control-flow契約、Exactな原文範囲から作る不変viewを使う。
文字列検索で同名を宣言と決めたり、StateStoreをsymbol databaseとして使ったりしない。
関数名はnativeのdefinition/registrationとsource witnessを照合し、local関数は文書外へ持ち出さない。
未open fileの現在の関数宣言にも移動できる。参照要求の`includeDeclaration`を尊重する。

汎用engineはroot/実行文脈を分け、根拠のある代入・parameter・delete・list操作と到達書込を表現する。
型flowは、要求型への変換前に変数へ保存された型を扱います。汎用engineはunion・list要素・
parameter・到達書込を表現できますが、精度には現在のproviderの根拠が必要です。現在のCoreが
証明するのはprimitive scalarの生成と、安全な継承`beforeChange` clone/set経路です。任意の
FunctionやExpressionList結果はUnknownになり得て、型付きparameter/returnにも未設定の可能性を残します。
戻り値classだけでは非空性や他localの不変性を証明しません。nativeの評価順付きearly operand契約は
opaque処理より前の安全な読取を示すもので、Effect/Condition全体のpure宣言ではありません。
未知のconverter・callback・alias・control flowでは、外側の要求型を借りず事実を忘れます。

branchは実行されない経路も含めてunionし、未設定の可能性を保持する。loopや意味を公開していない
control flow、未知の副作用やfunction call、動的名・aliasが事実を壊す場合は保守的に忘れる。
global変数の実行時の値や別eventの実行順を推測せず、手続き間の戻り値・副作用解析は行わない。
公開identityのcase policyが不明なら綴りを勝手に正規化しない。型が証明できない場合は
`unknown (runtime value is not established here)`を表示する。Unknownを要求型へ置き換えない。
古いschema 7 snapshotも引き続き読み込めます。cloner/`beforeChange`の確かな情報がない場合、
影響する代入や読取の事実はUnknownのまま保持し、型宣言だけから保証を補いません。

定義移動は現在の到達書込を返す。証明できない場合の静的候補やglobalの書込が複数あれば複数位置に
なり得るため、一意なruntime bindingを保証しない。動的identityや曖昧なmacro originは省略する。
解析中の要求は文書64件・server全体256件まで保持し、超過分はRequestFailedにする。
UTF-16 surrogate途中・逆転rangeは待機前にInvalidParams、閉じたURIは空配列を返す。
編集・project更新・closeは旧要求へContentModified、個別取消とshutdownはRequestCancelledを返す。
要求の取消は共有解析を止めず、旧世代のannotationを後から返さない。

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
関数は現在のnative registryで採用した文書・projectの定義とCatalogから取り、parserと同じ引数のcomponent type・single・optional変換を使う。
同名・同shapeの文書内関数によるshadowを維持し、戻り値のないshadowからCatalogの式候補を復活させない。
引数型に合うoverloadの選択は行わない。単独Function呼び出し文のparser制約も変わらない。

静的変数名は最終ASTのExpression、公開variable schema、子なし、原文一致のExact originを確認した参照履歴から得る。
localはカーソルより前に同じRawTree bodyまたはその祖先bodyで観測した参照だけを示し、兄弟bodyへ持ち出さない。
参照履歴候補そのものは宣言やruntime値を保証しない。解析済み変数には独立した現在のflow推論を説明へ添えられる。
動的名、未知schema、位置が曖昧な生成文、fault領域は履歴へ加えない。
文字列本文・コメントには候補を出さず、補間内では入力支援用の字句状態を使う。字句判定自体はparserの受理を保証しない。

completionの説明形式は`completionItem.documentationFormat`の対応する先頭値を選び、hoverの交渉と独立にする。
省略時はPlainTextである。Markdownではpatternとtemplateを`skript-pattern`、実コード例を`skript`として区別する。
説明を着色しても、候補label・literalな挿入文字列・snippet・source edit・登録identityは変えない。

編集は原文のUTF-16範囲で返す。既存の関数引数・変数の閉じbrace・右側の固定語を保持する。
カーソル右側の既存文字を置換する必要があるときは、その文字列が候補末尾と一致する場合だけ前方prefixの編集へ変換する。
末尾が一致しない途中置換は候補から省略する。右側の文字をmenu確定時に置換しないclientでも二重にしないためである。
既存Function callには`data["nlaocs.lsp.completion-edit"]` version 1で`edit: TextEdit`も付ける。
これは名前・空白・既存の開き括弧までを保持して置換する任意のclient adapter用編集であり、通常のprefix編集を置き換えない。
対応adapterはUTF-16の編集範囲をclient標準処理へ渡す。未対応clientはdataを無視でき、Function kindと既存引数を維持する。
`snippetSupport`があれば型slotや必須関数引数をtab stopにし、なければ最初のslot直前までの固定語だけを入れる。
解析済みのrevisionではworkerで作った不変データを使い、追加の型推論やWASM解析を要求ごとに行わない。
編集中に構文候補を返せる場合は、現在の原文だけからnative RawTreeを作り、全文WASM解析を待たずに`isIncomplete=true`で返す。
このcontextはrevisionごとに再利用し、256KiBを超える原文では即時経路を使わない。参照scope用の祖先列は即時経路では作らない。
字句optionsはHostがexplicit runtime設定とenabled plugin情報を統合した結果を共有し、補完側で版を再推定しない。
不明・不正な版は不明のまま保持し、共通の`RawTreeOptions::UNKNOWN_RUNTIME`で`###`間を保守的にコメント扱いする。
これは潜在的なコメント本文をコードとして補完・解析しないための方針であり、Skript 2.9等の版や全featureの対応を仮定するものではない。
既知の版には実際の版境界を使い、document呼出側が明示した`raw_tree` optionsは引き続き優先する。
即時応答は文頭構文と継続語を対象とし、その要求には未解析の関数・変数を後送しない。次の補完要求で現在の解析済み候補を得られる。
文字列本文とコメントで候補が無いと確定できる場合も、解析完了を待たずに空の完了応答を返す。
それ以外で構文候補を返せない場合は解析完了まで要求を保持する。旧revisionの履歴や文書関数を流用しない。
編集・closeは旧要求へContentModified、個別取消・shutdownはRequestCancelledを返す。
取消・stale・fatalは空結果、worker failureはerrorで終端し、旧revisionの候補を返さない。

### 本文解析を待たない構造内の仮候補

clientが `capabilities.experimental.skriptCompletionRefresh = true` を送る場合、serverは同じcapabilityを返し、同じ全文解析のText/Tree macroと全rootのEnterBodyが終わった段階を補完に公開する。
別のparse、旧revisionのAST、型・変数・文書関数の履歴は使わない。SSGと実際のowner hookのbody mode、凍結したEntryValidatorを使い、native parserと同じ宣言index選択・重複判定を共有する。
候補は成功した意味解析の証明ではない。各itemのdetailとdocumentationの先頭にprovisionalと明示し、dataの `completionPhase: "prepared"`、`ownerAttempt`（raw node・registration・pattern）と `completionTicket` に出所を付ける。
未確定の本文・guard・後続hookにより仮候補が取り消されることがある。応答は常に `isIncomplete=true` とする。

このviewは現在の原文との一致とExactなSourceMap射影を要求する。曖昧なheader、未知のnested section、参照だけの補完は最新の全文解析を待つ。
Rawな値のsectionを実行文として扱わない。256KiBを超える文書もこの経路を使わない。hover・診断・semantic tokens・signatureは従来の最終解析を使い、仮候補から型や署名を返さない。
特殊な構文名や実装classによる補完側の例外は設けない。

EnterBody後に本文・ExitBodyがownerを拒否またはfaultにした時点で、そのownerの仮候補を失効させる。別ownerへのfallbackを待たず、通常の補完要求は最終解析待ちに戻す。
失効・最終解析・worker failureではopt-in clientへ `skript/completionRefresh` notificationを送る。paramsは `uri, version, generation, revision, ticket, position, phase`、phaseは `revoked | final | failed` である。
これはLSP標準のrefresh要求ではない。既に送ったrequestに二度目の応答を送らず、clientが表示中の同じticket・文書版・位置を確認して補完を再要求する。
clientは失効したitemを先に除去し、閉じたmenuを再び開かない。非opt-in clientは構造内の要求を従来どおり全文解析まで保持する。
編集中・close/reopenのgeneration/revision/versionと取消tokenを全progress eventでも確認し、古い結果は公開しない。

## 引数案内

`signatureHelpProvider` は `(`、`,`、空白をtrigger、`)`をretriggerとして公開する。
現在の原文と同じPattern AST graphから元文法を表示し、現在のslotをその文法中のUTF-16範囲で示す。
先頭がslotの文法、option/choice、空白を挟まない固定文字、引数内のExpression/Conditionにも同じ探索を使う。
固定文字の一致が多く、消費済みslotが少ない候補を選び、同点はCatalog順に保つ。
入れ子は合意できたslot開始位置から最大16段まで進める。文字列・変数名・補間の区切り走査は補完と共有する。
これは構造上の入力支援であり、型、Addon guard、実行scope、意味的に正しいoverloadの採用を保証しない。

Functionは現在のnative registryで採用した文書・projectの定義とCatalogを使い、同shapeのshadowとvoid署名も維持する。
現在revisionの文書解析前はFunction案内を保留する。同名Functionがなければ登録patternの探索を続ける。
括弧・引用符・変数名・補間内のcommaを区別し、nativeの区切り規則で単数の位置引数を数える。
named argument、複数値parameter、範囲外の引数、曖昧なslot、固定語の途中では、未確定の引数を強調しない。

`labelOffsetSupport`がないclientには一意なparameter labelを文字列で返す。重複labelでは誤った位置を避けるためparameter一覧を省く。
`activeParameterSupport`がないclientにはhelp全体のindexを使い、別候補へ同じindexを誤適用しないようにする。
要求ごとの探索は共有100万操作、prefix4096 byte、文法/関数label1024 byte、最大20署名に制限する。
保留要求は文書ごと64件、server全体256件までで、超過分はRequestFailedにする。
編集・close・個別取消・shutdown・worker failureの終端方針は補完と同じである。結果が無い場合はnullを返す。

## 初期化済みの暗黙引数を示すinlay hint

`textDocument/inlayHint`は、native parserが省略を記録し、実際のDefaultExpression providerが
初期化したimplicit childを表示する。たとえばPlayerを提供するevent内の`send 1`は、
採用patternの省略optionが`to %players%`なら`to <event player>`を示せる。
`on load`でPlayerを補作せず、明示receiverには同じhintを追加しない。

省略理由は`skipped-option`・`unselected-choice`・`hook-bypassed`を区別する。現在の表示対象は
`skipped-option`だけである。元patternの接続語と一つの対応slotを確認し、nested option/choice/regexから
語句を勝手に選ぶ必要がある場合は表示を省略する。native owner・provider所有権・capture geometry・
Catalog登録を照合し、macro由来を含むzero-width Exact/Anchored位置を原文のUTF-16位置へ安全に投影する。

event-valueは実際に選ばれたEventValueの出典を示す。SimpleLiteralの保存済み実値はsnapshotにないため、
`<registered default: 型名>`と表示し、tooltipで実値が利用できないことを明記する。
これは型が分かるという情報であり、`1`・`true`等の値を推定しない。独自providerや根拠不足は表示を省略する。
hintは`textEdits`を持たず、sourceやcursorの編集可能な文字を増やさない。要求range内だけを返し、resolve要求は使わない。
tooltip内の説明とpatternは内容に合わせた安全なMarkdown fenceで区切る。

## 計測と検証

`window/logMessage` にstartupと解析単位のJSONを出力する。
`startupMs` はsnapshot読み込み・WASM host初期化・Catalog入力候補索引を含むworker準備時間。
`queueMs` は要求の待機、`parseMs` は `parse_document`、`reportMs` は診断・ホバー・token・補完view変換と結果の後始末、
`analysisMs` はworker処理全体、`cancelToFinishMs` は取消要求からworkerが返るまでの時間を表す。
取消前に終了していた場合や取消されなかった場合はnullになる。
これらはRustのbuild/compile時間や、clientが画面へ診断を描くまでの時間を含まない。
最新版へまとめられて実行されなかった要求の解析時間は計上しない。
`details.project`はepoch・nativeRevision・scannedDocuments・parsedDocuments・changedExportNamesを含む。
`parsedDocuments`はproject内のparser要求数で、古いASTを再利用した数ではない。
`details.incremental`は直近document parseのeligible・hits・misses・dependencyMisses・stored・discardedを示す。
project全体の総hook数やclient描画の短縮時間とは区別する。
`skript-lsp completion timing`の`source`は`syntax`、`prepared`、`analysis`を示す。
`skript-lsp preparation timing`は現在のjobの`prepareMs`を示す。本文解析完了時間ではない。
`skript-lsp signature timing`も同じ区分で`queryMs`、`waitMs`、署名数を記録する。
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


## Workspace discovery and incomplete projects

The client selects one explicit workspace root. The normal Neovim integration defaults to
the opened file's directory; an arbitrary ancestor Git repository does not identify a Skript
source scope. Set SKRIPT_LSP_ROOT explicitly when multiple source directories belong together.
Nested repositories (.git directory or worktree file) are separate project boundaries.
Discovery does not follow symlinks or Windows reparse points. Ordinary folders, including
hidden, dependency-looking and generated-looking names, remain eligible; no name blacklist
silently changes the language namespace.

The limits remain 50,000 directory entries, 512 sources, directory depth 64, 1 MiB per file
and 16 MiB total. Discovery/closed-file failures publish lsp.project-incomplete and analyze
the current immutable open buffer afresh without previous imports. Function calls remain
unresolved while the global namespace is incomplete, including catalog functions that could
be shadowed by undiscovered definitions. Unaffected statements keep their local analysis.
Function signatures/completions and exhaustive references are unavailable in this mode.
References and project-wide definitions return RequestFailed rather than a partial list.
Cancellation and parser/extension failures retain their distinct handling.
Timing details.project includes root, currentDocument, complete and incompleteReason so the
selected scope can be diagnosed without scanning unrelated parent directories.

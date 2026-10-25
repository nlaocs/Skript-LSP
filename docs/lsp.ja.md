# stdio Language Server

`skript-lsp --stdio`は、一つのprojectとSSG snapshotで動くLanguage Serverである。診断・ホバー・
Semantic Tokens・補完・引数案内・定義移動・参照検索・安全なリネーム・関数呼び出し階層・構造一覧・
inlay hintsを提供する。各機能の範囲は下記の意味情報と原文位置の契約に従う。
採用済みの根拠から保守的な変数型flowも得る。
必須CoreLibraryはexeへ埋め込まれ、通常の解析にJavaやMinecraftサーバーは必要ない。
全構文・全Addonの互換性、runtime値の評価、完全な手続き間解析を保証する完成版ではない。

## 初期化と対応メソッド

clientは `rootUri` または一件の `workspaceFolders` と、次の `initializationOptions` を渡す。
複数workspace、snapshot指定漏れ、読めないsnapshot、不正なAddonは初期化エラーとして返す。
相対pathはproject rootから解決する。読み込み後のsnapshot変更にはserverの再起動が必要。
LSPは`ssg::load_for_parsing`を使い、現行schema 7とCoreLibraryが必要とするmetadataを検証する。
互換archive readerの`ssg::load`がschema 3～7を読めることとは別で、古い形式をそのままruntime解析に使えるとは限らない。
必要なfieldが欠けている場合はexportを再生成する。任意の型推論metadataがないだけならUnknownとして残る場合もある。

```json
{
  "snapshotPath": "C:/path/to/complete-ssg-export",
  "addonPaths": []
}
```

起動は`skript-lsp`または`skript-lsp --stdio`。`--help`で引数を確認できる。
snapshotとAddonはCLI flagではなく、上記の初期化設定で指定する。

| 機能 | メソッド |
| --- | --- |
| ホバー・補完・引数案内 | `textDocument/hover`、`textDocument/completion`、`textDocument/signatureHelp` |
| 定義・参照 | `textDocument/definition`、`textDocument/references` |
| リネーム | `textDocument/prepareRename`、`textDocument/rename`。version付きdocumentChangesに対応するclientのみ |
| 呼び出し階層 | `textDocument/prepareCallHierarchy`、`callHierarchy/incomingCalls`、`callHierarchy/outgoingCalls` |
| 構造一覧 | `textDocument/documentSymbol` |
| 仮想文字 | `textDocument/inlayHint`。resolveなし |
| 意味による色付け | 交渉済みの`textDocument/semanticTokens/full`。range/deltaなし |

接続寿命は`initialize`、`initialized`、`shutdown`、`exit`で扱う。通知は
`textDocument/didOpen`・`didChange`・`didClose`、`workspace/didChangeWatchedFiles`、
`$/cancelRequest`に対応する。completion resolveも提供しない。
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
読めないfile、不正UTF-8、予算超過等による探索未完了は`lsp.project-incomplete`で明示する。
その場合も、以前のimportを持ち越さず現在のopen bufferを新しく解析し、安全な局所結果は保持する。
名前空間が不完全な間は、未発見の定義がshadowする可能性のあるCatalog関数を含め、関数を未解決にする。
関数補完・署名と網羅的なproject参照検索は利用できず、projectの定義・参照要求は不完全な一覧ではなく
RequestFailedを返す。取消・parser障害・extension障害とは区別する。

任意の祖先Git repositoryをSkriptのrootに選ばない。入れ子の`.git` directory/worktree fileは別projectの境界となる。
通常folderはhidden・依存物・生成物のような名前でも探索対象であり、名前のblacklistで名前空間を変えない。
解析中にclosed sourceが変化した場合も世代を照合し、古い結果を現在の結果として公開しない。

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

公開WITは`nlaocs:skript-parser-addon@0.44.0`、ABIは`26.0`である。旧ABIのcomponentは再buildする。
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
解決済みの単独Function呼び出し文も解析する。戻り値なしの呼び出しを式の値として扱うこととは区別する。

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
関数名はnativeのdefinition/registrationとsource witnessを照合する。通常の関数はprojectに公開し、
local functionは文書内に限定する。いずれも本文のlocal変数・引数はその関数の実行scopeで分離し、
呼出元のlocal変数を引き継がない。
未open fileの現在の関数宣言にも移動できる。参照要求の`includeDeclaration`を尊重する。

汎用engineはroot/実行文脈を分け、根拠のある代入・parameter・delete・list操作と到達書込を表現する。
型flowは、要求型への変換前に変数へ保存された型を扱う。引数の宣言型、parse contextの要求型、
地点ごとの現在値の推論は別の事実である。providerの根拠に基づき、primitiveの生成、parameter binding、
採用した関数の戻り値型、安全な継承`beforeChange` clone/set経路を扱う。戻り値classの宣言だけでは
非空性や他localの不変性を証明せず、未解決のFunction/ExpressionList結果はUnknownになり得る。
型付きparameter/returnにも未設定の可能性を残す。評価順と文字列化の根拠は安全なoperandで型を保持する
ための情報で、処理全体のpure宣言ではない。未知の変換・副作用・alias・制御flowでは、要求型を借りず
影響する事実を忘れる。補間内の後続読取でUnknownになる制約も残るが、標準Numberの文字列化が
無関係なlocalを書き換えると実証したものではない。

branchは実行されない経路も含めてunionし、未設定の可能性を保持する。loopや意味を公開していない
control flow、未知の副作用やfunction call、動的名・aliasが事実を壊す場合は保守的に忘れる。
global変数の実行時の値や別eventの実行順を推測せず、手続き間の戻り値・副作用解析は行わない。
公開identityのcase policyが不明なら綴りを勝手に正規化しない。型が証明できない場合は
`unknown (runtime value is not established here)`を表示する。Unknownを要求型へ置き換えない。
古いschema 7 snapshotも引き続き読み込めます。cloner/`beforeChange`の確かな情報がない場合、
影響する代入や読取の事実はUnknownのまま保持し、型宣言だけから保証を補いません。

providerは`skript.semantic.storage-result`で、`beforeChange`・複製・`serializeAs`変換を通した後の
ローカルscalarへの保存型を公開できます。完全な型集合・単複・未設定の可能性と、ほかのローカルbindingを
保持できるかは別々の情報です。型保証があっても未知の読取やcallbackをpure扱いせず、失効した過去の型を
現在型へ戻しません。保存後型の根拠はHoverで`provider-certified post-storage type`と表示します。

標準2.15.4のproperty式は、最終的な型解決と変換の後にこの契約を生成します。textcomponentはAdventureの
immutable／NonExtendableというAPI契約と、実ClassInfoの複製・保存情報に基づく対応です。標準登録だけの
Type登録一覧と該当ClassInfoの原データを確認できる場合に限定し、Addon型・複製override・情報欠落があれば保証を出しません。
無関係なAddon型の登録でもこの保証が止まる保守的な制限があります。API契約外の独自Component実装は
対象外で、任意のAddon値・Slot変換・global/listへの保存型を一律に保証するものではありません。
たとえば`player's name`の右辺はこの版ではstringではなくtextcomponentです。存在保証がない場合、
代入先には`textcomponent?`を表示します。通常の読取位置には型inlayを追加しません。

Type一覧はcatalog-data v4の上限付きフィールド射影でsource digestごとに一度確認し、各property式へ
全Typeのpayloadを繰り返し渡しません。射影APIがない旧hostは新Coreをlink時に拒否します。
API対応hostでCatalogが利用できない場合はUnknownを維持します。

定義移動は現在の到達書込を返す。証明できない場合の静的候補やglobalの書込が複数あれば複数位置に
なり得るため、一意なruntime bindingを保証しない。動的identityや曖昧なmacro originは省略する。
解析中の要求は文書64件・server全体256件まで保持し、超過分はRequestFailedにする。
UTF-16 surrogate途中・逆転rangeは待機前にInvalidParams、閉じたURIは空配列を返す。
編集・project更新・closeは旧要求へContentModified、個別取消とshutdownはRequestCancelledを返す。
要求の取消は共有解析を止めず、旧世代のannotationを後から返さない。

型候補の集合、候補集合の完全性、推論・heuristic・Unknownの根拠区分、未設定の可能性を独立して保持する。
既知の完全なunionはheuristicへ格下げしない。現在の分岐に未知の型が混ざる場合は、得られた候補を不完全な集合として
保持できるが、それだけで補完候補を除外したり確定的な型不一致診断を出したりしない。根拠の区分は代入・合流でも維持する。
型依存の保持判定にも完全な推論根拠を要求し、非空の保証が必要な場合は未設定の可能性も確認する。
ホバーでは候補の由来を採用済み結果・parameter・代入・現在の制御flow合流と区別する。
未知の処理を副作用なしとはせず、失効した過去の型や宣言型を自動的にheuristic候補として復活させない。
新しいheuristic生成方針は追加していない。現在型が実際に失効した場合はUnknownにし、直前の型と
失効理由・位置はホバーの履歴としてのみ保持する。履歴は文書内の上限付き情報で、現在型の候補、
補完の除外条件、確定診断、到達書込を復活させない。既知の再代入は新しい現在型へ置き換える。

### 書き込み対象の型ヒント

採用済み変更契約のtarget captureと静的な変数identityから、bindingの書き込み箇所にだけ
型inlay hintを表示する。初回代入・再代入の閉じ波カッコ直後に、書き込み後の型を出す。
通常の読取り、右辺の変数、propertyの対象となる変数読取りには表示しない。
型が明記されたparameter宣言には重複表示せず、そのparameterへの後の代入は別の書き込みとして扱う。
契約やExactなsource対応がなければ、構文名や字句から書き込みを推測しない。

型名はCatalogのcode nameを共通表示に使う。例えば現在の根拠が揃う場合は `number`、
未設定の可能性があれば `number?`、現在値に有効なheuristicなら `~number` / `~number?`、
現在型が分からなければ `?` と表示する。完全なunionはheuristicではない。
不完全な候補集合は短いヒントでは `?` とし、候補と完全性はホバーで区別する。
listの要素型には `[]` を付け、単複と未設定の可能性を混同しない。
ヒントは仮想文字で、textEditsを持たない。引数名・省略引数ヒントと共存し、矢印による変化表示は行わない。

### 型名の補完と parsed as

ClassInfo を要求する式の位置では、登録されたユーザー入力patternに一致する型名を補完する。
内部code nameや表示用nounだけで候補を作らず、有限に展開できるpatternも同じ登録順matcherで再検証する。
Addonの型も同じ経路を使う。無限・未対応のregexや予算上限は不完全として扱い、非対応型と断定しない。
一般のClassInfo位置ではparserを持たない型も合法なので一律には除外しない。
補完詳細にはparserの有無と観測済みcontextを表示し、親provider固有の対象型条件は未検証と明示する。

Coreの既存解析は parsed as の対象型から戻り値型と単複を解決する。
parser不在やtextをtextとして解析する指定などの確定した拒否は、対応するnative captureが一意なら対象型の範囲を指す。
能力metadataが無い場合やcontextが未観測の場合は未確定とし、非対応と断定しない。
patternでは能力未確定のplaceholderより後も検査し、確定した構文エラーを隠さない。
quoted patternのregex captureには現在mapped spanの輸送がないため、その診断は式全体を指す。
これらはload-timeの検証であり、定数文字列でもruntime parserを実行したり解析成功・値の存在を保証したりしない。

監査済み2.15.4の標準NumberParserについては、成功時の実装上の出力クラスと、
実際のClassInfoのclone・serializeAs情報をCoreで照合し、安全なlocal SETで数値型の上限と
未設定の可能性を代入先へ渡す。入力文字列が数値らしく見えても実行時の成功を断定せず、
`number?` を `number` に変えない。未知のparser・保存処理にはこの保証を流用しない。
これは任意のNumber subclassの文字列化をpureとする保証ではなく、Number引数の読取りや
補間後の型保持には従来の保守的な制限が残る。

## 安全なリネーム

`workspace.workspaceEdit.documentChanges = true`を表明するclientだけに`renameProvider`を公開する。
`prepareRename`と`rename`は現在版の解析とsource集合を確認し、解析待ちは保留せずContentModifiedを返す。
名前の一致による文字列置換ではなく、参照検索と共通のidentity・scope・原文証明を使う。

- 関数はソースのある宣言・登録identityと編集可能な名前範囲を必要とする。全project参加文書の編集可能な
  参照範囲が網羅され、宣言・Catalog・既存lookupとの衝突がないことを確認する。local functionの可視範囲を保つ。
- 変数は同じ実行scopeとcontextに属する、静的な単一localだけを扱う。対象文書とscope内の静的網羅性を要求する。
  global、list family、parameter、動的名は対象外。対象scopeに未知のidentity/aliasがある場合や、競合する名前・
  list family、曖昧なmacro origin、重複した編集所有権がある場合は拒否する。
- 名前はASCII識別子`[A-Za-z_][A-Za-z0-9_]*`に限定する。localのbraceと接頭辞は名前の編集範囲に含めない。
- 実際に編集する全fileがopenかつ現在版であることを要求し、version付き`documentChanges`を返す。
  closed fileへのversionなし編集へfallbackしない。prepareの時点でもこの条件を確認する。

read-onlyの参照が得られることは、安全に変更できることの証明ではない。serverは編集を返すだけで、原本へ適用しない。
clientにもversionの不一致を拒否する編集処理が必要である。

## 関数呼び出し階層

解決済みの関数宣言または呼び出しからprepareし、nativeのdefinition/registration/signature/source witnessに
基づいてincoming/outgoing callsを返す。nodeとedgeはソースのある関数本文同士を対象にし、再帰・複数call site・
複数文書を扱う。local functionは文書外へ漏らさない。関数headerのdefault式は本文のcall edgeにしない。

Event/Commandの入口、ソース本文のないCatalog関数、未解決・動的・原文を証明できない呼び出しをnodeとして補作しない。
実行時の全呼び出しgraphではない。同じ相手へのcall siteはまとめ、原文順で決定的に返す。
itemにはserver寿命・project世代・source・定義identityを結び付け、編集・close/reopen・依存変更後の古いitemを拒否する。
prepareは現在の解析を待つが、follow-upは解析待ち・不完全projectを拒否する。現在のprojectにあるclosed文書もread-onlyの対象になる。

## Document Symbols（構造一覧）

Core/Addonが採用したStructure/Sectionへ公開する共通metadataを使い、function・command・event等を一覧にする。
LSP側でheaderの綴りを再解析したり、特定の構文名を列挙したりしない。metadataのない構文は自動では追加されない。

現在のRawTreeの親子関係、block全体のrange、名前等のselectionRangeを確認し、Exactな原文のUTF-16範囲へ投影する。
位置が曖昧なmacro、混在source、不明schemaや未対応ownerは省略する。未完成文書でも、安全な範囲を証明できる構造は残せる。
clientが対応すれば階層付きDocumentSymbol、それ以外はcontainerName付きのSymbolInformationを返す。
最終解析の情報を使い、prepared補完から一覧を推定しない。project全体のworkspace symbolsとは別機能である。

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
全候補に共通の関連度順位を適用してから、全体を最大512件にする。登録順で先に512件を採る処理ではない。
有限のcapture値も共通の安全な展開から候補にできるが、任意のregexを無制限に列挙しない。
切り詰め時は`isIncomplete`を返し、追加入力したprefixで候補を再検索する。画面の表示件数とserverの転送上限は別である。
template化するpattern sourceは1024 byteまで、選択肢はpatternごと最大16通り、深さ32に制限する。
同じ登録・先頭固定語の重複をまとめる。
継続語は選択肢の直積を作らず、graphを最大2048 node、探索をpatternごと4096状態・slotごと128境界、
要求全体で100万操作（状態・辺・文字走査）、入力prefixを4096 byteまでに制限する。
切り詰めがあれば`isIncomplete`を返す。各候補のdocumentationには省略しない元patternまたは関数signatureを含める。

これらは入力支援候補であり、採用済みの構文ではない。nativeの期待slotを確定できる場合は、parserと共通の
継承・converter・union・単複規則でExpression、関数戻り値、現在地点の変数型を絞る。候補自身の型を使い、
同じ挿入文字列の別登録から型を借りない。Dynamic・Unknown・期待slot未確定では過剰除外しない。
一般のteleport第一slotはentitiesであり、Player専用とは扱わない。
別文法のcontinuationは引数・型・Addon guardを未検証のまま残る場合があり、その旨を説明へ表示する。
型付き候補が絞られていても、全候補の意味的適合が検証済みとは限らない。任意のregexから具体値を補作しない。
関数は現在のnative registryで採用した文書・projectの定義とCatalogから取り、parserと同じ引数のcomponent type・single・optional変換を使う。
同名・同shapeの文書内関数によるshadowを維持し、戻り値のないshadowからCatalogの式候補を復活させない。
補完一覧での戻り値型フィルタは、完成した呼び出しの引数型に基づくoverload採用とは別である。

宣言引数はproviderの skript.semantic.parameters v1/v2契約から得る。
所有rootとExactなheader範囲に結び付け、最初の参照前からその本文・確定した子scopeだけで候補にする。
宣言型・single/list・定義位置は declaredType と parameterDeclaration に保持するが、本文での現在値の型とは扱わない。
未知型も除外せず、pluralのruntime名とpercent escapeを保つ。異なる関数や関数外へ持ち出さない。
finalでは現在のprovider証明を要求する。本文の混合provider等でこの証明を満たさない場合は候補を確定しない。
braceや改行を含むruntime名の挿入は未対応で、動的・入れ子名の安全な置換範囲を推測しない。

静的変数名は最終ASTのExpression、公開variable schema、子なし、原文一致のExact originを確認した参照履歴から得る。
localはカーソルより前に同じRawTree bodyまたはその祖先bodyで観測した参照だけを示し、兄弟bodyへ持ち出さない。
参照履歴候補そのものは宣言やruntime値を保証しない。解析済み変数には独立した現在のflow推論を説明へ添えられる。
動的名、未知schema、位置が曖昧な生成文、fault領域は履歴へ加えない。
文字列本文・コメントには候補を出さず、補間内では入力支援用の字句状態を使う。字句判定自体はparserの受理を保証しない。

completionの説明形式は`completionItem.documentationFormat`の対応する先頭値を選び、hoverの交渉と独立にする。
省略時はPlainTextである。Markdownではpatternとtemplateを`skript-pattern`、実コード例を`skript`として区別する。
構文・戻り値型・説明・Pattern・Examples・Definitionを共通の整形で表示する。Expressionの型は既知・Dynamic・
Unknown、単複、候補型の完全性を区別し、登録上の未確定型を確定型と表示しない。
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

clientが `capabilities.experimental.skriptCompletionRefresh = true` を送る場合、serverは同じcapabilityを返し、同じ全文解析のText/Tree macroと全rootのEnterBodyが終わった段階を補完に公開する。依存解析で処理した別のopen文書も、producerではなく実際のsourceへ配信する。nativeが一意な入れ子sectionのEnterBodyを受理した後は、その現在のTrigger/Conditions本文scopeを小さなdeltaで追加する。未確定・曖昧な親を越えてscopeを推測しない。
別のparse、旧revisionのAST、型・変数・文書関数の履歴は使わない。現在の一意なheaderが宣言した引数は同じ契約で仮候補に含め、ownerのretry／撤回で置換・失効させる。SSGと実際のowner hookのbody mode、凍結したEntryValidatorを使い、native parserと同じ宣言index選択・重複判定を共有する。
候補は成功した意味解析の証明ではない。各itemのdetailとdocumentationの先頭にprovisionalと明示し、dataの `completionPhase: "prepared"`、`ownerAttempt`（raw node・registration・pattern・native内のattempt ID）と `completionTicket` に出所を付ける。
未確定の本文・guard・後続hookにより仮候補が取り消されることがある。応答は常に `isIncomplete=true` とする。

このviewは現在の原文との一致とExactなSourceMap射影を要求する。曖昧なheader、未知のnested section、参照だけの補完は最新の全文解析を待つ。
Rawな値のsectionを実行文として扱わない。256KiBを超える文書もこの経路を使わない。hover・診断・semantic tokens・signatureは従来の最終解析を使い、仮候補から現在値の型flowや署名を確定しない。引数候補が保持する型はheaderの宣言型である。
特殊な構文名や実装classによる補完側の例外は設けない。

EnterBody後に本文・ExitBodyがownerを拒否またはfaultにした時点で、そのownerの仮候補を失効させる。別ownerへのfallbackを待たず、通常の補完要求は最終解析待ちに戻す。
失効・最終解析・worker failureではopt-in clientへ `skript/completionRefresh` notificationを送る。paramsは `uri, version, generation, revision, ticket, position, phase`、phaseは `revoked | prepared | analysis-restarted | final | failed` である。
これはLSP標準のrefresh要求ではない。既に送ったrequestに二度目の応答を送らず、clientが表示中の同じticket・文書版・位置を確認して補完を再要求する。
clientは失効したitemを先に除去し、閉じたmenuを再び開かない。非opt-in clientは構造内の要求を従来どおり全文解析まで保持する。
編集中・close/reopenのgeneration/revision/versionと取消tokenを全progress eventでも確認し、古い結果は公開しない。source epoch・原文の同一性・producer revision・worker全体で単調増加するnative revisionも照合する。解析開始は旧viewを破棄し、abortはtombstoneを残して同じ試行の遅延通知による復活を防ぐ。親の失効は全raw子孫に及び、重複投影で表示行が除外されてもscopeを残さない。

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

## 関数の実引数名を示すinlay hint

採用した関数呼び出しの共通metadataにあるparameterと子nodeの対応を使い、明示した実引数の前に`name:`を表示する。
入れ子・複数値の対応もnativeが確定した範囲だけを使う。既に名前を指定した引数や省略引数へ重ねて付けず、
曖昧・未完成の呼び出しから名前を推測しない。関数名による特例や、LSP側でのsignature文字列の再解析は行わない。
現在版のrangeと原文対応を要求し、編集後に古いhintを残さない。下記の暗黙引数hintと共存し、どちらもtextEditsを持たない。
変数の閉じbrace横へ型を示すhintは未実装である。

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
`skript-lsp preparation timing`は通知先のuri/version、nativeRevision、phaseとproducer job開始からの`prepareMs`を示す。本文解析完了時間ではない。入れ子通知は通常の深さ優先解析がそのsectionへ到達してから発生するため、先行する本文の待機は残る。
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


## 大きな文書と表示上の制限

prepared補完の到達時刻、全文解析完了、semantic tokens応答、clientの適用・描画は別の時刻である。
大きな文書では初回の全文解析待ちが残り、20秒等の一定時点で未着色でも、全結果が失われたとは限らない。
本体を並列に解析するworkerやviewport優先は導入していない。宣言やhookの安全な再利用と、全文ASTの再利用も区別する。
一つの巨大な実行本文と多数のStructureでは処理負荷が異なり、行数だけから応答時間を保証しない。

補完phaseの更新では、clientのmenu再表示によりちらつく場合がある。同じ候補の再通知抑制やclient側の
更新方法も含め、ちらつきの完全解消は保証していない。headless/APIの受信結果と、実際の画面描画を混同しない。


[Source-backed storage type guarantees and limitations](storage-domains.md)

## 混在したソース範囲の精度

option置換の前後で変更されていない断片は、hoverや書き込み先の型ヒントを保持できます。
一方、providerが複数のoriginを持つ親spanから追加解析の子spanを作る場合、各断片のvirtual位置を
証明できず、親全体の範囲をReplacedとして保持することがあります。その子のhoverやsemantic tokensは
利用できません。たとえばoptionを含む文字列内の補間では、物理的に残っている関数もこの制限を受けます。
これは型がUnknownであることとは別の位置情報の制限です。生成部分や曖昧な子の位置を推測して補いません。

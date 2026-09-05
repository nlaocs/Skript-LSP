# skript-parser

[English](README.md)

`skript-parser`は、`.sk` documentのsource mapping、lossless RawTree、登録構文照合、再帰的な
構文木を所有します。Expression、Condition、Effect、Event header、Section、EntryValidator付きの
top-level Structureを解析し、document定義Functionの呼び出しで使うtransactional registryも持ちます。

各stageはlibrary APIです。callerがCatalogと`ExpressionParseEnvironment`を提供し、実際の
WASM環境とCoreLibraryの意味処理は`parser-wasm`が接続します。Skriptコードの実行、LSP/HTTP
transport、すべてのaddonの完全な意味解析は提供しません。未認識・拒否された入力もpartial treeと
diagnosticとして保持します。

## syntax-pattern-parserと分かれている理由

`syntax-pattern-parser`は次のような登録patternを解析します。

```text
send %string% to %players%
```

`skript-parser`は、次のように利用者が記述したsourceを扱います。

```skript
send "hello" to player
```

登録patternは「何が一致できるか」を表します。document parserは「利用者が何を書いたか」、
「どこに書いたか」、「preprocessによってどう変わったか」を追跡します。

## 公開model

### Text range

`TextRange`は半開区間`start..end`のUTF-8 byte rangeです。character boundaryを検証し、
安全なsource slice、包含・交差判定、zero-width cursorやEOF位置の表現を行えます。

このcrateのoffsetはすべてbyte単位です。Unicode scalar数でもLSPのUTF-16 positionでも
ありません。LSP positionへの変換はprotocol境界の責務です。

### Source map

`SourceMap`は、重複しないvirtual rangeと、そのoriginal originを保持します。
`OriginKind`はmappingの生成方法を記録します。

- `Exact`: virtual textがoriginal textへ直接対応する
- `Replaced`: 生成・変換されたtextがoriginal rangeを置き換える
- `Anchored`: 生成textがoriginalのzero-width位置へ結び付く

`MappedSource`はoriginal text、現在のvirtual text、検証済みSourceMap、ExpansionGraphを
所有します。複数のsource rangeや過去のexpansionを結合した生成segmentは、複数のoriginal
originを保持できます。`Exact` segmentは一対一のままです。`map_range`は関連するすべての
originを持つ`MappedSpan`を返し、生成sourceの情報も保持します。

### Text edit

`TextEdit`は半開byte rangeを置換し、生成textを明示的なoriginal anchorへ結び付けることも
できます。`MappedSource::apply_text_edits`はmacroが返したeditをsortし、batch全体を検証して
atomicに適用し、合成済みSourceMapとExpansionGraph entryを持つ新しいsourceを返します。

変更されないtextは既存originを維持します。置換には`Replaced`、zero-width insertには
`Anchored`を使い、連続するmacroでは親expansionへのlinkを維持します。空のedit listは
成功するno-opとして扱い、replacementも空のzero-width editは拒否します。複数editのbatchは
全editのoriginをcall siteとして記録します。すでに複数originを結合したtextを置換する場合も、
最初の1件だけを選ばず、すべてのoriginを次のmappingへ引き継ぎます。

`TextEditApplication::generated_bytes`はbatchが追加したreplacementのbyte数です。WASM hostは
この値をpipeline quotaの検証に使います。

### 展開の出典

`Expansion`は次を記録します。

- 安定した`ExpansionId`
- Text、Tree、ASTの展開種別
- 所有するWASM componentとhook
- 1件以上のcall siteと任意のdefinition site
- macro hygiene用の`SyntaxContextId`

`ExpansionGraph`は、有向非巡回graph全体のID重複、空のowner、未知の参照、cycleを検証します。
`backtrace`は単純なconsumer向けに最も内側の展開からrootまでの主経路を返し、`backtraces`は
異なる親expansion経路をすべて返します。

`Ast`という展開種別とsyntax-context IDはprovenance用の基礎データです。AST macroの実行pipelineや
完全なhygienic name resolutionまで実装済みであることを意味しません。

### Lossless RawTree

`parse_raw_tree`は`MappedSource`をarena形式の`RawTree`へ変換します。呼び出し側は通常
`RawTreeOptions::for_skript_version`で作成した`RawTreeOptions`を必ず渡し、version依存の字句規則を
明示的に選択します。nodeはsource順の`RawNodeId`を使い、次の種類に分類されます。

- `Blank`
- `Comment`
- `Simple`
- `Section`
- `Invalid`

各nodeはphysical `RawLine`、raw text、Skript規則でdecodeしたtext、indentation、末尾空白、
comment、line endingのtriviaを保持します。rangeはすべて`MappedSpan`なので、Text macroが
生成したlineでもoriginal source originとexpansion provenanceを失いません。

Section nodeはheader、body、subtree全体のspanを個別に公開します。空bodyはheader line直後の
zero-width spanです。treeはparent/child関係と、検出したspaceまたはtabのindentation unitも
保持します。

comment分離はSkriptの`Node.splitLine`に合わせています。

- quoted string外の`##`はliteral `#` 1個になる
- quoted string内の`#`はcommentにならない
- variableと`%...%`はSkriptのstate machineと同じ遷移を使う
- trim後に`###`と完全一致するlineだけがblock commentを開閉する
- blank/comment lineは現在openしているSectionに所属し続ける

triple-hash multiline commentは
[Skript 2.9](https://github.com/SkriptLang/Skript/commit/adac6e1984b54924583ce13dea6eb319bc61982c)
で導入されました。そのため`RawTreeOptions::for_skript_version(2, 8)`では各`###` lineを通常の
single-line commentとして扱い、2.9以降でのみblock comment stateを有効にします。line途中の
`###`はいずれのversionでもstateを切り替えず、通常の`##` escapeと`#` line-comment規則に
従います。

Skript runtime loaderと異なり、編集中のdocumentでも後続解析を続ける必要があります。そのため、
space/tabの混在、indent unitの途中までのindent、過剰indentはlineを捨てず、`Invalid` nodeと
diagnosticへ変換します。空Sectionはwarningとなり、未閉鎖block commentはopening markerと
EOFの両方を示します。

`apply_tree_edit`は生成local-ID fragmentを検証し、入力treeを変更せずに対象nodeまたは
Section bodyへ適用します。新しいRawNode IDを割り当て、Tree expansionと生成syntax contextを
登録し、すべての生成spanを置換対象nodeのcall-siteへ対応付けます。WIT変換と再帰的な
pre-order dispatchは`parser-wasm` hostが担当します。

## 登録patternの照合

`match_pattern_candidates`は`MatchInput`を解析済み登録patternへ照合します。literal、choice、
group、optional、空branch、regular expression、type expression、parse tag、XOR parse markを
処理します。Java whitespace規則でtrimした入力全体を消費した候補だけが成功します。

regex captureはnumbered groupとUTF-8 byte単位で正確な`MatchSpan`を保持します。type
expressionは`TypeExpressionResolver`へ委譲し、後続の再帰Expression parserがSkriptで有効な
split位置から複数の型付き解決結果を返せます。local rangeは照合対象lineからの相対位置を
維持し、各結果はeditor向けの`MappedSpan` provenanceも同時に持ちます。

異なるsyntax kindが混在する場合、まずcaller内で各kindが最初に現れた順にparser phaseを分けます。
同じkind内ではresolved registry順があるものを先にし、その値、numeric priority、registration順、
declaration順で並びます。patternの登録indexも維持します。結果には採用候補、
後続alternative、または全候補が失敗した場合の最遠failure diagnosticが含まれます。

SSG由来dataでは、`catalog_pattern_candidates`がstatic Catalog registrationを、
`snapshot_pattern_candidates`がfrozen済みstatic/dynamic混在snapshotをPattern ASTの
再解析なしで変換します。後者はregistryのtopological sort済み順序をmatcherへ渡します。
TypeとFunctionは専用parser経路を維持します。

`PatternMatchHooks`はdefinition、registration、pattern、nested elementのbefore/afterを観測・
overrideできます。element pathにはsequence位置とchoice branch位置の両方が含まれます。
state数、backtrack数、regex実行数、評価byte数、regex engine backtrack数の上限により、曖昧または
敵対的なpatternを制限します。transition memoizationは決定的なliteral/regex処理の再実行を
避けます。`RankedFailures`は候補IDとpattern原文を保持し、`FailureTrace`は外側構文・captureと
根本failureを結び付けます。上限付きdiagnostic回復では複数captureの失敗を保持できますが、
不完全な候補を成功扱いしたり、無制限にエラー回復したりするものではありません。

## 再帰Expression解析

`parse_expression`はSSG `Catalog`の登録構文と`ExpressionParseEnvironment`が提供するleaf
parserを統合します。`parse_expression_with_snapshot`はfrozen dynamic syntax snapshotも使い、
before/after解決順、dynamic return metadata、registry revisionをmemo keyへ保持します。

トップレベルの`ExpressionExpectedType`もJava classとsingular/plural要件を組で保持します。`%type%`のalternative、plural、nullable、literal/expression flag、time stateを失わず、Catalogの
class hierarchyと登録converterでreturn typeをfilterします。singular placeholderではMultiple-only結果を拒否し、
typed captureを子`ExpressionNode`として再帰的にASTへ接続します。variable、literal、function、
addon独自parserも登録Expressionと同じ`MappedSpan`を返します。

入力全体を囲む括弧、算術、Expression listには専用nodeがあります。listの分割はquote・variable・
nested parenthesesを考慮し、Function内で括弧に囲まれたlistは1引数として維持します。登録semantic
handlerはreturn class・multiplicity・metadata・parsed captureを補正できます。parser routeが必要な
regex captureは、routeがない状態で意味まで解析できたことにはしません。

`%strings% in upper case`のようなleft-recursive構文はseed-and-growで解析します。照合前にPattern
ASTから先頭・末尾literal制約を保守的に抽出し、depth、candidate、matcher、memoの各上限で
敵対的な再帰を制限します。memo keyにはsource range、expected type/context、StateStore revision、
dynamic registry revisionが含まれます。

## Function呼び出し解析

登録Functionは通常の登録Expression照合より前に専用call parserで処理します。SkriptのUnicode
Function名を認識し、quoted string、variable、nested parenthesesを飛ばしながらargument境界を
求め、SSG `Functions.json`とenvironmentのdocument registryからsignatureを解決します。
`FunctionVersionPolicy`はlocal Function・overload・named argument・return構文のversion境界を
表し、WASM hostがruntime profileから選択します。通常のexact signatureを単一plural parameterの
signatureより先に試します。

各argumentはparameterのJava component typeとして再帰解析します。named argumentは宣言済み
parameterへ並べ替え、optional parameterは省略bindingとして明示し、単一plural parameterには
comma区切りの子Expressionをすべて保持します。`FunctionCall`はFunction名、definition/registration
ID、parameterと子の対応を持ち、親`ExpressionNode`はreturn typeとmultiplicityを保持します。

`ExpressionParseEnvironment::lookup_functions`から現在のdocument/projectで見えるdefinitionを
先に提供できます。同じparameter shapeならcatalog globalをshadowします。hostはすでに
`StructFunction`宣言でこの経路を使い、全headerの登録後にbodyを解析します。
`FunctionRegistryTransaction`は宣言検証とrollbackを行い、`FunctionRegistrySnapshot`は後続lookup用の
signatureを保持します。project全体の複数file symbol indexやvariableの型flow解析とは別機能です。

## Effect解析

`parse_effect`はlosslessな`RawNodeKind::Simple` nodeを1件受け取ります。nodeの正確なcode spanを
まずstatic SSG EffectSection登録、その後に通常Effectへ照合し、採用`EffectCandidate`、決定的なalternative、または
`UnknownEffectNode`を返します。unknownは元の`RawNodeId`、正確なcode text、mapped source span、
順位付きの候補`FailureTrace`を保持するため、LSPの後続回復で未認識lineを失いません。

`parse_effect_with_snapshot`はstatic/dynamic登録をfrozen registry順で統合し、dynamic候補のopaque
handlerとmetadataも保持します。型付きcaptureは内部`ExpressionSession`を共有し、recursion limit、
memo、matcher hook、候補transaction境界を再利用しながら子`ExpressionNode`を接続します。
placeholderを持たないpatternではExpression経路を起動しません。

一行Effect解析に参加するSectionは、static登録で`effectSection`が設定されたものだけです。
source identityは`MatchSyntaxKind::Section`のまま、Sectionのdefinition/registration IDを維持します。
通常Sectionは対象外です。WASM hostではSection target・Effect phaseとして通知し、Section bodyの
lifecycleは実行しません。汎用dynamic Section登録だけではEffectSectionになりません。また、この入口は
単独のvoid Function呼び出しstatementには対応していません。

## Condition解析

`parse_condition`はstatic SSG Conditionをregistration順で照合し、
`parse_condition_with_snapshot`はfrozen dynamic登録も含めます。どちらもJava whitespaceをtrimし、
入力全体を囲む外側の括弧を繰り返し外すため、Skriptの`Condition.parse`と同じ挙動になります。
型付きcaptureは現在の`ExpressionSession`を再利用するため、採用`ConditionNode`は解析済みの子
Expressionを保持します。unknown入力もmapped spanと最遠pattern failureを維持し、後続diagnosticに
利用できます。

## Event解析

`parse_event`は、どのStructureが所有するかを仮定せずEvent headerを照合します。採用されたSSG ID、
Event実装class、参照Bukkit Event class、cancellable、regex capture、addon metadataを保持します。
これによりStructure hookは`host.event`をcapture parserとして使えますが、native parser自身は
`StructEvent`などのSkript実装classに依存しません。

## Section解析

`parse_section`は1件の`RawNodeKind::Section`を受け取り、子をnested SectionまたはEffectとして
再帰的に取得します。header候補は通常Section、EffectSection、SectionExpression指定された
Expression登録を統合します。採用nodeは3種類のmetadata flag、意味付きCondition capture、
子Expression、dynamic handler metadataを保持します。

body modeは`Trigger`（nested SectionとEffect行）、または`Conditions`（Condition行）です。
`Trigger`内でEffectとして一致しなかった行を、汎用的にCondition statementへ再解釈するfallbackはありません。

`ExpressionParseEnvironment::enter_section_children`はbody解析前にchild contextを派生でき、
`exit_section_children`はbody完了後に同じcontextを参照します。hookが承認した通常のcontext更新は
後続siblingへ伝播できますが、parserが所有するSection stackだけはparent scopeへ復元されます。
unknown header、未取得body line、複数候補に取得されたnodeはsubtree全体を中断せず、partial ASTと
`SectionDiagnostic`として保持されます。

child `ExpressionParseContext`には、parser所有の`section_stack`も外側から内側の順で格納します。
各immutable frameはparse内で安定したscope IDとparent、Sectionのdefinition/registration/pattern
identity、addon、実装class、Section flag、意味付きcapture、metadataを保持します。Effect、Condition、
Expression、Section lifecycle hookはすべて同じstackを参照します。rejectされた候補はrollbackし、
Sectionを出ると兄弟nodeを解析する前にparent stackへ戻します。

`SectionParserConfig::root_lifecycle`のdefaultは`Complete`です。要求したroot Sectionの内部として
後続行を解析するcallerは`RetainBody`を選べます。nested Sectionは通常どおり完了しますが、rootの
exit hookは実行せずbody contextをactiveのまま返します。callerはscopeを出る際にparser contextと
transactional addon stateの両方を復元する責任を持ちます。

## Structure解析

`parse_structures`はSkriptと同じtop-level二段階処理を行います。最初にすべてのStructure headerを
照合してenterし、その後で採用候補のbodyを解析します。native側はregistration順、`NodeType`、
lossless RawTree走査、宣言的な`EntryValidator`を担当します。literal、Expression、Trigger、
Container、Section、default、multiple entry、custom separator、nested validatorに対応します。
未知のaddon固有`EntryData`は破棄せず、raw sourceとdiagnosticを保持します。

`ExpressionParseEnvironment::enter_structure`と`exit_structure`が拡張境界です。environmentはheaderの
reject、body contextの派生、`None`/`Raw`/`Entries`/`Trigger`の選択、metadata追加、解析済みbodyの
参照を行えます。`StructEvent`、`StructFunction`、`StructCommand`のようなSkript固有の意味処理は
native moduleではなくWASM componentへ実装します。

`StructureParserConfig::headers_only`は、採用されたheaderと`enter_structure` hookの実行後に停止し、
body解析と`exit_structure`を意図的に省略します。Event context selectorなどのcallerは、Structureへ
入ったtransactionを保持したまま、そのcontext内のstatementを解析できます。

## Invariant

constructorは次の入力を拒否します。

- source length外、またはUTF-8 code point途中のrange
- overlapしている、または不足のあるSourceMap segment
- original sourceに収まらないmapping
- originを持たないSourceMap segment
- `Exact` originを含むmulti-origin segment
- overlapするedit、同じ位置への曖昧なinsert、不正なanchor
- replacementも空のzero-width edit
- 未知のexpansion参照
- cycleを持つexpansion chain

これにより、nested preprocessing後もdiagnosticの位置を有効に保てます。parser stageは
後から位置を再構築せず、`MappedSpan`を引き回してください。

## Source構成

| Module | 役割 |
| --- | --- |
| `text` | `TextRange`とUTF-8 range操作 |
| `source_map` | origin、segment、mapped source、mapped span |
| `expansion` | expansion graph、component/hook ownership、syntax context |
| `tree_edit` | node/body editの検証、生成treeのprovenance |
| `raw_tree` | physical line、comment分離、indentation回復、RawTree |
| `pattern_match` | 登録pattern照合、capture、候補順位、hook、quota |
| `failure` | 候補failureの順位、nested cause、semantic diagnostic span |
| `expression` | 再帰Expression AST、型filter、left recursion、memo、leaf parser統合 |
| `arithmetic` | 演算子の優先順位、catalogによる算術return type |
| `expression_list` | top-level list分割とconjunctionの意味 |
| `function` | 登録・document Function call、named/optional/list引数、overload |
| `function_registry` | 宣言検証、version policy、transaction、frozen document lookup |
| `effect` | Simple nodeのEffect候補、dynamic metadata、nested Expression、unknown回復 |
| `condition` | registration順Condition照合、外側括弧処理、nested Expression |
| `event` | Event header照合、参照Event class、cancellable、意味付きcapture |
| `section` | 再帰Section/Effect body、scoped context、意味付きcapture、partial回復 |
| `structure` | top-level二段階Structure解析、NodeType、EntryValidator、WASM lifecycle hook |
| `catalog_match` | static Catalogとfrozen dynamic snapshotの候補adapter |

公開itemはcrate rootからre-exportされます。

## テスト

```sh
cargo test -p skript-parser --locked
```

test suiteには、multibyte UTF-8 mapping、生成text、replacement range、空source、expansion
backtrace、multi-origin expansion、明示anchor、不正なsegment配置、identity mappingと任意
UTF-8 Text edit適用のproperty testが含まれます。RawTreeについてはSkript公式comment case、
LF/CRLF/最終改行なし、space/tab、nested Section、回復可能な不正indent、空Section、block
comment、macro origin、任意UTF-8入力のlossless性を検証します。pattern matcherでは構造要素、Skriptのliteral/split規則、UTF-8 capture、tag、mark、候補順位、hook、quota、生成source mapping、SSG pattern corpus、任意UTF-8 property caseを検証します。Expression testではstatic/dynamic登録、Core形式leaf、expected typeとMultiplicity filter、nested/left recursion、決定的順序、Function callとdocument shadow、multi-addon Catalog全体を検証します。Effect testでは実schema 3 DummyAddon登録を使い、placeholderなし、型付き、dynamic、unknown lineを検証します。Structure testではNodeType filter、default、multiple/nested entry、custom separator、未知のaddon EntryData、body末尾diagnosticを検証します。

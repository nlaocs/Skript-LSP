# skript-parser

[English](README.md)

`skript-parser`は、実際の`.sk` documentを解析するためのsource-level primitiveを所有します。
現在はUTF-8 range、virtual source mapping、検証済みText editの適用、macro展開の出典、syntax
context、losslessなindentation-based RawTreeを実装しています。

preprocess後sourceのphysical line分割、comment/indentation構造の構築、登録構文patternとの
照合までを実装しています。完全なSkript ASTの生成はまだ行いません。後続stageは、ここで
定義したrange、provenance、RawTree、capture、候補順序の上に追加します。

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

候補はdynamic registryのresolved orderがあればそれを使い、なければnumeric priority、
registration順、declaration順で並びます。patternの登録indexも維持します。結果には採用候補、
後続alternative、または全候補が失敗した場合の最遠failure diagnosticが含まれます。

SSG由来dataでは、`catalog_pattern_candidates`がstatic Catalog registrationを、
`snapshot_pattern_candidates`がfrozen済みstatic/dynamic混在snapshotをPattern ASTの
再解析なしで変換します。後者はregistryのtopological sort済み順序をmatcherへ渡します。
TypeとFunctionは専用parser経路を維持します。

`PatternMatchHooks`はdefinition、registration、pattern、nested elementのbefore/afterを観測・
overrideできます。element pathにはsequence位置とchoice branch位置の両方が含まれます。
state数、backtrack数、regex実行数、評価byte数、regex engine backtrack数の上限により、曖昧または
敵対的なpatternを制限します。transition memoizationは決定的なliteral/regex処理の再実行を
避けます。
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
| `raw_tree` | physical line、comment分離、indentation回復、RawTree |
| `pattern_match` | 登録pattern照合、capture、候補順位、hook、quota |
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
comment、macro origin、任意UTF-8入力のlossless性を検証します。pattern matcherでは構造要素、Skriptのliteral/split規則、UTF-8 capture、tag、mark、候補順位、hook、quota、生成source mapping、SSG pattern corpus、任意UTF-8 property caseを検証します。

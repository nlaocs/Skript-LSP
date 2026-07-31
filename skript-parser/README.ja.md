# skript-parser

[English](README.md)

`skript-parser`は、実際の`.sk` documentを解析するためのsource-level primitiveを所有します。
現在はUTF-8 range、virtual source mapping、検証済みText editの適用、macro展開の出典、syntax
context、losslessなindentation-based RawTreeを実装しています。

preprocess後sourceのphysical line分割とcomment/indentation構造の構築までは実装済みです。
登録構文との照合と完全なSkript ASTの生成はまだ行いません。今後の解析stageは、ここで
定義したinvariantの上に追加します。

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

公開itemはcrate rootからre-exportされます。

## テスト

```sh
cargo test -p skript-parser --locked
```

test suiteには、multibyte UTF-8 mapping、生成text、replacement range、空source、expansion
backtrace、multi-origin expansion、明示anchor、不正なsegment配置、identity mappingと任意
UTF-8 Text edit適用のproperty testが含まれます。RawTreeについてはSkript公式comment case、
LF/CRLF/最終改行なし、space/tab、nested Section、回復可能な不正indent、空Section、block
comment、macro origin、任意UTF-8入力のlossless性を検証します。

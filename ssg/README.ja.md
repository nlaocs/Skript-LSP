# ssg

[English](README.md)

`ssg`は、SkriptSyntaxGeneratorが生成したsnapshot directoryを読み込む信頼境界です。
snapshot全体の完全性、file間の関係、構文patternを検証し、generatorのJSON DTOを
[`syntaxes`](../syntaxes/README.ja.md)のruntime modelへ変換します。

このcrateは生成済みdataを読み込みます。Minecraft serverへの接続、Java classの調査、
snapshotの生成は行いません。

## 対応format

現在はSSG schema version 3と4に対応しています。完全なsnapshotは`Manifest.json`と18個の
data fileで構成されます。

- syntax: Conditions、Effects、Events、Expressions、Functions、Sections、Structures、Types
- relationship: ClassHierarchy、Converters、Comparators、EventValues
- additional registry: Aliases、Differences、Operations、Operators、Properties、PluralRules

正確なfile名は`DATA_FILES`と`ALL_FILES`として公開されています。

schema 3 snapshotは互換性のため読み込みを維持し、現在のgeneratorはschema 4を生成します。
機能の有無はManifest capabilityで
表現されるため、意図的に非対応のregistryと、file欠落・不正値は区別されます。

## 読み込みpipeline

`load(directory)`は次の順序で処理します。

1. `Manifest.json`を読み込み、deserializeする
2. schema versionが3から4の対応範囲内であることを要求する
3. manifestと完全なfile inventoryを検証する
4. すべてのdata fileを読み込む
5. serialized fileに対するcontent digestを検証する
6. manifest由来のsnapshot IDを検証する
7. raw DTOをJSON path付きerrorでdeserializeする
8. file単体とfile間のinvariantを検証する
9. version固有のplural ruleを構築する
10. raw snapshotを`syntaxes::Catalog`へ変換する

返される`Snapshot`は、raw Manifestと正規化済みCatalogの両方を保持します。

```rust
fn load_catalog(
    path: impl AsRef<std::path::Path>,
) -> Result<syntaxes::Catalog, ssg::SnapshotError> {
    let snapshot = ssg::load(path)?;
    println!("{}", snapshot.manifest().snapshot_id);
    Ok(snapshot.into_catalog())
}
```

## 検証内容

検証errorはsource JSON fileを示し、取得できる場合は正確なnested pathも含みます。主な
checkは次のとおりです。

- required file、schema、digest、snapshot identity
- registration orderとIDの一意性
- capabilityとdataの整合性
- resolution stateとnullable valueの整合性
- function signature、parameter、modifier
- typeとJava classの参照
- EventValueのtime rangeとresolution field
- alias target indexと到達可能性

schema 3 / 4 readerのforward compatibilityのため、未知のJSON fieldは受け入れます。ただし
digestは元のserialized file全体を対象とするため、未知fieldによってdigest検証を回避する
ことはできません。

## Moduleの境界

| Module | 役割 |
| --- | --- |
| `raw` | 生成JSONと対応するSerde DTO |
| `loader` | file I/O、schema gate、digest検証、公開Snapshot |
| `validate` | semanticおよびfile間の検証 |
| `convert` | raw DTOから`syntaxes` modelへの変換 |
| `digest` | Java互換content digestとsnapshot IDの計算 |
| `error` | fileとJSON path contextを持つtyped error |

通常のconsumerは`load`と返却されたCatalogを使います。公開されている`raw` moduleは、
format toolingと詳細調査のためのもので、LSP向けの推奨modelではありません。

## syntaxesとの関係

`ssg`はserialization knowledgeを所有します。`syntaxes`は保存形式に依存しないruntimeの
意味を所有します。構文、assignability、EventValue、converter、aliasを検索するcodeは
`ssg::raw`ではなく`syntaxes`へ依存してください。

この分離により、testや将来の別data sourceが、SSG snapshotを装わずCatalogを構築できます。

## テスト

```sh
cargo test -p ssg --locked
```

fixtureは次をcoverします。

- Minecraft 1.12.2上のSkript 2.6.4
- modern multi-addon snapshot
- file欠落と改ざん
- 非対応schema
- forward-compatibleな未知field
- 正確なJSON error path

fixture directoryは、変更していない完全な生成snapshotです。出典は
[`tests/data/README.md`](./tests/data/README.md)に記録されています。

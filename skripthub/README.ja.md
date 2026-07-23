# skripthub

[English](README.md)

`skripthub`は、SkriptHub addon syntax list APIが返す構文data用のlegacy互換layerです。

新しいLSPの構文読み込みには、[`ssg`](../ssg/README.ja.md)と
[`syntaxes`](../syntaxes/README.ja.md)を通して生成済みSSG snapshotを使用してください。
このcrateは移行、corpus調査、古いdata shapeとの互換性のために残されています。language
serverがSkriptHubのnetwork可用性へ依存してはいけません。

## Data source

`api::fetch_data`は、次のURLへblocking GET requestを送ります。

```text
https://skripthub.net/api/v1/addonsyntaxlist/
```

responseは`AbstractAddonSyntaxList`へdeserializeされます。API fieldにはSkriptHub ID、
addon metadata、compatibility text、pattern文字列、return情報、削除状態、documentation
fieldが含まれます。

頻出するresponse文字列は`Arc<str>`へinternし、memory使用量を減らします。

## Legacy変換

`AbstractAddonSyntaxListEntry::to_syntax`はAPI entryを次のlegacy entity typeのいずれかへ
変換します。

- Event
- Condition
- Effect
- Expression
- Type
- Function
- Section
- Structure

Function以外のpatternは、明示的に渡された`PluralRules`を使い、
`syntax-pattern-parser`で解析します。推測したruleやglobal hardcodeのplural設定を
使用してはいけません。

返される`SkriptHubSyntax` trait objectはSkriptHubへのlinkを保持します。SSGで使用する
正規化済み`syntaxes::Syntax` modelとは別のものです。

## Function pattern互換

SkriptHubはfunctionを次のようなflatten済み文字列で公開します。

```text
example(value: string = "default")
```

`function_pattern`には、この古い表現専用のparserがあります。function名、argument、
default文字列、parenthesis、separator位置を検証します。

SSGの`Functions.json`には、name、parameter、modifier、default、return metadataがすでに
構造化されています。SSG functionをlegacy文字列parserへ通してはいけません。

## Module構成

| Module | 役割 |
| --- | --- |
| `api` | remote response DTOとblocking fetch helper |
| `addon_syntax_list` | legacy syntax traitとentity変換 |
| `function_pattern` | flattenされたSkriptHub function文字列用parser |

## 適切な用途

次の用途に使用できます。

- parserがhistorical SkriptHub entryを受理できるか調べる
- 旧service dependencyを除去する間のmigration比較
- 保存済みSkriptHub response fixtureを読む

次の用途には使用しません。

- 通常のLSP起動
- server固有の正しい構文data
- type assignabilityやEventValue resolution
- 新しいfunction解析

## テスト

```sh
cargo test -p skripthub --locked
```

testはnetworkを使わず、保存済みの実responseを解析し、entity/function変換を検証します。
別の[`invalid-syntax-searcher`](../utilities/invalid-syntax-searcher/README.ja.md) utilityが、
networkを使用するcorpus調査のentry pointです。

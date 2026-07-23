# invalid-syntax-searcher

[English](README.md)

`invalid-syntax-searcher`は、このworkspaceのparserが拒否するhistorical SkriptHub patternを
探すdeveloper CLIです。

corpus調査用toolであり、language server runtimeには含まれません。通常のtestとは異なり、
実行時にSkriptHubへのnetwork accessが必要です。

## 入力

commandには生成済み`PluralRules.json`のpathが必要です。

```sh
cargo run -p invalid-syntax-searcher -- path/to/PluralRules.json
```

構文patternの解釈は、対象Skript versionとaddonが登録したoverrideに依存するため、plural
ruleを明示します。このtoolはEnglish fallbackを使用しません。

ruleの読み込み後、次のAPIへrequestします。

```text
https://skripthub.net/api/v1/addonsyntaxlist/
```

## 解析

空でない各syntax pattern行をkindによって振り分けます。

- Function entryは、flattenされたlegacy API表現に対応する
  `skripthub::function_pattern`を使用します。
- それ以外のsyntax kindは`syntax-pattern-parser`を使用します。

utilityはsyntax entryごとに最初の拒否patternを記録し、typed parser errorごとに分類します。

pattern errorのcategoryには次が含まれます。

- 閉じていないgroup、option、type、regex delimiter
- type-expressionの不正なtime state
- 不正なparse mark

function categoryには、不正なname、空name、不正argument、閉じていないparenthesis/string、
spaceを含むnameがあります。既知のtyped categoryに一致しないerrorは`unknown`へ出力します。

## 出力

結果はstdoutへ出力します。各groupにはSkriptHub documentation linkと拒否された登録patternが
含まれます。

```text
unclosed_parenthesis:
    https://skripthub.net/docs/?id=123: example (pattern
```

sourceの変更、fixture更新、parser errorの抑制は行いません。出力は、parser issue、
upstream data issue、明示的な互換caseのどれを作成すべきか判断するために使用します。

## Testとの関係

通常のparser regressionは、`syntax-pattern-parser`の決定的なcommit済みtestにしてください。
変化するremote serviceはCIの正解判定に適さないため、このutilityはcaseの発見だけに使います。

生成済みSSG corpusと`proptest`が、自動化された堅牢性検証の中心です。

## Buildとcheck

```sh
cargo check -p invalid-syntax-searcher --locked
```

legacy response modelをofflineでtestする場合:

```sh
cargo test -p skripthub --locked
```

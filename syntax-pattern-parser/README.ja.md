# syntax-pattern-parser

[English](README.md)

`syntax-pattern-parser`は、Skriptとaddonが登録した構文patternを解析します。SSG読み込み、
構文検証、将来の候補照合、legacy SkriptHub調査で利用する、span付きASTを生成します。

利用者が記述する`.sk` fileは解析しません。実sourceの位置とmacro展開の出典は
[`skript-parser`](../skript-parser/README.ja.md)が扱います。

## 対応するpattern要素

| Pattern | AST | 意味 |
| --- | --- | --- |
| `text` | `Literal` | 固定text |
| `a|b` | `Choice` | 選択branch |
| `(a|b)` | `Group` | 必須group |
| `[a]` | `Option` | 任意group |
| `<.+>` | `Regex` | inline regular expression |
| `%string%` | `TypeExpr` | Skript expressionのplaceholder |
| `:tag` | `ParseTag` | parse tag marker |
| `¦1` | `ParseMark` | numeric parse mark |
| 空branch | `Empty` | `a|`などの明示的な空選択肢 |

`|`はtop-levelを含むすべてのscopeでchoiceを作ります。`\|`はliteral pipeです。そのため
`<.+> \|\| <.+>`の`||`はchoiceになりません。

AST nodeは元patternのUTF-8 byte spanを保持します。parserはdelimiter内部のwhitespaceを
含め、入力を自動的に正規化しません。

## Type expression

`%...%`内部は`TypeExpression`として構造化されます。1つのplaceholderに`/`区切りのtype
alternativeを複数含められ、各typeは次のmodifierを持てます。

| Modifier | 意味 |
| --- | --- |
| `-` | expressionを許可しない |
| `~` | literalを許可しない |
| `*` | expressionを許可しない複数値形式 |

`@` time stateも構造化されます。不正な位置や重複状態はtyped parse errorです。

解析済みtype名は`TypePattern`と`PluralRules`を使って正規化します。plural ruleはSkriptと
addonがruntime登録するため、callerは対象SSG snapshotの`PluralRules.json`を渡す必要が
あります。

## Spanとdiagnostic

`Span`は元patternに対する半開UTF-8 byte rangeです。errorは`ParseErrorKind`とprimary spanを
持ちます。

unclosed delimiterではprimary spanをEOFのzero-width位置に置き、`related_spans`に対応する
opening delimiterを保持します。UIはこれを「ここで閉じられるはずだった」と「ここで
開いた」の2箇所として表示できます。

例:

```text
((group)
```

primary errorはEOF、related spanは内側の未閉じ`(`を指します。外側のgroupは閉じているため
errorではありません。

すべての返却spanは、空入力やmultibyte UTF-8入力でもvalid character boundaryです。

## Plural rule

`PluralRules`は、Skript runtimeが登録した単数・複数overrideを表します。

```rust
let rules = PluralRules::from_ssg_json(plural_rules_json)?;
let parsed = parse("give %items% to %player%", &rules)?;
```

`from_ssg_json`はSSGの`PluralRules.json`全体を受け取ります。`rules` fieldだけを渡すもの
ではありません。

English fallback ruleはありません。対象serverに対応する生成ruleがない状態で推測すると、
addon overrideやSkript version差によって誤ったtype名になるためです。

test corpusは次を含みます。

- Skript 2.6.4 / Minecraft 1.12.2
- modern multi-addon snapshot

## Public API

代表的な利用例:

```rust
let rules = PluralRules::from_ssg_json(plural_rules_json)?;
let parsed = parse("(send|message) %string%", &rules)?;
```

`ParseResult`はtop-level AST elementとwarningを持ちます。spanは元の文字列をindexするため、
consumerはASTと一緒に`source`も保持してください。

## テスト

```sh
cargo test -p syntax-pattern-parser --locked
```

test suiteには次が含まれます。

- grammarとdiagnosticのfocused regression test
- 生成済みSSG pattern corpus
- legacy/modern plural-rule fixture
- 任意およびdelimiter-heavyなUTF-8入力に対する`proptest`
- determinism、valid span、no-panic assertion

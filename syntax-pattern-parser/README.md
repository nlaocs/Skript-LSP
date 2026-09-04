# syntax-pattern-parser

[日本語](README.ja.md)

`syntax-pattern-parser` parses the syntax patterns registered by Skript and
addons. It produces a span-preserving AST used by SSG loading, syntax
validation, candidate matching in `skript-parser`, and legacy SkriptHub analysis.

It does not parse user `.sk` files. See [`skript-parser`](../skript-parser/)
for document source ranges and macro expansion provenance.

## Supported Pattern Elements

| Pattern form | AST representation |
| --- | --- |
| literal text | `PatternElement::Literal` |
| `a|b` or `(a|b)` | `Choice` and `Group` |
| `[optional text]` | `Option` |
| `<[0-9]+>` | `Regex` |
| `%string%` | `TypeExpr` |
| `tag:value` | `ParseTag` |
| `1¦value` | `ParseMark` |
| an empty choice branch | `Empty` |

Choices work at the top level and inside nested groups or options. Escaped
delimiters remain literal. The parser preserves delimiters in the element span
and represents empty branches with valid zero-width ranges.

## Type Expressions

A `%...%` type expression may contain:

- `/` separated type alternatives
- `-` for nullable input
- `~` to disallow literals
- `*` to disallow expressions
- `@<integer>` for Skript time state
- singular or plural type names

`PatternTypeExpr` stores normalized singular names plus a plural flag for each
alternative. `display_with` reconstructs a type expression using the active
plural rules.

## Spans and Diagnostics

`Span` is a half-open UTF-8 byte range in the original registration pattern.
Every AST element is a `Spanned<PatternElement>`.

Parse errors contain a primary span. Unclosed groups, options, type
delimiters, and regex delimiters also contain a typed related span pointing to
the opening delimiter. This lets diagnostics highlight EOF while still
showing where the construct began.

For `((group)`, the existing `)` closes the inner group: the primary error is
`8..8` (EOF), and the related opening span is `0..1` (the outer `(`).

Current fatal error kinds cover:

- unclosed group, option, type, and regex delimiters
- invalid type-expression time state
- invalid parse mark

Non-fatal compatibility concerns are returned as `ParseWarning` values in
`ParseResult`.

## Plural Rules

Pattern parsing requires a `PluralRules` value. The rules come from the SSG
snapshot for the exact Skript and addon set, not from a hardcoded English
fallback in this crate.

`PluralRules::from_json` validates and loads generated `PluralRules.json`.
Both legacy and singular-aware Skript algorithms are supported, including
addon plural overrides and registration order. The API exposes:

- `to_singular`, including whether the input was plural
- `to_plural`
- algorithm and override capability metadata
- the ordered source rules and their addon ownership

## Public API

The crate root exposes the `syntax` module:

```rust
use syntax_pattern_parser::syntax::{parse, ParseResult, PluralRules};

fn parse_pattern(
    plural_rules_json: &str,
) -> Result<ParseResult, Box<dyn std::error::Error>> {
    let rules = PluralRules::from_json(plural_rules_json)?;
    Ok(parse("(send|message) %string%", &rules)?)
}
```

`ParseResult` contains top-level AST elements and warnings. Consumers should
retain `source` alongside the AST because spans index the original string.

## Testing

```sh
cargo test -p syntax-pattern-parser --locked
```

The suite has:

- focused grammar and diagnostic regression tests
- generated SSG pattern corpora
- legacy and modern plural-rule fixtures
- `proptest` coverage for arbitrary and delimiter-heavy UTF-8 input
- determinism, valid-span, and no-panic assertions

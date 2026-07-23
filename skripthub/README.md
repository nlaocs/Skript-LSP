# skripthub

[日本語](README.ja.md)

`skripthub` is the legacy compatibility layer for syntax data returned by the
SkriptHub addon syntax list API.

New LSP syntax loading must use generated SSG snapshots through [`ssg`](../ssg/)
and [`syntaxes`](../syntaxes/). This crate remains for migration, corpus
analysis, and old data-shape compatibility. The language server must not depend
on the network availability of SkriptHub.

## Data Source

`api::fetch_data` performs a blocking GET request to:

```text
https://skripthub.net/api/v1/addonsyntaxlist/
```

The response is deserialized into `AbstractAddonSyntaxList`. API fields include
SkriptHub IDs, addon metadata, compatibility text, pattern strings, return
information, removal state, and documentation fields.

Frequently repeated response strings use `Arc<str>` interning to reduce memory
usage.

## Legacy Conversion

`AbstractAddonSyntaxListEntry::to_syntax` converts an API entry into one of the
legacy entity types:

- Event
- Condition
- Effect
- Expression
- Type
- Function
- Section
- Structure

Non-function patterns are parsed by `syntax-pattern-parser` using an explicitly
provided `PluralRules` value. They must not use a guessed or globally
hardcoded plural configuration.

The resulting `SkriptHubSyntax` trait objects preserve links back to SkriptHub.
They are separate from the normalized `syntaxes::Syntax` model used by SSG.

## Function Pattern Compatibility

SkriptHub exposes functions as flattened strings such as:

```text
example(value: string = "default")
```

`function_pattern` contains a dedicated parser for this old representation. It
validates function names, arguments, default strings, parentheses, and
separator placement.

SSG `Functions.json` already contains structured names, parameters, modifiers,
defaults, and return metadata. SSG functions must never pass through the
legacy string parser.

## Module Layout

| Module | Responsibility |
| --- | --- |
| `api` | remote response DTOs and blocking fetch helper |
| `addon_syntax_list` | legacy syntax traits and entity conversion |
| `function_pattern` | parser for flattened SkriptHub function strings |

## Appropriate Uses

Use this crate for:

- checking whether the pattern parser accepts historical SkriptHub entries
- migration comparison while removing old service dependencies
- reading saved SkriptHub response fixtures

Do not use it for:

- normal LSP startup
- server-specific syntax truth
- type assignability or EventValue resolution
- new function parsing

## Testing

```sh
cargo test -p skripthub --locked
```

The tests parse a saved real response and exercise entity/function conversion
without requiring network access. The separate
[`invalid-syntax-searcher`](../utilities/invalid-syntax-searcher/) utility is
the networked corpus-analysis entry point.

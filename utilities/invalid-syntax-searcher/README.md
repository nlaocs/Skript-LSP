# invalid-syntax-searcher

[日本語](README.ja.md)

`invalid-syntax-searcher` is a developer CLI for finding historical SkriptHub
patterns that are rejected by this workspace's parsers.

It is a corpus-analysis tool, not part of the language server runtime. Unlike
normal tests, running it requires network access to SkriptHub.

## Input

The command requires a generated `PluralRules.json` path:

```sh
cargo run -p invalid-syntax-searcher -- path/to/PluralRules.json
```

Plural rules are explicit because syntax pattern interpretation depends on the
Skript version and addon overrides that registered them. The tool does not use
an English fallback.

After loading the rules, it fetches:

```text
https://skripthub.net/api/v1/addonsyntaxlist/
```

## Parsing

Each non-empty syntax pattern line is routed by kind:

- Function entries use `skripthub::function_pattern`, matching the flattened
  legacy API representation.
- Every other syntax kind uses `syntax-pattern-parser`.

The utility records the first rejected pattern for each syntax entry, then
groups entries by typed parser error.

Pattern categories currently include:

- unclosed group, option, type, or regex delimiter
- incorrect type-expression time state
- invalid parse mark

Function categories include invalid names, empty names, invalid arguments,
unclosed parentheses or strings, and names containing spaces. Errors that do
not match a known typed category are printed under `unknown`.

## Output

Results are written to stdout. Each group contains SkriptHub documentation
links and the rejected registration pattern:

```text
unclosed_parenthesis:
    https://skripthub.net/docs/?id=123: example (pattern
```

The tool does not modify source, update fixtures, or suppress parser errors.
Its output is intended for deciding whether a parser issue, upstream data
issue, or explicit compatibility case should be created.

## Relationship to Tests

Normal parser regressions should become deterministic checked-in tests in
`syntax-pattern-parser`. This utility is useful for discovering those cases,
but a changing remote service is unsuitable as a CI oracle.

Generated SSG corpora and `proptest` remain the primary automated robustness
coverage.

## Build and Check

```sh
cargo check -p invalid-syntax-searcher --locked
```

For an offline test of the legacy response model, use:

```sh
cargo test -p skripthub --locked
```

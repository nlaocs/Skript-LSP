# skript-parser

[日本語](README.ja.md)

`skript-parser` owns source-level primitives for parsing actual `.sk`
documents. Its current implementation focuses on UTF-8 ranges, virtual source
mapping, validated Text edit application, macro expansion provenance, and
syntax contexts.

Despite the crate name, it does not yet tokenize Skript, build indentation
trees, match registered syntax, or produce a complete Skript AST. Those stages
will be added on top of the invariants defined here.

## Why This Is Separate from syntax-pattern-parser

`syntax-pattern-parser` parses a registration pattern such as:

```text
send %string% to %players%
```

`skript-parser` handles source written by a user, for example:

```skript
send "hello" to player
```

Registration patterns describe what may match. Document parsing tracks what
the user wrote, where it came from, and how preprocessing changed it.

## Public Model

### Text ranges

`TextRange` is a half-open `start..end` UTF-8 byte range. It can validate
character boundaries, safely slice source, test containment and intersection,
and represent zero-width cursor or EOF locations.

All offsets in this crate are bytes, not Unicode scalar counts and not UTF-16
LSP positions. Conversion to LSP positions belongs at the protocol boundary.

### Source maps

A `SourceMap` contains non-overlapping virtual ranges and their original
origins. `OriginKind` records how the mapping was produced:

- `Exact`: virtual text corresponds directly to original text
- `Replaced`: generated or transformed text replaces an original range
- `Anchored`: generated text is attached to an original zero-width location

`MappedSource` owns original text, current virtual text, a validated SourceMap,
and an ExpansionGraph. `map_range` returns a `MappedSpan` with every relevant
origin and preserves generated-source information.

### Text edits

`TextEdit` replaces a half-open byte range and may attach generated text to an
explicit original anchor. `MappedSource::apply_text_edits` sorts a macro's
edits, validates the whole batch, applies it atomically, and returns a new
source with a composed SourceMap and ExpansionGraph entry.

Unchanged text preserves its existing origin. Replacements use `Replaced`,
zero-width insertions use `Anchored`, and sequential macros retain parent
expansion links. An empty edit list is a successful no-op, while a zero-width
edit with an empty replacement is rejected.

`TextEditApplication::generated_bytes` counts replacement bytes introduced by
the batch; the WASM host uses it to enforce pipeline quotas.

### Expansion provenance

`Expansion` records:

- a stable `ExpansionId`
- Text, Tree, or AST expansion kind
- owning WASM component and hook
- call site and optional definition site
- `SyntaxContextId` used for macro hygiene

`ExpansionGraph` validates duplicate IDs, blank owners, unknown references, and
cycles. Its `backtrace` returns expansions from the innermost call to the root.

## Invariants

Constructors reject:

- ranges outside source length or inside a UTF-8 code point
- overlapping edits, ambiguous same-position insertions, and invalid anchors
- zero-width edits with an empty replacement
- overlapping or incomplete SourceMap segments
- mappings whose original ranges do not fit the original source
- unknown expansion references
- cyclic expansion chains

These checks keep diagnostics valid after nested preprocessing. Parser stages
should carry `MappedSpan` rather than reconstructing locations after the fact.

## Source Layout

| Module | Responsibility |
| --- | --- |
| `text` | `TextRange` and UTF-8 range operations |
| `source_map` | origins, segments, mapped source, and mapped spans |
| `expansion` | expansion graph, component/hook ownership, and syntax contexts |

All public items are re-exported from the crate root.

## Testing

```sh
cargo test -p skript-parser --locked
```

The test suite includes multibyte UTF-8 mapping, generated text, replacement
ranges, empty sources, chained expansion backtraces, explicit anchors, invalid
segment layouts, and property tests for identity mappings and arbitrary UTF-8
Text edit application.

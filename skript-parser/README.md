# skript-parser

[日本語](README.ja.md)

`skript-parser` owns source-level primitives for parsing actual `.sk`
documents. Its current implementation focuses on UTF-8 ranges, virtual source
mapping, validated Text edit application, macro expansion provenance, syntax
contexts, and a lossless indentation-based RawTree.

The crate now splits preprocessed source into physical lines and builds its
comment/indentation structure. It does not yet match registered syntax or
produce a complete Skript AST. Those stages will be added on top of the
invariants defined here.

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
and an ExpansionGraph. A generated segment may retain multiple original
origins when it combines text from multiple source ranges or prior expansions.
Exact segments remain one-to-one. `map_range` returns a `MappedSpan` with every
relevant origin and preserves generated-source information.

### Text edits

`TextEdit` replaces a half-open byte range and may attach generated text to an
explicit original anchor. `MappedSource::apply_text_edits` sorts a macro's
edits, validates the whole batch, applies it atomically, and returns a new
source with a composed SourceMap and ExpansionGraph entry.

Unchanged text preserves its existing origin. Replacements use `Replaced`,
zero-width insertions use `Anchored`, and sequential macros retain parent
expansion links. An empty edit list is a successful no-op, while a zero-width
edit with an empty replacement is rejected. A multi-edit batch records the
origins of every edit as call sites. Replacing text that already combines
several origins carries all of them forward instead of selecting only the
first one.

`TextEditApplication::generated_bytes` counts replacement bytes introduced by
the batch; the WASM host uses it to enforce pipeline quotas.

### Expansion provenance

`Expansion` records:

- a stable `ExpansionId`
- Text, Tree, or AST expansion kind
- owning WASM component and hook
- one or more call sites and an optional definition site
- `SyntaxContextId` used for macro hygiene

`ExpansionGraph` validates duplicate IDs, blank owners, unknown references, and
cycles across the resulting directed acyclic graph. `backtrace` returns the
primary path from the innermost call to the root for simple consumers, while
`backtraces` returns every distinct parent-expansion path.

### Lossless RawTree

`parse_raw_tree` converts a `MappedSource` into an arena-backed `RawTree`.
Nodes use source-order `RawNodeId` values and are classified as:

- `Blank`
- `Comment`
- `Simple`
- `Section`
- `Invalid`

Every node retains its physical `RawLine`, raw text, decoded Skript text,
indentation, trailing whitespace, comment, and line-ending trivia. All ranges
are `MappedSpan` values, so lines produced by Text macros keep their original
source origins and expansion provenance.

Section nodes expose separate spans for the header, body, and complete
subtree. An empty body uses a zero-width span immediately after the header
line. The tree also preserves parent/child relationships and the detected
space or tab indentation unit.

Comment splitting follows Skript's `Node.splitLine` behavior:

- `##` becomes one literal `#` outside quoted strings
- `#` inside a quoted string is not a comment
- variable and `%...%` state transitions follow Skript's state machine
- only a trimmed line equal to `###` opens or closes a block comment
- blank and comment lines remain attached to the currently open Section

Unlike Skript's runtime loader, the parser must remain useful while a document
is being edited. Mixed indentation, partial indentation units, and excessive
indentation therefore produce `Invalid` nodes and diagnostics without dropping
later lines. Empty Sections produce warnings, and unclosed block comments point
to both the opening marker and EOF.

The WIT conversion and recursive Tree macro application are intentionally left
to the following Tree macro pipeline stage.

## Invariants

Constructors reject:

- ranges outside source length or inside a UTF-8 code point
- overlapping edits, ambiguous same-position insertions, and invalid anchors
- zero-width edits with an empty replacement
- overlapping or incomplete SourceMap segments
- mappings whose original ranges do not fit the original source
- SourceMap segments with no origins
- multi-origin segments containing an `Exact` origin
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
| `raw_tree` | physical lines, comment splitting, indentation recovery, and RawTree |

All public items are re-exported from the crate root.

## Testing

```sh
cargo test -p skript-parser --locked
```

The test suite includes multibyte UTF-8 mapping, generated text, replacement
ranges, empty sources, chained and multi-origin expansion backtraces, explicit
anchors, invalid segment layouts, and property tests for identity mappings and
arbitrary UTF-8 Text edit application. RawTree tests cover Skript's comment
cases, LF/CRLF/no-final-newline inputs, spaces and tabs, nested Sections,
recoverable invalid indentation, empty Sections, block comments, macro origins,
and lossless arbitrary UTF-8 input.

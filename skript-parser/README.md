# skript-parser

[日本語](README.ja.md)

`skript-parser` owns source mapping, lossless RawTree construction, registered
syntax matching, and recursive source-level syntax trees for `.sk` documents.
It parses Expressions, Conditions, Effects, Event headers, Sections, and
top-level Structures with declarative EntryValidator bodies. It also owns the
transactional registry consumed by document-defined Function calls.

These stages are library APIs. Callers supply a Catalog and an
`ExpressionParseEnvironment`; `parser-wasm` provides the real WASM-backed
environment and CoreLibrary semantics. The crate does not run Skript code,
provide LSP/HTTP transport, or guarantee complete semantics for every addon.
Unknown or rejected input remains available as partial trees and diagnostics.

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

The `Ast` expansion kind and syntax-context IDs provide provenance primitives;
they do not imply an implemented AST macro execution pipeline or complete
hygienic name resolution.

### Lossless RawTree

`parse_raw_tree` converts a `MappedSource` into an arena-backed `RawTree`.
Callers must pass `RawTreeOptions`, normally created with
`RawTreeOptions::for_skript_version`, so version-dependent lexical behavior is
selected explicitly. Nodes use source-order `RawNodeId` values and are
classified as:

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

Triple-hash multiline comments were introduced in
[Skript 2.9](https://github.com/SkriptLang/Skript/commit/adac6e1984b54924583ce13dea6eb319bc61982c).
`RawTreeOptions::for_skript_version(2, 8)` therefore treats each `###` line as
an ordinary single-line comment, while version 2.9 and later enable block
comment state. Triple hashes in the middle of a line never toggle that state and
continue to follow the ordinary `##` escape and `#` line-comment rules.

Unlike Skript's runtime loader, the parser must remain useful while a document
is being edited. Mixed indentation, partial indentation units, and excessive
indentation therefore produce `Invalid` nodes and diagnostics without dropping
later lines. Empty Sections produce warnings, and unclosed block comments point
to both the opening marker and EOF.

`apply_tree_edit` validates generated local-ID fragments and applies targeted
node or Section-body replacements without mutating the input tree. It allocates
new RawNode IDs, registers a Tree expansion, assigns a generated syntax
context, and maps every generated span back to the replaced node's call-site.
The `parser-wasm` host owns WIT conversion and recursive pre-order dispatch.

## Registered Pattern Matching

`match_pattern_candidates` matches a `MatchInput` against parsed registration
patterns. It supports literals, choices, groups, optional elements, empty
branches, regular expressions, type expressions, parse tags, and XOR parse
marks. A candidate is successful only when it consumes the complete
Java-whitespace-trimmed input.

Regex captures include numbered groups and UTF-8 byte-accurate `MatchSpan`
values. Type expressions are delegated to `TypeExpressionResolver`, which may
run the recursive Expression parser and return one or more typed resolutions at
legal Skript split points. Local ranges remain relative to the matched line,
while every result also carries its editor-facing `MappedSpan` provenance.

For mixed syntax kinds, candidates first follow the caller's first-seen kind
order, preserving separate parser phases. Within each kind, resolved registry
order comes first when present, followed by numeric priority, registration
order, and declaration order.
Patterns retain their registration index. Results contain the selected match,
all later alternatives, or a farthest-failure diagnostic when nothing matches.

For SSG-backed data, `catalog_pattern_candidates` adapts static Catalog
registrations and `snapshot_pattern_candidates` adapts a frozen mixed
static/dynamic snapshot without reparsing its Pattern ASTs. The latter carries
the registry's topologically resolved order into the matcher. Type and Function
data keep their dedicated parser paths.

`PatternMatchHooks` observes or overrides definition, registration, pattern,
and nested element scopes before and after matching. Element paths include both
sequence and choice-branch positions. Configurable state, backtrack, regex
execution, evaluated-byte, and regex-engine limits bound ambiguous or hostile
patterns. Transition memoization avoids repeating deterministic literal and
regex work. `RankedFailures` retains candidate identities and pattern sources;
`FailureTrace` connects enclosing syntax/captures to the innermost failure.
Bounded diagnostic recovery can retain several failed captures without treating
the incomplete candidate as accepted. It is not unrestricted error recovery.

## Recursive Expression Parsing

`parse_expression` combines SSG `Catalog` registrations with leaf parsers from
an `ExpressionParseEnvironment`. `parse_expression_with_snapshot` additionally
uses a frozen dynamic syntax snapshot, preserving resolved before/after order,
dynamic return metadata, and registry revision in memo keys.

Each top-level `ExpressionExpectedType` retains both its Java class and singular/plural requirement. The parser preserves `%type%` alternatives, plurality, nullable markers,
literal/expression flags, and time state. It filters return classes through the
Catalog hierarchy and registered converters, rejects Multiple-only results for singular placeholders,
and recursively attaches typed captures as child `ExpressionNode` values.
Variables, literals, functions, and custom addon parsers use the same mapped
spans as registered expressions.

Complete outer parentheses, arithmetic, and Expression lists have dedicated
node forms. List splitting respects quotes, variables, and nested parentheses;
a grouped list in a Function call remains one argument. Registered semantic
handlers can refine return classes, multiplicity, metadata, and parsed captures.
Regex captures requiring a parser route are not silently treated as fully
understood syntax when that route is unavailable.

Left-recursive forms such as `%strings% in upper case` use seed-and-grow parsing.
Leading and trailing literal constraints are extracted conservatively from the
Pattern AST before matching, while depth, candidate, matcher, and memo limits
bound hostile recursion. Memo keys include source range, expected type/context,
StateStore revision, and dynamic registry revision.

## Function Call Parsing

Registered Functions use a dedicated call parser before ordinary registered
Expression matching. It recognizes Skript's Unicode Function names, skips
quoted strings, variables, and nested parentheses while finding argument
boundaries, and resolves signatures from the SSG `Functions.json` catalog and
the environment's document registry. `FunctionVersionPolicy` selects version
boundaries for local Functions, overloads, named arguments, and return syntax;
the WASM host derives the policy from the runtime profile. Exact signatures are
attempted before single-plural-parameter signatures.

Each argument is recursively parsed as the parameter's Java component type.
Named arguments are rebound to their declared parameter, optional parameters
remain explicit omitted bindings, and a single plural parameter can retain all
comma-separated child Expressions. `FunctionCall` keeps the Function name,
definition/registration IDs, and parameter-to-child ranges; its parent
`ExpressionNode` keeps return type and multiplicity.

`ExpressionParseEnvironment::lookup_functions` may prepend definitions visible
in the current document or project. A definition with the same parameter shape
shadows the catalog global. The host already uses this for `StructFunction`
declarations: all accepted headers register before bodies are parsed.
`FunctionRegistryTransaction` validates declarations and supports rollback;
`FunctionRegistrySnapshot` retains signatures for subsequent lookup. This does
not implement a project-wide cross-file symbol index or variable type-flow analysis.

## Effect Parsing

`parse_effect` consumes one lossless `RawNodeKind::Simple` node. It matches the
node's exact code span against static SSG EffectSection registrations first,
then ordinary Effects, and returns the
selected `EffectCandidate`, deterministic alternatives, or an
`UnknownEffectNode`. The unknown form retains the original `RawNodeId`, exact
code text, mapped source span, and ranked candidate `FailureTrace` values so later
LSP recovery does not discard an unrecognized line.

`parse_effect_with_snapshot` combines static and dynamic registrations in the
frozen registry order. Dynamic candidates retain their opaque handler and
metadata. Typed captures share an internal `ExpressionSession`, attaching child
`ExpressionNode` values while reusing recursion limits, memoization, matcher
hooks, and candidate transaction boundaries. Patterns without placeholders do
not instantiate an Expression path.

Only static Sections marked `effectSection` participate in one-line Effect
parsing. Their source identity remains `MatchSyntaxKind::Section`, including
Section definition/registration IDs; ordinary Sections are excluded. In the
WASM host this is a Section target in the Effect phase, not a Section body
lifecycle. General dynamic Section registrations do not imply EffectSection
support. This entry point does not add a standalone void Function-call statement path.

## Condition Parsing

`parse_condition` matches static SSG Conditions in registration order, while
`parse_condition_with_snapshot` also includes frozen dynamic registrations.
Both trim Java whitespace and repeatedly remove complete outer parentheses,
matching Skript's `Condition.parse` behavior. Typed captures reuse the current
`ExpressionSession`; the selected `ConditionNode` therefore owns its parsed
child Expressions. Unknown input retains its mapped span and farthest pattern
failure for later diagnostics.

## Event Parsing

`parse_event` matches an Event header without assuming which Structure owns it.
It preserves the selected SSG identities, event class, reference Bukkit event
classes, cancellability, regex captures, and addon metadata. Registered capture
bindings can therefore use `host.event` from a Structure hook while the native
parser remains independent of `StructEvent` and other Skript implementation
classes.

## Section Parsing

`parse_section` consumes one `RawNodeKind::Section` and recursively claims its
children as nested Sections or Effects. Header candidates combine ordinary
Sections, EffectSections, and Expression registrations marked as
SectionExpressions. The selected node retains all three metadata flags,
semantic Condition captures, child Expressions, and dynamic handler metadata.

The body mode is either `Trigger` (nested Sections and Effect lines) or
`Conditions` (Condition lines). There is no general Condition-statement fallback
for an unmatched Effect line in `Trigger` mode.

`ExpressionParseEnvironment::enter_section_children` may derive a child
context before the body is parsed; `exit_section_children` observes that same
context after the body finishes. Hook-approved ordinary context updates may
propagate to following siblings, while the parser-owned Section stack is
restored to the parent scope. Unknown
headers, unclaimed body lines, and multiple successful claims remain available
as partial AST nodes and `SectionDiagnostic` values instead of aborting the
whole subtree.

The child `ExpressionParseContext` also contains a parser-owned
`section_stack`, ordered outermost to innermost. Each immutable frame records a
parse-local scope ID and parent, the Section's definition/registration/pattern
identity, addon, implementation class, Section flags, semantic captures, and
metadata. Effects, Conditions, Expressions, and Section lifecycle hooks all
observe this same stack. A rejected candidate is rolled back, and leaving a
Section restores its parent stack before parsing a sibling.

`SectionParserConfig::root_lifecycle` defaults to `Complete`. Callers that need
to analyze later statements as if they were still inside the requested root
may select `RetainBody`; nested Sections still complete normally, but the root
exit hook is deferred and its body context remains active. Such callers must
restore both parser context and transactional addon state when leaving it.

## Structure Parsing

`parse_structures` performs Skript's two-pass top-level flow: it first matches
and enters every Structure header, then parses the selected bodies. Native code
owns registration order, `NodeType`, lossless RawTree traversal, and declarative
`EntryValidator` behavior. Supported entry forms include literal, Expression,
Trigger, Container, Section, defaults, repeated entries, custom separators, and
nested validators. Unknown addon `EntryData` is retained with its raw source and
a diagnostic instead of being discarded.

`ExpressionParseEnvironment::enter_structure` and `exit_structure` form the
extension boundary. An environment may reject a header, derive a body context,
select `None`, `Raw`, `Entries`, or `Trigger` parsing, attach metadata, and
inspect the parsed body. Skript-specific Structure semantics such as
`StructEvent`, `StructFunction`, and `StructCommand` belong in WASM components,
not this native module.

`StructureParserConfig::headers_only` stops after accepted header and
`enter_structure` hooks. It deliberately skips body parsing and
`exit_structure`, allowing callers such as an Event-context selector to retain
the entered Structure transaction while they parse statements in that context.

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
| `tree_edit` | validated node/body edits and generated-tree provenance |
| `raw_tree` | physical lines, comment splitting, indentation recovery, and RawTree |
| `pattern_match` | registered-pattern matching, captures, ranking, hooks, and limits |
| `failure` | ranked candidate failures, nested causes, and semantic diagnostic spans |
| `expression` | recursive Expression AST, type filtering, left recursion, memoization, and leaf parser integration |
| `arithmetic` | operator precedence and catalog-backed arithmetic result types |
| `expression_list` | top-level list splitting and conjunction semantics |
| `function` | registered/document Function calls, named/optional/list arguments, and overloads |
| `function_registry` | declaration validation, version policy, transactions, and frozen document lookup |
| `effect` | Simple-node Effect candidates, dynamic metadata, nested Expressions, and unknown recovery |
| `condition` | registration-order Condition matching, outer-parenthesis handling, and nested Expressions |
| `event` | Event header matching, reference event classes, cancellability, and semantic captures |
| `section` | recursive Section/Effect bodies, scoped contexts, semantic captures, and partial recovery |
| `structure` | top-level two-pass Structure parsing, NodeType, EntryValidator, and WASM lifecycle hooks |
| `catalog_match` | adapters from static Catalogs and frozen dynamic snapshots |

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
and lossless arbitrary UTF-8 input. Pattern matcher tests cover structural elements, Skript literal and split rules, UTF-8 captures, tags, marks, ranking, hooks, limits, generated-source mapping, SSG pattern corpora, and arbitrary UTF-8 property cases. Expression tests cover static and dynamic registrations, Core-style leaves, expected-type and multiplicity filtering, nested and left recursion, deterministic ordering, Function calls and document shadowing, and the full multi-addon Catalog. Effect tests use real schema 3 DummyAddon registrations for plain, typed, dynamic, and unknown lines. Structure tests cover NodeType filtering, defaults, repeated and nested entries, custom separators, unknown addon EntryData, and body-end diagnostics.

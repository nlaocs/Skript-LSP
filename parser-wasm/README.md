# Parser WASM ABI

[日本語](README.ja.md)

`parser-wasm` owns both sides of the WebAssembly Component Model boundary
between the Rust parser, mandatory CoreLibrary, and parser addon components.
It contains the shared ABI model plus the optional native Wasmtime host.

It does not parse Skript patterns or `.sk` source. It transports typed parser
data, orders hook calls, enforces limits, and makes accepted component side
effects transactional.

## Feature Modes

The default `host` feature enables Wasmtime, persistence, URL handling, and the
`syntaxes` integration used by the LSP:

```toml
parser-wasm = { path = "../parser-wasm" }
```

WASM guest crates should disable default features:

```toml
parser-wasm = { path = "../parser-wasm", default-features = false }
```

That mode exposes ABI versions, capability IDs, and compatibility validation
without linking the native host.

## WIT Contract

The WIT package is `nlaocs:skript-parser-addon@0.13.0`. Its
`parser-addon` world imports host services and exports guest implementations.

Guest exports:

- `addon`: static manifest and host-profile negotiation
- `hooks`: parser-stage observation, transformation, and override
- `text-macro`: edits over virtual UTF-8 source text
- `tree-macro`: targeted edits over the indentation-based lossless RawTree
- `ast-macro`: replacement of the parsed AST arena

Host imports:

- `state-store`: scoped key/value storage with compare-and-swap and prefix scan
- `dynamic-syntax-registry`: add, override, and remove syntax definitions

All parser payloads are WIT records and variants. JSON is not part of the ABI.
RawTree and AST values use node-ID arenas so their payloads remain non-recursive
Component Model values.

## Compatibility

Every manifest exposes a component ID and component version for diagnostics.

The package version identifies the WIT shape. Text edit anchors changed the
package from 0.1.0 to 0.2.0; the lossless RawTree and targeted TreeEdit model
changed it to 0.3.0; typed pattern-matching scopes, paths, status, and spans
changed it to 0.4.0; Expression leaf requests and candidates changed it to
0.5.0; typed Effect lifecycle candidates and failures changed it to 0.6.0;
post-match registered Expression resolution changed it to 0.7.0; declaring
the registered Expression classes handled by a component changed it to 0.8.0;
generic registered syntax handlers, semantic Condition/Effect captures, and
the Section lifecycle changed it to 0.9.0; registered property axis metadata
changed it to 0.10.0; finite type-literal candidates changed it to 0.11.0; structured literal metadata changed it to 0.12.0; SSG supplier metadata in expression type options changed it to 0.13.0. The manifest's current
`abi` value is 1.12 and is a runtime handshake that
requires an exact `major.minor` match.

Capabilities use stable string IDs and independent integer versions instead of
a closed enum. This allows a newer component to describe a capability to an
older host without failing while lifting its manifest.

- A missing or older required capability rejects initialization.
- A missing or older optional capability is ignored.
- Duplicate or blank capability IDs are invalid.
- Both the host and guest use the same negotiation rule. The host validates the
  component manifest, then the guest validates the host profile in
  `addon.initialize`.

The host advertises and executes Text and Tree macros. The AST macro capability
remains contract-only and is not advertised yet.

## Expression Parsing

`ParserHost::parse_expression_in_parse` freezes the document's dynamic syntax
registry and runs `skript-parser` with one transactional WASM environment.
Expression subscriptions target `ParseStage` in the `Expression` phase with
`Transform` mode and the `parser.expression` capability. Their typed payload
contains the complete virtual source, remaining range and mapped span, expected
Java types and plurality, legal split points, literal/expression flags, time
state, depth, matching finite type-literal options, and accumulated leaf candidates. The host
builds the finite literal index once from SSG type metadata and aliases, then sends only options
matching the current legal split points to avoid copying the complete registry into WASM.

CoreLibrary and addons may append Variable, Literal, Function, or Custom leaf
candidates. After a registered Expression and its typed children match,
the host sends a second payload containing parse tags, children, known return
types, and applicable property metadata, including supported component axes.
CoreLibrary or an addon may resolve
the effective Java return type and multiplicity, or reject the candidate.
Components declare the syntax kind, Java class suffix, and meaning of regex
captures in `registered-syntax-handlers`. The native parser considers an
otherwise incompatible dynamic registration only when an enabled component
declares it, avoiding broad unresolved registrations during every type search.
The host validates immutable request fields, UTF-8 ranges, parser
IDs, return type/multiplicity, and metadata before the native parser ranks the
results with registered static and dynamic expressions. A leaf set eliminated
by native range, type, or multiplicity validation restores its dispatch
savepoint and effects. Every recursive matcher invocation owns a nested
candidate frame, so child selection cannot overwrite its parent's selected
state. No-match and parser failure restore the entry StateStore savepoint. The
parse-overlay revision is part of native memo keys and is itself restored by
candidate rollback.

## Effect Parsing

`ParserHost::parse_effect_in_parse` takes a lossless `RawNodeKind::Simple`
node and matches its exact `code_span`, excluding indentation and trailing
comments. Static SSG Effects and frozen dynamic registrations share the same
resolved ordering. `%type%` captures re-enter the recursive Expression session,
so selected Effect and child Expression state use one transaction hierarchy.

Effect subscriptions declare `parser.effect` and run in the `Effect` phase.
A category hook runs before native matching, then the selected exact
registration, or the Effect category for an unknown node, runs afterward. The
typed payload retains definition and registration IDs, element class, pattern
index, capture spans, parse tags, XOR marks, parsed Condition or nested Effect
captures, dynamic handler metadata, alternatives, and the farthest failure. A replacement may update only the selected handler and
metadata; immutable registration identity, captures, alternatives, and spans
are validated by the host.

Unknown, rejected, invalid-output, and failed Effect pipelines restore their
entry StateStore savepoint. The returned unknown node keeps its exact source,
mapped span, and farthest failure. Reject diagnostics remain observable even
though the rejecting hook's state is rolled back.

## Condition and Section Parsing

`ParserHost::parse_condition_in_parse` applies Skript's registration-order
Condition matching, including repeated removal of complete outer parentheses.
It shares the recursive Expression session, so Condition patterns may contain
typed Expressions and can themselves be semantic regex captures of registered
Expressions, Effects, or Sections.

`ParserHost::parse_section_in_parse` consumes a lossless
`RawNodeKind::Section`. Its header is matched against normal Sections,
EffectSections, SectionExpressions, and frozen dynamic Sections. The selected
candidate preserves those three metadata flags. Before and after recursively
parsing its child Sections and Effects, the host dispatches the exact
registration in the `Section` phase with `parser.section`. Enter-phase context
updates apply only to that body and its descendants. Unknown or multiply
claimed body nodes remain in the partial tree with diagnostics.

CoreLibrary declares semantic handlers for Skript's conditional and while
Sections, `ExprWhether`, `ExprTernary`, and `EffDoIf`. Addons can use the same
manifest declarations for their own raw, Condition, or nested Effect captures.
## Text Macros

A Text macro subscribes to `ParseStage` during `Preprocess` with `Transform`
mode. Matching subscriptions run in deterministic priority order and receive
the current virtual UTF-8 source.

Each output contains byte-range edits. The host sorts edits, rejects overlap,
invalid UTF-8 boundaries, ambiguous insertions, and invalid anchors, then
applies the complete output atomically. Replacement text maps to its replaced
call-site by default; an optional `anchor` maps generated text to an explicit
zero-width location. Sequential macro outputs compose the SourceMap and append
parent-linked Text entries to the ExpansionGraph. Multi-edit outputs and
replacements spanning several earlier mappings preserve every origin and form
an expansion DAG rather than discarding all but the first call-site.

Diagnostics in effects, rejections, and addon errors, and parse-request spans
are interpreted against the macro's input virtual source. The host ignores
guest-provided origins and rebuilds both primary and related spans from its
current MappedSource. This maps effects from later macros through every earlier
expansion to the original document. An invalid diagnostic or parse-request byte
range makes that call invalid and rolls back its text and state changes.

A whole-pipeline rejection marks every call as unaccepted, clears call
expansion IDs, and removes diagnostic expansion references that are absent
from the restored source graph. Context updates and parse requests are
discarded with the rejected transformation. A rejection diagnostic's
`virtual-range` still identifies the rejecting macro's input snapshot, while
its rebuilt origins identify locations in the original document; consumers
should use those origins for editor diagnostics. Returned metadata therefore
cannot refer to an expansion that was rolled back.

Every call receives a StateStore invocation transaction. Addon errors, traps,
invalid edits, and rejected pipelines discard the corresponding text and state
changes. Successful calls expose their read/write set so future incremental
parsing can track state dependencies.

`HostConfig` limits expansion count, generated replacement bytes, and total
virtual source bytes. Exceeding a pipeline-wide quota restores the original
source and StateStore savepoint.

## Tree Macros

A Tree macro subscribes to `ParseStage` during the `Tree` phase with
`Transform` mode. The host walks the lossless RawTree in pre-order. Every call
receives the current complete tree, the target node ID, and generated-node
depth, including raw lines, trivia, invalid-node reasons, diagnostics,
indentation metadata, spans, and syntax contexts.

A `TreeEdit` targets the current node implicitly. It can replace that node with
zero, one, or many generated nodes, replace only a Section body, or attach the
original Section children to a generated Section before or after its generated
children. Generated fragments use local IDs; the host validates uniqueness,
reachability, acyclicity, node kinds, text, and child relationships, then owns
allocation of final RawNode IDs, ExpansionIds, call-site spans, and
SyntaxContextIds.

Generated roots and generated Section children re-enter the same Tree macro
stage. Structural nesting and macro re-entry are limited independently:
`max_raw_tree_depth` defaults to 256, while
`max_tree_macro_expansion_depth` defaults to 64. Total node, hook-call, and
output-byte quotas provide separate pipeline bounds. The host also detects
direct and indirect cycles using macro identity, input origin, and subtree
content. A cycle preserves the current node and produces a component failure
plus a `tree-macro-cycle` diagnostic.

Each candidate runs in a StateStore invocation transaction. TreeEdit
validation and state adoption are atomic: addon errors, traps, invalid edits,
and cycles preserve the current node and roll back that candidate's writes. A
typed rejection or a pipeline quota error restores the original tree, source
provenance, and parse StateStore savepoint. Successful edits append Tree
entries to the ExpansionGraph, so recursively generated nodes retain complete
call-site backtraces.

## Pattern Matching Hooks

`ParserHost::match_patterns_in_parse` runs the native matcher with the same
parse transaction used by other parser stages. `MatchingPayload` exposes the
input and optional pattern, definition and registration IDs, pattern index,
nested element/branch path, pattern-source range, local input range,
editor-facing mapped span, scope, timing, status, and failure reason.

Matching hooks run before and after definition, registration, pattern, and
element scopes. A handled override must return `matched` or `failed`; a matched
element may consume a validated prefix, while broad definition, registration,
and pattern matches must consume the complete trimmed input. Identity and
provenance fields are immutable across hook replacement.

Each syntax candidate starts from the same parse StateStore savepoint. Failed
candidates and non-selected alternatives are rolled back. Only writes made by
the selected candidate remain in the parse transaction, while hook calls and
component failures stay available for diagnostics and tracing.
## Hook rules

A subscription selects a target, phase, signed priority, and mode.

- `observe` reads a payload but must not replace it.
- `transform` may return a replacement payload for later hooks.
- `override` handles the target instead of its normal implementation.

The host validates mode-specific behavior, payload variants, subscriptions, and
capabilities when components are registered. Runtime limits and trap handling
belong to the Wasmtime host implementation.

Subscription ordering is deterministic:

1. exact registration targets before syntax-kind targets before parse-stage
   targets
2. signed subscription priority
3. component load order
4. declaration order inside the component manifest

A handled override stops later matching hooks. Addon errors are reported as
component failures. Traps, timeouts, fuel exhaustion, and resource-limit
violations disable the component.

## StateStore

StateStore is a host import available to hooks and macro calls.

| Scope | Lifetime |
| --- | --- |
| `invocation` | One component call |
| `parse` | One parse transaction |
| `document` | Committed document revisions |
| `project` | Documents and addons in one project |
| `persistent-project` | Project state stored across LSP restarts |

Namespaces are private to one component or shared through an explicit schema
declaration. Shared declarations specify schema ID, schema version, readers,
and writers. Values use a declared raw, CBOR, or JSON encoding, but the host
does not interpret their bytes.

Each component call receives an invocation overlay. Rejected, trapped, or
invalid calls roll it back. Accepted calls merge into the parse overlay. A
parse commits only when its document revision is still current and no project
revision conflict occurred. Persistent project state uses `redb` below the OS
application data directory, partitioned by canonical project URI.

## Dynamic Syntax

When `HostConfig::syntax_catalog` contains an SSG-backed
`Arc<syntaxes::Catalog>`, the host advertises `parser.dynamic-syntax`.
Components may then:

- register new syntax with namespaced component/local IDs
- override static syntax by definition ID or registration ID
- add ordering constraints against static or dynamic syntax
- remove their own dynamic entries

Updates are allowed during component initialization and Document/Preprocess
prepass. The registry freezes before later parser phases. Its immutable
snapshot combines static and dynamic candidates in deterministic topological
order. Rejection and host failure roll dynamic updates back alongside
StateStore updates. Unloading a component removes its entries from future
snapshots without mutating already frozen document snapshots.

The capability is intentionally unavailable when no Catalog is connected.

## Native Host API

The main entry points are:

- `ParserHost::new`: instantiate the mandatory CoreLibrary
- `load_addon` and `unload_addon`: manage component lifecycles
- `begin_parse`: create a multi-phase parse transaction
- `dispatch_in_parse`: invoke matching hook subscriptions
- `expand_text_in_parse`: run Text macros in an existing parse transaction
- `expand_text`: convenience API for a one-pipeline parse transaction
- `expand_tree_in_parse`: recursively run Tree macros in an existing parse transaction
- `expand_tree`: convenience API for a one-tree-pipeline parse transaction
- `dynamic_syntax_snapshot`: freeze and retrieve ranked syntax candidates
- `match_patterns_in_parse`: match ranked candidates with transactional WASM hooks
- `parse_expression_in_parse`: parse a typed recursive Expression
- `parse_condition_in_parse`: parse a Condition in registration order
- `parse_effect_in_parse`: parse one simple RawTree node as an Effect
- `parse_section_in_parse`: parse one Section and recursively claim its body
- `dispatch`: convenience API for a one-dispatch transaction

`HostConfig` controls call fuel, epoch timeout, Wasmtime memory/table/instance
limits, dispatch, Text macro, and Tree macro quotas, StateStore configuration, and the
optional syntax Catalog.

## Source Layout

| Path | Responsibility |
| --- | --- |
| `wit/` | Component Model package, world, records, variants, and host imports |
| `src/bindings.rs` | Wasmtime bindings generated from WIT |
| `src/host.rs` | component lifecycle, subscriptions, dispatch, limits, and dynamic syntax bridge |
| `src/state/mod.rs` | namespace registry and in-memory transactional StateStore |
| `src/state/persistent.rs` | `redb` persistent-project backend |
| `tests/contract.rs` | host and guest binding contract |
| `tests/host.rs` | CoreLibrary lifecycle and Wasmtime behavior |
| `tests/state.rs` | scopes, permissions, conflicts, quotas, and persistence |
| `tests/dynamic_syntax.rs` | real WASM dynamic registration against an SSG fixture |
| `tests/text_macro.rs` | ordered real-WASM expansion, diagnostic mapping, rollback, quotas, and traps |
| `tests/tree_macro.rs` | real-WASM node/body edits, recursive provenance, cycles, rollback, quotas, and traps |
| `tests/pattern_match.rs` | real-WASM element override and selected-candidate StateStore rollback |

## Testing

Build embedded components before integration tests:

```sh
cargo run -p xtask --locked -- build-core-library
cargo run -p xtask --locked -- build-test-components
cargo test -p parser-wasm --locked
```

The complete workspace check also verifies that host-only dependencies are not
accidentally required by guest components.

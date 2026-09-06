# Omitted typed arguments

[日本語](default-expressions.ja.md)

`skript-parser` retains every typed slot in a registration pattern, including
slots in unselected optional and choice branches. `[]` makes a branch optional;
the `-` flag makes its argument nullable. They have different semantics.

| Capture | Result |
| --- | --- |
| Explicit input | `TypeCaptureState::Explicit`; no default provider call |
| Omitted `%-type%` | `Null`; no child Expression or provider call |
| Omitted `%type%`, provider succeeds | `Default`; an `ExpressionNodeKind::Default` child |
| Provider rejects context or contract | Candidate failure with `DefaultExpressionFailureKind::Rejected` |
| No provider, missing Catalog data, unsupported semantics | Candidate failure with `Unresolved` |

The standalone structural pattern matcher retains `Omitted` slots. Semantic
parsing through `ExpressionSession` completes them before subsequent syntax
hooks. A structurally recognized candidate with an invalid or unresolved
default is never reported as a verified match.

## Shared parser API

`ExpressionParseEnvironment::provide_default_expression` receives a borrowed
`DefaultExpressionRequest`: syntax definition/registration and pattern identity,
capture index and pattern span, Type and requested cardinality, flags/time,
mapped insertion anchor, and Event/Section context. It returns
`DefaultExpressionDecision::{Resolved, Rejected, Unresolved}`. The parser validates
the resolved type, multiplicity, literal/expression flags and time before
adopting the child. Normal branch transactions also cover default children.
Failed completion restores its local branch checkpoint. A later hook rejection
uses the existing environment scope transaction to discard explicit and default
children together; the parser must not restore a newer branch checkpoint after
that scope has rolled back.

`DefaultExpressionInfo` retains requested Type identity, provider/component,
reason, Event classes, Section scope IDs, Catalog references and mapped anchor.
The child retains its resolved type, multiplicity, public data and namespaced
metadata. `ParsedCaptureSemanticSummary` and `SectionScopeCapture` expose the
same implicit provenance. LSP consumers and a future multiline REPL can traverse
these shared results without reconstructing a default from rendered text.

Implicit nodes have empty source ranges. Every mapped origin is `Anchored`,
including all origins and expansion IDs of a macro-derived insertion point.
No invented `to player` text is inserted into `MappedSource`.

## WASM addon contract

WIT `nlaocs:skript-parser-addon@0.35.0` uses ABI `17.0`. Request capability
`parser.default-expression` version 1 and subscribe to phase `default-expression`
with the existing `HookSubscription`, target and selector machinery. Dispatch
targets the requested **Type** registration; `default-expression-payload`
separately identifies the parent syntax and capture. Providers may target Type
definition/registration IDs or use a Type selector; registered handler bindings
can resolve exact parser classes to stable Catalog registration IDs. A
`RegisteredSyntaxHandler` declares its `phase`: use `default-expression` for a
default provider and `expression` for an explicit-input Type parser. This keeps
default-only support from changing ordinary Type parsing or its diagnostics.

The order remains target specificity, ascending priority, component load order,
then subscription declaration order. Multiple addons may inspect or transform
the same result. `NotApplicable` leaves it untouched. Start a replacement with
`component-id: none`; the host supplies ownership. Metadata follows the normal
component namespace and ownership rules, and later hooks can inspect it.

Set `outcome` to `resolved(default-expression-resolution)` or `unresolved(reason)`;
use the existing `HookDecision::Reject` for an invalid context. Identity, request
context and anchor are read-only. Provider effects are deferred until the child
is adopted. Rejection, invalid output, trap, cancellation and later candidate
rejection roll back speculative metadata, diagnostics and StateStore changes.
Diagnostics explaining a rejection remain attached to the failed candidate. A provider
failure cannot leave an earlier partial result reported as verified success.

`RegisteredExpressionChild.default-expression` and `ParseSummary.default-expression`
let later syntax hooks see implicit provenance. The request carries small source
references instead of Catalog documents. Use existing `catalog-data` queries and
`type-for-class` (indexed exact ClassInfo lookup); bounded `read-record` calls
retrieve raw SSG evidence when needed. Subscription routes are indexed by phase
and target. Explicit and nullable captures do not call WASM default providers.

## CoreLibrary standard provider

`core.default-expression.skript` is the final fallback for the standard
`SimpleLiteral`, `EventValueExpression`, and `ExprDamageCause` implementations.
SSG supplies an immutable descriptor with the implementation class, literal flag,
return class and concrete `isSingle()` result. The provider does not guess missing
shape data and does not depend on the Type owner's addon name, so an addon Type may
reuse a standard Skript implementation without changing CoreLibrary.

`SimpleLiteral` defaults are context-free literals and reject nonzero time states.
`EventValueExpression` defaults resolve the descriptor's return class through the
shared Catalog hierarchy, conversion, exclusion and ambiguity rules. For example,
Audience resolves its actual `CommandSender` target rather than assuming the
Audience Type class. `ExprDamageCause` preserves Skript's special rule: past and
present syntax is accepted, future syntax is rejected, and lookup still uses the
present EventValue.

The standard provider runs after more specific addon and parser-local providers.
Scoped `DefaultValueData` overrides and custom `DefaultExpression` subclasses stay
unresolved until their owning addon provides semantics. Unknown validators and
incomplete SSG descriptors are likewise never promoted to verified success.

`effectcommandcli` report schema 7 renders the same shared results. Capture
`state` is `explicit`, `omitted`, `null`, or `default`; implicit Expression reports include
`defaultExpression`, an empty `source`, and a zero-width anchor. Failure reports
include the recognized Effect/pattern and a typed `defaultExpression` reason
with capture index, expected types and rejected/unresolved state. A session keeps
its Snapshot and WASM host across inputs; `parseDurationNs` excludes Snapshot
loading and report rendering.

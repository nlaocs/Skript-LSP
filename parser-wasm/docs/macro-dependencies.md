# Unavailable source-macro dependencies

This is WIT0.50 / ABI32. Host, Core and every Component must be rebuilt together.
Older worlds are rejected, not silently treated as having no dependencies.
Capture schemas, source-preparation schema and fuel per callback are unchanged.

## Meaning and responsibilities

A language provider may know that a same-document macro name is declared while
its value is not statically available. This protocol records that negative fact.
It never supplies a constant, invokes a runtime callback, proves purity, grants a
type or changes a registry. Core interprets `skript.options`; the host knows no
addon names, option grammar or getter semantics.

After text macros and native raw-tree creation, the host runs mandatory
`parser.macro-declarations` v1 Document/Transform subscriptions. They use an empty
Parser carrier with ID `host.macro-declarations`, request ID0, empty input/types,
the current source span and one native `native.tree-view.v1` lease option.
Discovery is read-only: State reads/writes/scans, context changes, additional
parsing, dynamic registration and terminal decisions are rejected. The mode and
all leases are restored/revoked on normal return, invalid output, trap or cancel.
The discovery lease lists only nontrivia root headers and their node summaries.
Children, descendant/trivia text, details, spans, whole-tree info, diagnostics and
namespace queries are unavailable in this mode, after normal lease/cancel/query
checks. Invalid roots remain visible. Other Tree/document-declaration leases are
unchanged. Filtered page cursors address the same immutable header sequence.

The required capability is also a provider promise: names and completeness
depend only on ordered kind/text/context/child-count inputs plus current immutable
registration data. Numeric IDs, tokens, revision counters and carrier spans are
authority/location information, not semantic inputs. The host restricts observed
inputs; it does not prove arbitrary guest determinism or runtime semantics.

Return the unchanged Parser carrier plus one owned `macro.unavailable-bindings.v1`
JSON option (camelCase, unknown fields rejected):

```json
{"version":1,"complete":true,"bindings":[
  {"root":12,"namespace":"example.macro","name":"literal name"}
]}
```

Root12 must be a current top-level, nontrivia, context0 node in that lease. The
host authenticates/stamps ownership. Each provider receives a fresh carrier;
other providers' records are accumulated natively, never mutable guest input.
An explicit complete empty result differs from a missing response, disabled or
faulting provider, overflow or `complete:false`. Incompleteness applies to every
queried namespace, since a missing provider cannot prove which names are absent.
Names shared by providers compose their unavailable dependencies; no value writer
wins. This first version exports no cross-file binding data or global cache.

## Consumption and adoption

An active Tree macro may page `tree-data.unavailable-bindings(tree, namespace,
cursor, limit)` under its current immutable lease. The page contains `complete`,
opaque binding IDs/names, and `next`. Existing query/reply/byte/cancellation quotas
apply before cloning/lowering. No whole inventory is copied into each input.

The Tree output's `unavailable` list may reference an ID actually returned in
this callback, or an actually observed incomplete namespace. A stale/unqueried
ID, unsupported positive-completeness claim, foreign lease or invalid namespace
is invalid output. `parser.macro-dependencies` v1 is mandatory for adoption. The
result must be ContinueProcessing with no edit or HookEffects. A report never
becomes semantic proof merely because a guest supplied its spelling.

The host restores the whole current target's State, effects and edits, marks its
calls unaccepted, stops later providers and children, and maps the original raw
target span. Blocked generated replacements propagate to their original target;
unrelated unchanged siblings remain usable. Original raw nodes remain physically
present while the syntax/declaration view is masked. A source-macro dependency is
distinct from a guest crash and a missing Function namespace. The LSP's existing
Source barrier invalidates prior local/shared facts at the missing semantic node.
Unavailable regions and incomplete discovery suppress exports/body-cache-ready
publication; they cannot certify assignment, copy, completion type or diagnostics
that require the unavailable value.

## Refresh, limits and compatibility

There is no cross-revision snapshot cache. Source/preparation/provider fences and
fresh lease tokens bind each parse. After accepted tree edits, ordered nontrivia
root kind/text/context/child-count signatures detect declaration-input changes.
Preparation calls count against the existing total tree-macro-call budget. A
successful refresh with unchanged names/owners/completeness merely refreshes
locations. A changed semantic set falls back to whole-document unavailable:
all tree-stage edits/State/effects are rolled back to post-text-macro input, old
prototypes/exports are withdrawn and body-cache readiness is false. Generated
declaration-set changes are deliberately unsupported in this bounded phase.

Initial bounds: 64 bindings/provider, 256 globally, 256 bytes per namespace/name,
64KiB aggregate discovery JSON, 64 references/target, page size1..128, and the
existing total call/output/source/tree limits. The external adapter additionally
bounds discovery to2,048 nontrivia roots; comments do not consume that bound.
Overflow or opaque discovery is incomplete, never a truncated absence proof.

Core checks negative dependencies before positive option substitution, including
quoted/generated contexts and the result of each existing substitution/preload
pass. Known positive expansion still respects syntax-context hygiene. Truly
missing ordinary options retain their undefined diagnostics. A declared but
unevaluated option is reported as `parser.source-macro-unavailable` instead.
GET-body validation/evaluation, dynamic names and runtime initialization success
are separate capabilities and are not claimed by this protocol.

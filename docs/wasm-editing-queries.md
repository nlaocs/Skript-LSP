# Live suffix-member editing queries

This optional public host service uses the existing WIT 0.49.0 / ABI 31.0 custom
parser interface. A Component requires `parser.editor-completion` version 1 and
`parser.additional-parse`, together with project-stable semantics. An older host
without the editing capability rejects that manifest. ABI equality alone is not
a sufficient compatibility check; no WIT record layout or production fuel limit
changes for this service.

An opt-in Parser subscription has the route
`editor.completion/<own-component-id>/<one-punctuation-trigger>`, phase `Parser`,
capability `parser.additional-parse`, and mode `Override`. A foreign owner route
is rejected. Registered order is preserved. The current LSP advertises `.` as an
automatic completion trigger in addition to `{`; other trigger routes are only
available through ordinary/manual completion and document reanalysis.

## Query and output

The native parser invokes the service once before each eligible body line's
statement candidate speculation, while its accepted prefix and prepared owner
are still alive. The request contains that exact line input and mapped span.
Additional `host.expression` requests receive the full current native context,
including event classes and active Section frames. The provider may parse
receiver candidates and return editing proposals. Arbitrary source string
matching is not a substitute for receiver/type resolution.

A handled service returns one successful custom parser result with one
`editor-query-result` root, no summary, children, function or diagnostics. Its
single `skript.editor.query-result.v1` metadata record contains:

```json
{
  "version": 1,
  "complete": false,
  "exclusive": true,
  "proposals": [{
    "id": "stable-full-signature-identity",
    "label": "member()",
    "filterText": "member",
    "insertText": "member()",
    "snippet": null,
    "start": 7,
    "end": 7,
    "detail": "Declaration information",
    "documentation": "Source and unresolved conditions"
  }]
}
```

`start` and `end` are UTF-8 byte offsets into the exact line input, not document
or UTF-16 offsets. The LSP validates and converts them for the actual request
position, preserving receiver text and existing call syntax. Current v1 proposals
are exclusive member slots rendered with Method kind; nonexclusive output is
rejected instead of accidentally replacing ordinary completion.

Success here means that an editing query completed. Its result is never adopted
as an Expression, a semantic AST or a type/storage/callability proof. Reserved
editing parser results cannot be requested as ordinary semantic parsers. The LSP
records `queryContextVerified: true` but `contextVerified: false` and
`semanticGuarantee: false`: provider candidate semantics remain unverified.

## Isolation and bounds

StateStore writes, including deletion and compare-and-swap, are denied during the
query. Overlay changes are rolled back even on failure. Read observations,
semantic receipt registries, unavailable-dependency observations and incremental
invocation cache work are quarantined from the adopted parse. Guest purity is an
explicit project-stable service contract; host rollback is not a claim to undo
arbitrary guest memory or I/O. Existing extension faults and cancellation retain
their ordinary failure behavior.

Preparations in a nested query cannot replace or revoke the outer document's
owner attempts. An outer owner rejection revokes descendant batches, unknown
parent recovery does not publish them, and failed/cancelled/stale document results
are not published. Trigger-bearing bodies are not cached without a replayable
editing ledger; unrelated bodies can still reuse. With no service loaded the
normal parser skips editing context/source preparation entirely.

Current limits are 4096 bytes per line, 128 trigger-bearing line query attempts
per document, 256 proposals per result, 128 KiB per metadata record and 2 MiB
retained payload. The client menu has an aggregate 512-item limit across services.
Providers should use a smaller batch and mark truncation/incomplete source data.
Exact single-origin source equality and UTF-8 boundaries are required; generated
or ambiguous source is suppressed. The service is intentionally bounded, not a
promise of arbitrary late-file completion after its query budget is exhausted.

## Stored-variable editing inputs (optional v2)

`parser.editor-typed-inputs` version 2 adds an editing-only second pass over the
same ABI 31 / WIT 0.49 metadata transport (introduced in ABI 29 / WIT 0.47). Require the capability and project-stable
semantics, and register exactly one owner route `editor.refine/<component-id>`.
Older hosts reject that required capability; an ABI number alone is insufficient.

The live query may add `typedInputs` to its existing JSON output:

```json
{"id":"receiver-1","subject":{"hostToken":7,"rootId":0},"anchor":null,"start":8,"end":11}
```

Each subject must reference an actual successful native `host.expression` result
from this isolated query. Tokens or returned AST labels do not grant authority.
The host freezes a bounded private native forest, current receipts and exact
source proof before query cleanup. It retains no parsing scope or live StateStore
resource. Provider revocation, a new parse of the document or document release
invalidates the observation. The result's revision and original/virtual text must
still match. Effect results remain ineligible for Expression adoption.

The current frontend handles fixed-name scalar local/global subjects (including native
grouping). A plain subject begins at the raw code start. A later subject needs
an `anchor` reference to a native `host.effect` prefix ending at the receiver.
Its authenticated operand-evaluation contract must identify the same variable
and source range as its first evaluated operand in the current context. This
proves only the parsed prefix used for an editing suggestion, not the unfinished
statement's eventual runtime evaluation. No effect/syntax name is special-cased.

Preparation attempts and physical body owners are separate. The host indexes
the current native indentation tree (at most 65,536 nodes), identifies one exact
line's direct parent, and verifies its complete ancestry under the active
preparation owner. Typed inputs keep that direct parent, including executable
entry bodies. The frontend still requires exact current body membership and
source geometry. Duplicate spans, missing/cyclic ancestry, an oversized index,
or a changed source/tree pair provide no typed binding; a changed pair also
revokes the session's old batches. Attempt rejection and revision invalidation
retain their existing independent lifetime checks.

The semantic frontend reads the existing ordered flow state before the current
raw line's own recovery barrier, after all preceding statements. An earlier
unparsed line or unknown local-binding effect invalidates prior facts; a later
certified write may establish a new current binding. There is
no fallback to occurrence history, lexical declarations or an older revision.
List, generated, ambiguous or unavailable input has no typed result. A separate,
bounded same-flow global candidate may be sent with `certainty: "inferred"`,
`complete: false` and `readClosed: false`; it never supplies semantic proof.

The host then invokes the owner refinement route with one ownerless metadata
entry `editor.typed-inputs.v2`:

```json
{"version":2,"documentRevision":4,"inputs":[{"id":"receiver-1","start":8,"end":11,"storedTypes":["java.lang.String"],"complete":true,"multiple":false,"maybeUnset":true,"readClosed":false,"certainty":"current","evidence":"current-native-semantic-flow","boundary":"parsed-prefix-first-operand","semanticGuarantee":false}]}
```

`storedTypes` describes the current stored binding. `maybeUnset` and `readClosed`
are separate: unknown lookup paths may use default/global fallback, converters
or Player refresh. Core's variable-read v2 can close only the absent-value branch
for audited 2.15.4 fixed-name scalars (`missingValue: "empty"`); it does not
close refresh or conversion, or make a nullable value present. The typed-input envelope is v2 only. Consumers preserve source/revision ownership;
inferred inputs use evidence `same-flow-prior-candidate` and cannot establish types.
See [shared scalar flow](shared-scalar-flow.md) for candidate lifetime and effects.
A conditional member proposal is useful editing information, not a new runtime
call guarantee. Nullable copy/type inference and hard completion filters still
require the relevant lookup and storage contracts. Refinement must
not turn this record into semantic proof or claim runtime callability.

Refinement runs after semantic flow, without any old StateStore invocation,
dynamic-scope update or tree lease. State reads/writes and scope imports are
denied. Additional parse requests, context updates, diagnostics, semantic graphs
and replacement payloads are rejected. Only one bounded proposal output is
accepted, within the exact selected member ranges. Immutable catalog/addon data
may be read. The input-only/project-stable contract also applies to guest private
memory; rollback cannot certify arbitrary guest side effects. Existing fuel,
deadline, cancellation, Store restart/disable and output quotas remain in force.

Bounds per query: 64 observed native results, 128 nodes and 128 KiB per result,
32 selected inputs, and the existing 2 MiB document ledger. The frontend sends
at most 16 class names per input, either a complete current bound or explicitly
incomplete inferred candidates; a provider may support fewer and must fail
closed otherwise. Reflect handles listed unions and marks nullable, unclosed
or inferred receivers incomplete. This is not a promise that all runtime
members, unknown types or unions can be enumerated.

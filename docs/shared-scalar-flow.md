# Shared scalar completion and provider effects

Fixed-name global scalar state has two distinct channels:

1. A current, complete type bound may be used by semantic flow only inside an
   authenticated provider's explicit shared-execution contract.
2. An inferred completion candidate is an incomplete same-flow suggestion. It
   cannot justify diagnostics, exclude alternatives, satisfy a transfer input,
   or become a current fact. It is never a persistent type declaration.

A source entry starts with no known global values. Neither a variables section
nor an earlier event establishes the persistent value at this entry. Ordinary
captured Core data does not establish exclusion of concurrent writers.
Consequently its global suggestions are labelled **[inferred]**, both before
and after a suspension. Merely having no wait does not make them certain.

## Provider effects

A selected provider may publish owner-authenticated metadata:

```json
{"version":1,"local":"preserve","shared":"suspend"}
```

under skript.semantic.binding-effects. Local is preserve or invalidate;
shared is preserve, invalidate, suspend or reenter. Undeclared, malformed,
duplicate or untrusted effects are not harmless. Shared barriers revoke current
shared facts and reaching information; bounded prior candidates may remain for
completion. Known writes replace their target's candidate, DELETE clears it,
and source/alias uncertainty can discard candidates. Lists and interpolated
names are not tracked.

The syntax provider owns the meaning of suspension. Common native flow does
not inspect the word wait or recognize a special Delay node. The audited
Skript2.15.4 standard Delay provider declares operand evaluation before
suspension and local-binding preservation. This preserves variable type bounds,
not mutable object state. Other versions/implementations do not inherit that
new contract merely through a matching class suffix.

A provider with an applicable execution/interference model may publish
skript.semantic.shared-scope:

```json
{"version":1,"model":"serialized-segment","interference":"excluded-by-provider-contract"}
```

This is an explicit semantic premise owned by that provider, not an inference
from a synchronous header. It covers the owner's execution segments and
resumption policy. Every shared barrier still expires old facts. Reassignment
can establish a new fact only through an authenticated flow transfer and
explicit shared-preserve effects within this contract. Without such evidence,
the global current type remains Unknown. The synthetic contract tests do not
demonstrate isolation of a live captured server.

## Planned write versus shared poststate

Core's skript.standard-global-write-candidate.v1 computes the bound of the
cloned/serializeAs-transformed value passed to global persistence. It accepts
the existing primitive default-clone witness or the source-fenced registered
expression storage contract, including the audited delegate path. An invalid
explicit clone witness cannot fall back to another contract. The actual native
RHS bound is transformed once; a prior stored bound is not stored a second time.
Queue, serialization, refresh or other shared effects are not silently certified.

The selected parser.semantic-transfer.v1 metadata uses version2 and
outputUse: completion-candidate. A frontend flow request cannot consume this
operation as a current type result. Existing version1 flow operations keep
their meaning. The WIT world remains0.49.0 / ABI31; the generic JSON contracts
are explicitly versioned rather than changing the WIT records.

## Editing contract v2

parser.editor-typed-inputs version2 uses editor.typed-inputs.v2, envelope
version2. Each input declares certainty=current or inferred. Inferred inputs
have complete=false, readClosed=false, evidence=same-flow-prior-candidate and
semanticGuarantee=false. They are bounded editing-only data, not AST/type proof.

Reflect proposals from these inputs have [inferred] labels, incomplete menus
and exclusive=false. Native completion merges them with ordinary suggestions,
deduplicates edits and enforces the same menu limit. Older typed-input addons
cannot silently consume the new schema; their refinement manifest is rejected
until rebuilt/adapted. Existing WIT compatibility is not a claim of metadata
contract compatibility.

Candidates share current source, owner, revision, preparation-generation and
provider lifetime fences. Editing, close/reopen, cancellation or a stale source
cannot import prior-revision candidates. Per-binding candidates are limited to
16 names /8192 name bytes; new candidate entries stop at4096 fact entries.
The existing20M per-call fuel budget and host editing/transfer quotas remain.

This phase does not implement list/interpolated identities, whole-program event
order, persistent-value collection, live runtime guarantees or common Java
class catalogues. General condition/type-guard narrowing is not implemented;
this phase establishes reassignment and bounded branch candidates.


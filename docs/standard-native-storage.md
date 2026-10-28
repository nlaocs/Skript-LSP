# Audited standard native-event storage target

The Core storage model can qualify a standard native `on join` producer from a
source-bound SSG registry observation v2. Loading another plugin does not by
itself invalidate this target. An old v1 capture remains readable, but does not
gain the observations added in v2 and cannot authorize this model.

This is a static-analysis target with an explicit initialized, quiescent capture
premise. It is not an attestation of every future event delivery, arbitrary
manually constructed events, arbitrary reflective mutation, or a permanent JVM
mutation epoch. Its versioned model, basis, premise and adopted event registration
are part of the semantic provenance and Core-private scope receipt.

## Evidence and ownership

SSG observes facts; Core interprets their semantics. The host keeps immutable raw
source identity, bounded reads/preparation, component ownership and resource
lifetimes. No Java- or addon-specific host import is required.

SSG v2 records the complete ordered ClassInfo/exact binding inventory, actual
ClassInfo/Class/loader identities, and full EventValue/getter observations at both
ends of a synchronous export. A supported-API operation sequence, in-flight count,
coverage sequence and explicit method coverage detect changes, including ABA and
incomplete hook coverage. These observations do not turn the capture premise into
an observed global freeze.

Factory and registration evidence connects an actual retained EventValue and
converter to the original classfile method-reference recipe, caller method/BCI,
artifact and loader. A registration attempt, hidden lambda name, matching Java
type spelling, or plugin name is insufficient. Duplicate/no-op registration and
failed insertion do not create an acceptance proof. Registry instances remain
separate. Getters are not executed to infer their return domain.

The optional new Manifest registryCapture tables are `factorySources`,
`factoryMethods`, `factorySites` and `interval`. `factorySites` is a compact tuple
of exactly fourteen mandatory elements, including zero offsets and false flags:

```
[id, methodId, bytecodeIndex, eventClassId, valueClassId, factoryClassId,
 targetClassId, targetName, targetDescriptor, referenceKind, targetInterface,
 registrationMethodId, registrationBytecodeIndex, registryClassId]
```

Other tables retain named objects. The enclosing observation version distinguishes
this encoding. Missing, malformed, truncated, conflicting or oversized evidence
does not authorize a storage fact. Manifest separation is not part of this change.

## Conditional source model

The initial source rule pins Skript 2.15.4 and Paper 1.21.11-132-c5eb079 artifacts,
the reviewed SSG artifact, actual related Class/loader tokens, factory/registration
recipe and adopted standard join registration. It models the audited Paper native
join producer and PlayerEvent getter, keeping the public Player bound separate
from the source-audited implementation bound. It does not infer implementation
classes from a public type name or sampled runtime objects.

The existing generic storage dispatch checker still evaluates reachable exact
dispatch, registry order, ancestry, custom clone/serializeAs and beforeChange
behavior. Conflicting Class/loader identities cannot be collapsed into a graph
node merely because their catalog names match. Unrelated getter-only class rows
do not become storage graph dependencies.

Only the actually selected standard ExprEntity/EventValue path can use the rule.
Custom validators, context-dependent or unsupported conversions, replaced getters
and unsupported producers retain Unknown where their semantics are not proved.
The new native-event model does not certify command-argument producers. The older
source model remains a separate conditional path; no v1 evidence is promoted into
v2. Possible absence remains represented (`player?`), rather than forced non-null.

## Preparation, consumption and invalidation

Bounded source preparation validates the observation, then its correspondence to
the exact retained Type/EventValue rows. The completed witness owns those same
validated Type rows, so publication cannot substitute a different row vector.
The semantic model is constructed only after the structural checks finish.

At use, Core binds the qualified result to the current raw source digest,
snapshot, model/basis/premise, adopted registration, owner, document revision,
scope, range and expansion. Core-private Parse receipts carry authority; public
context metadata alone does not. Source/model/context changes, stale or foreign
receipts, cancellation and missing preparation cannot manufacture a current proof.
Source-preparation generation continues to invalidate body reuse through ABI 30.

The WIT/ABI stays `nlaocs:skript-parser-addon@0.48.0` / `30.0`; this change adds
observation semantics, not a host ABI. Existing per-call fuel and paging limits
are retained. Preparation failure leaves dependent storage facts unresolved and
does not masquerade as successful semantic certification.

## Verification boundary

Owned Java fixtures test collection without attaching an agent or starting a
server. A Java-emitted observation is decoded by Core both as a whole Manifest
and as staged raw rows. Explicitly simulated v2 observations test the conditional
source rule and public-client pipeline; they are not recaptured runtime evidence.

The existing real v1 Core/reflect captures remain unchanged. Actual successful
factory-site capture, live hook coverage, final v2 size and this target's success
on a real mixed server require an approved new capture using the final SSG JAR.
Missing or mismatched reflect member data is an independent dependency and must
not be relabeled to fit another environment. Synthetic success alone is not a
claim that the user's Player hint has been restored.

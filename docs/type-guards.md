# Fixed-name simple type guards

Status: implementation in progress after the separately verified test repair
(LSP 21ea157 / reflect 30a9b25; full Windows 2,152 passed, 0 failed, 24 ignored).
The original pause/resume boundary remains in type-guard-plan.md. No deployment.

## Provider and flow responsibilities

The provider of a condition publishes versioned skript.semantic.type-guard
metadata: native subject capture, true/false presence and positive upper bounds,
and optional current static-operand evidence requirements. Common flow does not
recognize source spelling, Java condition classes, event names or addon names.
Core publishes scalar CondIsSet presence semantics. Reflect publishes its actual
CondIsInstanceOf semantics for a statically resolved imported JavaType operand.
The latter unwraps ObjectWrapper; its positive *raw binding* upper bound therefore
includes the wrapper, rather than falsely treating the unwrapped type as stored.
Both written instanceof forms are false for unset input. Negative class exclusion
is deliberately not implemented; only justified presence and positive bounds.

Core's audited simple runtime if publishes control-flow v2 (condition capture,
body truth edge). Each branch is evaluated with its own facts and then joined
with the bypass. Else/elseif false-edge chaining, standalone condition success
fallthrough, compound predicates, list and interpolated subjects are not refined.

## Checked value and lifetime

Equal type sets do not prove value identity. An authenticated semantic-transfer
descriptor v3 promises same-source-when-unchanged and binding preservation;
the native Session grants a transient identity only after a successful current
read with unchanged source, no writeback, preserved bindings and identical bound.
Legacy v1/v2 outputs confer no identity. Same-type writes, refresh/writeback,
unknown effects or intervening opaque operands expire the relation. Existing
source/provider/preparation-generation/owner/revision certificates remain in use.

An explicitly pure operation may preserve the relation; binding-values v1 is a
stronger promise than merely preserving a type. A fixed local's current complete
fact may be constrained; heuristic completion candidates never become proof.
Shared variables remain incomplete suggestions unless an actual interference
exclusion contract already justifies a current shared fact. No absence of wait
or source spelling grants exclusivity. Impossible present intersections do not
become known-unset facts. Incomplete hierarchy/interface overlap stays open.

## Mixed provider authentication

Whole-tree unique-issuer checks remain unchanged. A bounded, opt-in
parser.semantic.intrinsic-facts.v1 request selects operation-local contracts
and immutable native Condition child fields. The private accepted-receipt registry
binds them to exact current header identity, native captures, input summaries,
metadata, public schemas and actual child owners. A copied token is not authority.
Foreign direct modifying authors reject a claim; independently accepted child,
ancestor and body writers do not become its issuer. Harmless private observers
must not be confused with payload authors. This scope certifies no whole value,
general state purity or body semantics. All operands still run their own effect
and read checks. Selected static RHS evidence is semantic data, not presentation.

Requests/results are bounded in count, bytes and traversal. Current guard-specific
intrinsic claims are not imported through body receipts; unsupported reuse falls
back rather than rebasing old authority. Ordinary non-guard bodies keep their
existing reuse path. The new host capability parser.semantic-type-guards v1 is
required by the new Core/reflect components. WIT 0.49 / ABI 31 and 20M fuel stay.

## Verification checkpoint

The release candidate passes public RPC with 27 ordinary editing cases,
6 existing edit/reopen observations, 6 guard edit/reopen observations and
negated-instance true/false-edge assertions. Isolated Neovim verifies both new
guards through actual menu acceptance and insertion. Saved actual snapshot
metadata passes Core-only and mixed-provider presence/absence/Unicode checks;
the mixed path also supplies declaration-only member completion. These offline
results do not constitute a fresh runtime capture or Java execution guarantee.

The authored mixed fixture combines previously saved registration/declaration
metadata and an owned collector fixture. Its provenance is not a real capture.
Native tests also exercise unknown types, mismatched owners/tokens/captures,
intervening effects, same-bound writebacks, legacy descriptors and stale lifetime.
Final focused and full aggregate outcomes are recorded in the closing checkpoint.

One existing limit was reproduced with the same source and fixture on old and
current binaries: a non-guard chance section with a mixed-provider body can
lose its whole-section flow contract and analyze its body from Unknown facts.
This phase does not widen authentication for all sections or change ordinary
body-cache policy. Guard removal into that path must produce Unknown, never a
stale positive refinement. Negated presence separately verifies known-unset.

The intrinsic registry admits at most 2,048 records / 2 MiB of bounded projected
data, with 128 KiB per projection and bounded ancestry traversal. Exhaustion or
unsupported provenance yields no new claims. Refreshing/opaque reads without
the stronger same-binding relation are not refined merely from equal type sets.

Official source anchors used: Skript 2.15.4 CondIsSet, CondIsOfType, SecConditional;
skript-reflect 5eb024bcc7f3d0d5b0714bfe5485c66f916f58da CondIsInstanceOf and
SkriptMirrorUtil.getClass. CondIsOfType is item/entity-specific and is not a
generic Java-type guard. No live server/oracle/capture was run for this phase.

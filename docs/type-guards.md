# Fixed-name simple type guards

Status: bounded local candidate; the if/else extension is in else-flow.md.
The original simple-guard checkpoint used source 8cd5883; final
capability-test correction 5d2f09d3 / reflect 9e8bda96.
The separate test-repair checkpoint remains 2b05a876 / reflect 30a9b250.
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

Core's audited simple runtime if/else publishes control-flow v3 with native
condition capture/truth and start/else roles. Current authenticated adjacent
siblings can pass the false input into else and join only the two branch exits;
old v1/v2 and unpaired branches keep their conservative bypass join.
See else-flow.md for the bounded candidate and validation status. Elseif,
standalone condition success fallthrough, compound predicates, list and
interpolated subjects are not refined.

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
existing reuse path. The original guard delivery introduced parser.semantic-type-guards v1 and used
WIT 0.49 / ABI31. Those are historical checkpoint versions; current if/else uses
WIT 0.50 / ABI32 as described in else-flow.md. The 20M fuel budget is unchanged.

## Original guard verification checkpoint (historical)

The original guard release candidate passed public RPC with 27 ordinary editing cases,
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

At that original checkpoint, a limit was reproduced on the same source and fixture on old and
current binaries: a non-guard chance section with a mixed-provider body can
lose its whole-section flow contract and analyze its body from Unknown facts.
That original phase did not widen authentication for all sections or change ordinary
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

## Final offline verification

The final committed test source passed the complete Windows command

```text
cargo test --offline --locked --workspace --all-features --no-fail-fast -- --test-threads=1
```

2163 passed, 0 failed, 24 ignored across 90 result groups.
The first full run is retained as 2,161/2/24: its only failures were stale
capability count/list expectations. The correction retains exact count/list
checks and adds required-version/uniqueness assertions; it changes no product
code. The second full run is the final outcome, not stitched focused coverage.

Focused checks: native host 57, LSP guards 6, reflect native 23, ordinary SDK 27
and explicit guard SDK 2. Public RPC: 27 cases, 12 edit/reopen observations and
2 additional edge/proof-boundary cases. Saved actual metadata: 4 Core-only and
5 mixed guard cases, plus 21/25 shared-state regressions. The separately ignored
actual-copy test was run explicitly using an existing current test binary.
Isolated Neovim completed 9 menu insertions, including both new guards.
The old host rejects the new component with the expected missing capability.
Independent review found no remaining must-fix.

Linux, live server/oracle and fresh capture were not run in this phase. No normal
executable, distributed demo, user editor, remote branch or PR was changed.
The 20M fuel, 22 GiB build-start and 10 GiB sampled floor remain unchanged.
Guard-specific body reuse and the non-guard mixed-section limit above are
explicit remaining constraints; no speed improvement is claimed.

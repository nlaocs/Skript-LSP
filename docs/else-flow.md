# Simple runtime if/else flow

Status: bounded local candidate; the cache regression repair has passed focused verification. This extends the guard contract;
it does not add elseif or compound predicates, and has not been deployed.

## Provider contract

The provider that owns the section declares control flow in authenticated
skript.semantic.control-flow metadata. Common variable analysis does not inspect
Skript condition/section spelling or maintain an event/type allowlist.

A version 3 start has kind branch, chain start, a native conditionCapture and
bodyWhen. An immediate alternative has kind branch and chain else, with neither
conditionCapture nor bodyWhen. Both sections must have the same authenticated
metadata owner. The alternative must have no non-body operands.

Example start: {"version":3,"kind":"branch","chain":"start","conditionCapture":0,"bodyWhen":true}
Example else: {"version":3,"kind":"branch","chain":"else"}

Common flow checks exact current source geometry, ordered direct siblings in one
raw body, and missing/rejected statements. Blank lines and comments are harmless.
Another statement, opaque geometry, wrong owner/version/shape, or an intervening
unresolved raw statement breaks the link. An old v1/v2 or unpaired branch retains
its conservative standalone join. An empty current v3 body still goes through
the ordinary body/editor-input analysis.

The start materializes both condition edges immediately after header evaluation.
It analyzes the taken body and returns only its exit Facts and bypass Facts.
The containing body can consume that outcome once for its immediate alternative.
The alternative uses bypass input, runs normal header/body processing, and joins
its exit with the taken exit. It adds no extra nonexistent bypass.

Guard epochs, read links, receipts, contexts and arbitrary Engine state are never
restored from the other branch. Reassignment, suspension and opaque effects run
through existing provider contracts and invalidation. Locals preserved by a
suspension remain local facts; shared-state certainty is not granted by this
contract. Completion-only candidates remain separate from current facts.

## Core adapter and compatibility

The standard adapter is audited against Skript 2.15.4:
- https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/ch/njol/skript/sections/SecConditional.java
- https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/ch/njol/skript/conditions/CondIsSet.java

Only current source-authenticated simple runtime If and Else receive a role.
Parse-if, multiline conditions, ElseIf and Then do not. Core's intrinsic proof
selects only its own current header contracts, not the child condition or body.
A condition provider retains ownership of its presence/type predicate.

Additional intrinsic claims are requested only by guard If and its immediate
Else. The preceding unique Core-owned v3 start and own DRAFT are routing inputs
for a fresh Else claim, never copied/adopted proof. Ordinary nonguard branches
retain their existing contributor proof and body replay path. Intrinsic guard
receipts remain cold; host replay guards and hit expectations are unchanged.
Nonguard mixed sections can retain the previous proof limitation and remain
Unknown when current evidence is unavailable.

WIT 0.50 / ABI32 and the 20M guest-call budget are unchanged. This is a versioned
metadata extension: older consumers treat v3 conservatively. Use a matching
Core/consumer pair to observe the new refinement. No old runtime success,
class exclusion, impossible-path elimination or whole-program ordering is inferred.

## Validation scope

The candidate tests cover nullable fixed locals, both-branch SET union and
reaching definitions, missing-path initialization, nested and empty branches,
raw barriers, owner/version/capture/operand rejection, effects inside either
branch, and current Component/LSP body editing, pre-cancellation and release/reopen.
Authored-provider fixtures test the common contract; saved SSG data and a separate
mixed-provider profile test actual Core/reflect integration without live capture.

Before the cache repair, focused verification: Core conditional unit13 passed; guard/native/Component/LSP15 passed, with the two explicit mixed-provider cases also passed. The first misfiltered zero-case run is not validation.

The first full run at the initial else candidate exposed seven ordinary-edit
body-cache regressions (2300 passed/7 failed/36 ignored). Those results remain
preserved. The repair keeps v3 roles but asks for extra intrinsic claims only
for guard If and its immediate Else; no host replay guard or old hit expectation
was weakened. Repaired-Core checks pass: Core conditional14, guard15, explicit
mixed2, incremental24 (one manual measurement ignored), and LSP worker19. The
ordinary If+Else test retains stored/hit2, cold equality and fewer guest calls;
the guard pair stays cold and requests fresh claims on both headers. No actual
UI latency improvement is claimed.

Final results are recorded only after the final-source focused and workspace
checks plus independent review. Dedicated client RPC/Neovim, live server/oracle,
fresh capture and deployment are not part of this candidate's verification.

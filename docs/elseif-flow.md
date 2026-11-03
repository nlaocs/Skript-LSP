# Conditional continuations

Status: bounded local implementation candidate. Final validation is recorded
after focused checks and one final-source workspace run; no deployment.

The section provider declares authenticated skript.semantic.control-flow
metadata version 5 for current simple and compound conditional roles. Simple
start/next have one Condition capture and bodyWhen; else and then have no
condition operand. Multiline start/next carries conditionMode (all or any) and
has an exact ordered body of Condition nodes. Native flow does not recognize
Skript spelling, event names or type names. Core emits these roles only for
source-authenticated standard runtime SecConditional behavior audited against
Skript 2.15.4. Parse-if remains outside the contract.

A body-local outcome contains accumulated taken exits and remaining false
facts. Only an immediate, same-owner/version, correctly ordered raw sibling can
consume it. Each next header runs normally from the remaining false input with
a fresh guard epoch. Its taken exit is added to the accumulated join; the
remaining false exit is retained. Every continuation publishes their conservative
join so missing else or interrupted editing cannot discard an earlier exit.
A paired then section consumes the true path from a multiline all/any result
without advancing its guard epoch; its exit rejoins the remaining false/unknown
paths. Final else consumes only the remaining false path. An unpaired next never
starts another linked chain. Missing/rejected statements and malformed or
foreign contracts break the link; comments and blanks do not.

Guard epochs/read links, parser/init evaluation context, receipts and resource
handles are never restored from another runtime path. Assignment, suspension,
opaque effects, nullability, initialization and completion-only candidates retain
their existing contracts. Incomplete candidates do not become current proof.
Parser/init context may remain conservatively unavailable after opaque code.

Core requests fresh intrinsic header claims only for a guarded condition or a
continuation of a guard-bearing chain. The preceding owned DRAFT routes a fresh
claim and is never adopted as proof. Ordinary comparisons retain the existing
body cache path; intrinsic receipts remain cold. Cold/hit equivalence and prior
stored/hit expectations remain required.

WIT 0.50 / ABI32 and the 20M guest budget are unchanged. Existing v3/v4 simple start/else
contracts remain supported. Different flow versions cannot be linked. Older consumers treat unknown v5 conservatively; use matching Core and consumer for the new precision. Compound paths preserve a Kleenean unknown outcome unless each condition publishes a current Boolean result-domain contract (guard v2). The condition list is bounded to 64 entries; malformed geometry, a raw barrier, or stale provider ownership disables narrowing.

Target source: https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/ch/njol/skript/sections/SecConditional.java

Tests cover false-path inputs, all exits/residual joins, multiple continuations,
nesting, missing/invalid roles, ownership/version/capture barriers, effects,
current Component and in-process LSP hints, Unicode/edit/delete/cancel/reopen,
mixed providers and ordinary body reuse. Dedicated new client, release/Linux and
live oracle/capture are separately unverified unless explicitly recorded.

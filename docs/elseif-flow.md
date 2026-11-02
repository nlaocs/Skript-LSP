# Conditional continuations

Status: bounded local implementation candidate. Final validation is recorded
after focused checks and one final-source workspace run; no deployment.

The section provider declares authenticated skript.semantic.control-flow
metadata version 4. Roles are start, next and else. Start/next have one
Condition capture and bodyWhen; else has no non-body operand. Native flow does
not recognize Skript spelling, event names or type names. Core emits these roles
for source-authenticated simple runtime If/ElseIf/Else audited against Skript
2.15.4 SecConditional. Parse-if, multiline/Then and compound predicates are not
added here.

A body-local outcome contains accumulated taken exits and remaining false
facts. Only an immediate, same-owner/version, correctly ordered raw sibling can
consume it. Each next header runs normally from the remaining false input with
a fresh guard epoch. Its taken exit is added to the accumulated join; the
remaining false exit is retained. Every continuation publishes their conservative
join so missing else or interrupted editing cannot discard an earlier exit.
Final else consumes only the remaining false path. An unpaired next never
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

WIT 0.50 / ABI32 and the 20M guest budget are unchanged. Existing v3 start/else
contracts remain supported. Different flow versions cannot be linked. Older
consumers treat unknown v4 conservatively; use matching Core and consumer for
the new precision.

Target source: https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/ch/njol/skript/sections/SecConditional.java

Tests cover false-path inputs, all exits/residual joins, multiple continuations,
nesting, missing/invalid roles, ownership/version/capture barriers, effects,
current Component and in-process LSP hints, Unicode/edit/delete/cancel/reopen,
mixed providers and ordinary body reuse. Dedicated new client, release/Linux and
live oracle/capture are separately unverified unless explicitly recorded.

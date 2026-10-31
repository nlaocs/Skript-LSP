# Fixed-name type guards: saved pause and resume plan

Historical pause record: saved before implementation. The separate test-repair source checkpoint
2b05a876 / reflect 30a9b250 is now verified (Windows full 2,152/0/24). Resume
from its documentation successor; do not restore the old baseline over those
repairs. No guard implementation is included in that checkpoint.

Current continuation: implementation resumed from 21ea157 / reflect 30a9b25.
See [type-guard contracts and verification](type-guards.md). The baseline and
initial work-state below describe the original pause, not the current candidate.

## Baseline and preserved state

- LSP: cfdda3c99b59690df6d2a96db34fdcb49234e6e5 (tree db50f505badca0b97118cc6ba87cc75590cacdc0).
- Reflect addon: 226225d49792053f9538631f5b49beb40a6967f8 (tree 383509280a1fc25169a2bff72074a82a80e2f74f).
- WIT 0.49 / ABI 31, typed-inputs v2, 20,000,000 fuel per guest call.
- Previous global-scalar checkpoint: Windows workspace/all-features 2,155 passed,
  0 failed, 23 ignored. These are previous results, not this phase's validation.
- Work done for guards: scope confirmation and repository/skill inspection only.
  No guard product code, draft patch, build or test command was started.
  Initial tracked diff and untracked-file inspection produced no changes.
  No Cargo/rustc/Neovim or shared build lock was found at this boundary.

## Authorized bounded design

The provider that understands a condition declares its refinement semantics.
Common flow analysis applies authenticated information in the corresponding
branch; it must not recognize condition spelling or whitelist event/type names.
First inspect actual registered conditions and the audited standard source:
illustrative Player syntax is not an assumed valid Skript guard.

Separate justified current facts from incomplete completion-only candidates.
A check concerns the value actually checked. It does not establish exclusivity
for a later unrelated global read. Reassignment, suspension, re-entry, unknown
effects, branch exit and joins must invalidate or merge evidence conservatively.
Null/unset and negative refinements require supported condition semantics;
do not invent exclusions or atomicity. Locals and shared globals have different
lifetimes. Reuse current effect, ownership and generation contracts where sound.

Scope: fixed-name variables and simple runtime type checks. Lists,
interpolated names, whole-program event ordering, common Java catalogs and
unrelated product features are separate work. Any required API/ABI migration
must keep host/Core/addons consistent and reject incompatible contracts.

## Resume sequence and acceptance

1. Read the completed test-review audit and inspect current source/diff,
   owned processes and shared lock before reusing any command or artifact.
2. Inspect real condition semantics and current branch/fact interfaces; obtain
   an independent design review of the minimal provider-to-flow contract.
3. Implement a coherent minimal path, with no source syntax checks in common
   variable analysis. Keep ordinary inference separate from stronger proof.
4. Verify positive/negative conditions only where supported, nullable/unset,
   reassignment, wait/re-entry/opaque effects, branches/joins, local/global
   differences, unknown types and an addon-defined equivalent guard.
5. Check inferred-to-certified boundaries, stale provider/source/preparation
   generations, Unicode offsets, edits and reopen. Use saved data and authored
   fixtures without changing their provenance.
6. Exercise public RPC and isolated Neovim completion plus actual insertion;
   run focused and applicable final full checks and independent review.
7. Record exact commits/artifact hashes, compatibility, passed/failed/not-run
   checks, limits, owned-process completion and lock release.

## Unchanged operating boundaries

Keep the 22 GiB build-start condition, 10 GiB sampled free-space floor,
shared build output/lock ownership and the 20M guest-call budget. Never retry
denied formatter/Git operations through alternative routes. Use reversible
local commits. No live server/capture, transfers, deployment, usual executable
or distributed demo/editor replacement, push or PR is authorized here.

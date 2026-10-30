# Test review before fixed-name type guards

## Scope and criterion

Baseline: LSP cfdda3c99b59690df6d2a96db34fdcb49234e6e5 and reflect
226225d49792053f9538631f5b49beb40a6967f8. The guard plan was saved separately
in [type-guard-plan.md](type-guard-plan.md); no guard implementation is included.

The user's review requirement is:

> 見つけたバグの挙動は、通常のユーザー操作・利用条件のもとで本当に発火するものか、再検討すること。コード上だけで成立する仮説、到達不能な経路、通常利用では成立しない限定的な条件は、実際のユーザー影響が確認できない限り指摘にしない。

Synthetic tests for real extension boundaries remain valuable: malicious or
broken addon results, owner/revision/cancellation, fuel/memory and old contracts
are not deleted merely because a fixture is synthetic. Similar setup does not
make two tests redundant when their behavioral boundary differs.

All Rust test-bearing files in the workspace and external reflect source were
inventoried (2,186 textual test attributes before edits, including ignored tests;
not a runtime pass count or coverage percentage). Independent source reviews
covered Core/storage/SSG/catalog, parser/host/incremental state, and LSP/reflect
RPC/editor tests. This is not a proof that every possible runtime path is tested.

## Changes and preserved coverage

| Area | Change | Behavior retained or strengthened |
|---|---|---|
| Property storage | Consolidate repeated modern-property override matrix into the complete family table | All families, each unknown/override, exact query order and early stop; missing/malformed records remain separate |
| Declared storage | Remove a test-only ancestor walker and its shadow test | Actual default-hook helper tests each receiver's override/unknown; real class-lineage tests keep missing ancestors, cycles and the bound |
| Saved snapshot | Replace success-on-missing-environment with explicit ignored test and required input | Both actual copy domains still run when the saved snapshot is explicitly selected; ordinary totals do not claim it ran |
| Checkpoints | Combine two identical alternate-branch-length matrices | Call order/tail sharing, cloned restore, all four effect fields, base/selected arenas and exported mutation isolation |
| Incremental equivalence | Disable both body subscription contracts in invocation/header-only comparisons | Assert no body hit or storage; cold/warm semantic comparison and actual guest-call accounting remain |
| Reflect component | Remove the repeated no-data initialization check from one test | Dedicated missing-data/wrong-binary test and both invalid-data directions remain |
| Reflect public RPC | Add two-step/nullable copies and real inlay/hover requests, proper EOF/error handling and stable method identity | Exact current types and hint columns, numeric edits, rapid revisions, close/reopen and all prior negative cases |
| Neovim | Add optional external-addon cases to the existing stock-menu test | Actual snippet acceptance and Unicode receiver with existing parenthesis; default Core test stays independent |

No product feature, inference rule, fuel budget or ABI is changed by this repair.
Removing the unused walker affects test-only code. Frozen matched host/Core/reflect
artifacts are reused for RPC/editor qualification; a release rebuild would not
validate a changed product path because there is no product-path change here.

## Findings distinguished from regressions

- The new RPC copy assertion first exposed a harness error: a source without a
  final newline used an out-of-range inlay EOF, and a missing-result fallback hid
  the protocol error as an empty array. Correcting the range and checking errors
  restores the expected real inlay/hover result; this is not a product regression.
- The owned fixture's global case actually returns no methods and has no current
  shared-write registry contract. It stays a negative. Prefix-independent method
  matching prevents future inferred labels from bypassing a negative assertion;
  captured-data global inference is qualified separately.
- The UTF-8 chunk-boundary test already contains a multibyte character. A console
  decoding misreading was withdrawn; that test is unchanged.
- Comparing a stale mixed-provider Analysis was not shown to fail with its older
  fixture, so it is not reported as a product bug.
- An optional body hit is not a promise of a miss. Consecutive-edit cold/trace
  comparisons remain; no blanket miss assertion is added for an otherwise valid
  future remapping strategy.

## Verification checkpoint

Focused results so far: Core native 484 passed / 0 failed / 4 ignored; the
explicit saved-snapshot copy test 1 passed / 0 failed. Public RPC checks now
cover 18 cases plus six edit/reopen observations, and the isolated stock Neovim
test has accepted both new external-addon insertions. Exact raw artifacts and
source hashes are retained locally, outside version control.

Final committed test sources (LSP 2b05a876 / reflect 30a9b250) passed:

| Check | Result |
|---|---|
| Windows full workspace/all-features/integration/doctest | 2,152 passed / 0 failed / 24 ignored, exit 0 |
| Explicit saved-snapshot copy | 1 passed / 0 failed / 0 ignored |
| Core native | 484 passed / 0 failed / 4 ignored |
| Checkpoint / incremental focus | 12 passed; 22 passed / 1 ignored; no failures |
| External reflect native and doctest / Component integration | 23 passed / 27 passed; no failures |
| Owned-fixture public RPC | 18 cases and six edit/reopen observations; exact current type and insertion edits |
| Saved actual-data public RPC | Core-only 21 cases / mixed 25 cases; inferred globals remain uncertified |
| Isolated real stock Neovim | Seven actual menu acceptances (two new reflect cases) and request summary; owned LSP/Neovim exit 0 |

The full count differs from the previous 2,155/0/23 by two consolidated test
functions and one explicit manual snapshot test moving from passed to ignored.
That snapshot test was separately run above; this is not lost negative coverage.
All ten protected normal/demo file hashes remained unchanged. The final
independent source reviews found no must-fix or lost distinct behavior. Frozen
matched artifacts retain their previously qualified product hashes; no new
release build, deployment or runtime-compatibility claim is made. Linux and live
server/oracle execution were not performed in this offline scope.

The exact full command was:


```sh
cargo test --offline --locked --workspace --all-features --no-fail-fast -- --test-threads=1
```

Keep the 22 GiB build-start condition, 10 GiB sampled floor, shared exclusive
output lock, one build job and 20M guest-call fuel. No formatter retry, live
server/capture, transfer, deployment, normal/demo/editor modification or
push/PR is part of this work. Linux is not rerun without transfer authority.

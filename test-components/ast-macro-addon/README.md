# AST macro test addon

A deterministic WASM fixture for the Issue #58 AST macro component-boundary tests.
It targets the `nlaocs:skript-parser-addon@0.34.0` package and the ABI 16
contract exposed by the current worktree.

The addon subscribes to the AST transform phase and selects its behavior from
the target node's `text`:

| Node text | Behavior |
| --- | --- |
| `delete` | Return an empty replacement tree, deleting the target node. |
| `one` | Replace the target with one generated root. |
| `many` | Replace the target with two generated roots. |
| `metadata` | Preserve the target subtree and add addon-owned metadata. |
| `preserved` | Keep the node without a replacement; the input node is marked `preserved`. |
| `cycle` | Replace with another `cycle` node so host cycle detection is exercised. |
| `reject` | Return a typed `Reject` decision with a diagnostic. |
| `addon-error` | Return a typed addon error with a diagnostic. |
| `trap` | Panic so the host observes a WASM trap. |
| `state-write` | Write a parse-scoped private StateStore value and keep the node. |

Every generated node reuses the target's mapped span. Generated nodes request
the `macro` context origin; retained nodes request `preserved`. The fixture
also records every invocation in a private parse-scoped namespace, which lets
host tests verify commit and rollback behavior for accepted, rejected, failed,
and trapped calls.

The crate intentionally implements only the AST macro export. Text, Tree, and
generic hook exports return `UnsupportedCapability` so accidental dispatch is
visible to a test.

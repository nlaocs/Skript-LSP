# Native entry-scope composition (v1)

`parser.semantic-entry-scopes` version 1 is an explicit capability for a
Structure entry-plan owner and a semantic fact composer. Both require the
capability. The WIT package remains 0.49 and the ABI remains 31; a host lacking
the capability rejects a new requiring component before use. Existing
components and compose v1 retain their previous behavior.

This contract permits selected facts to cross an admitted entry's header
authorship boundary. It does not erase the header author, certify an entire
body, execute a handler, establish presence/purity, or relax a foreign input's
storage/conversion rules.

## Native scope, not a public context claim

The document parser brackets only actual direct-entry Trigger/Conditions
parsing, after the current validated plan's context is applied. It carries the
exact original native layout lease. Raw/unplanned entries do not open a scope.
Defaulted native callbacks leave other environments unchanged. Additional parse
requests and public dispatch calls do not inherit the native environment stack.

The private registry keeps source identity, provider/component/preparation
generation, `(root, lease)`, owner, EntryData index, native entry/body geometry,
applied context and Enter receipt roots. Discarding another header alternative
cannot revoke or replace the selected lease. Leaving a body always expires its
live frame, including fatal failure and cancellation. Body completion alone does
not grant final admission.

The host settles scopes once at the native document-to-AST projection, before
mutable AST phases. The selected native candidate, immutable lease-bearing
plan, completed entry, exact graph and accepted Exit receipt ancestry must all
match. Exit roots must include the original retained Enter roots. No scope is
reconstructed from editor recovery, span similarity, public keys or a later AST.
Later AST/source mutation is still rejected by the existing immutable proof seal.

## Wire metadata

Only an opted-in current consumer on a genuine native Effect dispatch receives
ownerless candidate metadata:

```json
{"key":"parser.native-entry-scope-input.v1","value":"{\"version\":1,\"reference\":\"opaque-current-input\"}"}
```

This reference has no authority by itself. The host binds it to the complete
active native frame chain, exact consumer and operation/context. Input metadata
is removed from returned payloads; output bytes are accounted before stripping.
It is not a reusable storage resource or an importable body-cache receipt.

A composer may opt a normal selected fact composition into version 2 of
`parser.semantic.compose.v1`:

```json
{"version":2,"contextInput":"opaque-current-input","metadataKeys":["owned.fact"],"inputs":[{"capture":0,"publicSchemas":[]}]}
```

The existing selected metadata and native input seals remain mandatory. A v2
claim additionally needs the live private input at record time and the completed,
finally adopted scope at certification. An invalid v2 never falls back to the
ordinary v1 ancestor path. v1 must have no contextInput and keeps that path.

Version 2 may also request inputs[].metadataKeys for selected intrinsic
contracts already authored on a current native input by the composing issuer:

~~~json
{"version":2,"contextInput":"opaque-current-input","metadataKeys":["owned.assignment"],"inputs":[{"capture":0,"publicSchemas":["owned.identity"]},{"capture":1,"publicSchemas":[],"metadataKeys":["owned.value-result"]}]}
~~~

The host preflights at most 16 unique keys per input and 16 KiB of selected
key/value/owner bytes across all inputs before cloning those values. It requires
exact unique owner/key/value matches on the native input and its final AST;
requested public-data schemas likewise bind their exact version and JSON.
Only the exact single-child native Grouped chain recorded when sealing may
expose its actual author node. It never searches arbitrary descendants or
certifies a whole operand. Foreign inputs may retain a sealed value domain
without acquiring the composer's intrinsic facts. Compose v1 rejects the new
input field, including an empty array or null; its old shape is unchanged.

Core requests only existing intrinsic RHS contracts and variable identity on the
actual changer capture. It does not invent non-empty/preserve effects or copy
RHS type facts as destination storage. This preserves literal presence and
variable read-transfer semantics across the admitted header boundary while
leaving runtime callbacks and nullable event values conservative.

Only the exact admitted header's owner is permitted at that ancestor. An extra
writer on the fact, body/entry/outer ancestor, a global or Unbound effect, a stale
source/provider/generation, changed input, failed Exit or missing scope still
rejects it. Whole-subtree `unique_issuer` remains strict.

## Bounds and conservative failure

Per parse: at most 256 recorded header alternatives, 256 frames, 1,024 inputs,
32 active frames per input and 2 MiB of registry accounting. Native graph
admission is bounded to 256 nodes / 128 KiB per root region and retained receipt
ancestry to 4,096. Opaque reference text is at most 128 bytes; the guest input
object is at most 256 bytes. Existing call/output quotas and 20M guest fuel are
unchanged. Missing or exceeded bounds withdraw admission while ordinary parsing
continues conservatively. No old/private scope is retained to make a fallback pass.

Entry inputs/composition drafts are volatile: dispatch/body cache machinery
cannot replay or export their private authority. A fresh parse must rebuild its
own native scopes. The originating provider retains its meaning and ownership;
the host establishes current identity, reach and adoption only.

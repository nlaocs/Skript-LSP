# Sealed value inputs and selected semantic facts

The opt-in host capability `parser.semantic-value-facts` v1 uses WIT 0.48 /
ABI30 metadata transport (unchanged from ABI29). A producer requires this capability before emitting
value-domain drafts. Core advertises optional participation: without native
input seals it keeps the older conservative result. A host lacking this capability rejects
a producer that requires the capability. This does not change the WIT layout.

## Value-domain draft

An accepted registered-expression provider can attach its own metadata key
`parser.semantic.value-domain.v1`:

```json
{"version":1,"classes":["example.FinalValue"],"complete":true,"maybeUnset":true,"beforeChange":"default-clone","evidence":"versioned source/data provenance"}
```

`classes` describes the non-null values after the expression's output conversion,
not the registered or advertised return type. Completeness, absence and single
cardinality are separate. `beforeChange` is either `unknown` or an explicitly
audited `default-clone` contract. This is not a purity certificate. Declaring a
capability or copying a metadata entry does not supply runtime evidence.

The private per-parse registry records the actual accepted issuer/receipt,
registration, exact mapped span and effective output summary. When a native
Effect capture actually contains that current value, the host can issue
`parser.native-semantic-input.v1` on its immutable capture summary. The reference
is bound to the canonical native expression subtree, including grouping,
provenance, child structure, public data and receipt topology. Guest-supplied
copies are removed at this boundary. Generated, ambiguous and expanded ranges
remain unsupported. The native source and full forest are sealed before mutable
AST phases and checked again when final facts are consumed. The private subject
hash projection validates each original geometry checksum before normalizing
text and its derived checksum; it never publishes or reseals a modified AST.

Core consumes only the native seal and independently checks its actual
ClassInfo clone/storage evidence. It can derive a local scalar post-storage
type without changing an advertised Object return type. Unknown callbacks,
unproved class dispatch, incomplete domains and multiple values retain unknown.
The expression's evaluation effects remain unknown unless separately certified;
other local facts must therefore still be invalidated by an unknown call.

A post-storage fact describes the stored binding. It is not a certificate for a
later Variable lookup/conversion. Variable-read v2 adds an independently owned
`missingValue: "empty"` contract. Current Core emits it only for fixed-name
scalar locals under the source-audited Skript 2.15.4 adapter. That path uses
LiteralString/MESSAGE and does not enumerate default/global fallback names.
With this claim, a complete nullable bound can survive the lookup while remaining
nullable. Player refresh, typed conversion and the consuming SET's storage
transformation still require their own existing contracts. Other names/versions
and v1 reads do not gain this absence claim; unsupported metadata stays unknown.
This composition feature alone does not establish a read or a nonempty value.

## Selected fact composition

A participating parent provider may publish `parser.semantic.compose.v1`:

```json
{"version":1,"metadataKeys":["example.assignment"],"inputs":[{"capture":0,"publicSchemas":["example.variable-identity"]},{"capture":1,"publicSchemas":[]}]}
```

The private registry binds the parent output and exact immutable sealed captures.
The final proof validates the current capture association, subtree digests,
retained accepted receipts, all other authors in the affected ancestor/subtree,
global/unbound effects and the final forest/source. Input public-data claims
must be the composing provider's own facts. A later modifying provider,
unretained/foreign receipt, changed capture or unknown extra author denies the
composition. Only the listed metadata and input schemas gain that certificate.
`unique_issuer` and the existing whole-node provider proof are unchanged.

The initial Core consumer composes only its local scalar SET post-storage
conclusion, operand order, applicable evaluation-context fact and static target
identity. No Java, reflect, player, method-name or variable-name branch exists
in the native composition code.

## Bounds and reuse

- At most 256 value outputs, input subjects and compositions per parse.
- At most 256 native nodes / 128 KiB canonical representation per subject.
- At most 16 closed classes and a 16 KiB draft; at most 8 input captures and
  16 selected metadata keys per composition.
- Only Effect candidates with an actual current value draft enter this path;
  ordinary Core-only inputs retain their prior dispatch-cache behavior.
- New draft/seal/composition paths bypass dispatch and body replay, including
  nested receipt closures. Edits, reopen and cancellation require fresh evidence.

The reference in metadata is never authority on its own. Value semantics and
supporting source/data remain the producing addon's responsibility. A complete
owned source model is distinct from an incomplete API declaration list or a
captured production server environment. This contract alone does not expose
LSP flow facts to live variable-member editing queries.

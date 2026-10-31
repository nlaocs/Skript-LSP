# Independent addon inputs and editing proposals

The current contract is WIT `nlaocs:skript-parser-addon@0.49.0`, ABI `31.0`.
The `parser.addon-data` v1 capability was introduced in WIT 0.47 / ABI 29.
Version 2 uses the same wire interface and adds initialization-time validation
of explicitly configured component binary pins. The host advertises v2; v1
consumers remain compatible.
Rebuild the host, CoreLibrary and all external
components together. Older WIT imports or ABI manifests are rejected. SSG files
and the Catalog remain separate from these optional addon inputs.

## Immutable dataset inputs

LSP `initializationOptions.addonDataSources` accepts entries with `path` and `pin`.
Relative paths resolve against the project root. A pin contains `componentId`,
the exact Wasm binary's `componentSha256`, dataset `id`, `schemaId`, positive
`schemaVersion`, whole-file `sha256`, and a string map of `bindings`. Each file is
a JSON envelope:

```json
{
  "formatVersion": 1,
  "componentId": "example.analysis",
  "id": "members",
  "schemaId": "example.members",
  "schemaVersion": 1,
  "bindings": { "environment": "producer-defined-identity" },
  "payload": { "records": [] }
}
```

All envelope identity fields and bindings must equal the explicit pin. Bindings
are opaque to the host: the addon must check what they mean against its current
runtime and domain. Equality of strings does not prove an actual server's loader,
artifact set or access rights. Missing information cannot certify absence.

The host reads and verifies files once during project initialization: at most 64
sources, 4 MiB per envelope, 16 MiB combined. Counts and the remaining byte budget
are checked before reading additional sources. The guest receives no filesystem
path and cannot specify another owner. After its manifest identity is validated,
`addon-data.info(id)` exposes the authorized source's schema, bindings, payload
length and separate `envelope-sha256` / `content-sha256`. The latter hashes exactly
the bytes returned by `read(id, offset, max-bytes)`, in chunks of at most 64 KiB.
Chunks can split UTF-8 and must be assembled before decoding. In v2, after the
loading manifest identifies a component and before its initialization, every
explicitly configured source for that component must match the actual Wasm
binary SHA-256. A mismatch rejects the load even if another source matches;
it cannot masquerade as an omitted optional dataset. Sources for other
component IDs remain unavailable to this component and are checked only when
their own component loads. Omitted sources remain optional. Payload/schema,
identity and size validation remain separate. Calls before manifest validation
are denied. Consumers that rely on distinguishing omission from an invalid
configured binary pin must require `parser.addon-data` v2; older hosts do not
provide this distinction.

These bytes are shared unchanged across safe epoch recreation. The restart-safe
promise permits immutable Catalog/profile and authorized addon-data queries;
other unobserved I/O or mutable global semantics still violate the promise.
Updating a dataset requires a new project initialization and matching pins, not
an unnoticed file reread during parsing. Native embedders can use
`AddonDataSource::from_file_with_budget` and `HostConfig.addon_data_sources`.

## Current-document declarations

`parser.document-declarations` v1 is an explicit `ParseStage` subscription at
`Document` / `Transform` with an empty selector. It also requires
`parser.project-stable-semantics` v2. After preprocessing and fault masking, before
the native dynamic registry freezes, the host sends an input-only Parser payload
with parser ID `host.document-declarations`, empty input/expected types and one
ownerless `native.tree-view.v1` option. Its JSON has `version: 1` and canonical
decimal-string `epochHigh`, `epochLow`, `token` and `generation` fields. These
identify an invocation-owned `tree-data` view, not transferable project data.

Read roots/details/children/text through the existing bounded tree-data pages.
The view is revoked at return, failure or cancellation and cannot cross stores
or calls. Output may contain diagnostics, private Parse state and registrations;
payload replacement, context changes and extra parsing/results are rejected.
Accepted registrations are document/revision private and rolled back on failure.
The result exposes the frozen native snapshot for current editing projections.
Neither this capability nor project-stability v2 exports declarations across
files or certifies execution, presence, purity or runtime values. Components with
document declarations cannot certify a closed startup grammar/binding table.

Structure entry context updates are a separate native boundary. Context-only
updates to the current context remain provisional until all providers accept a
final native Section with a scoped body and no declaration output. Their author
and receipt dependencies remain attached to that body. Body-less directives,
foreign contexts, extra parsing, shared State and other unbound effects retain
document-wide conservative reach; a later provider can prevent scoped adoption.

## Current-document identifier proposals

An accepted physical root Structure may add metadata key
`skript.editor.document-completions`. The host stamps the provider owner. The LSP
only consumes exports from reachable, exact current-source root Structures with
a valid current native seal and an attached host-stamped owner present in the
accepted writer set; copying this metadata onto a foreign or
unverified node grants no authority. `SemanticWitnesses::metadata_owner` checks
this data-only authorship without weakening the separate single-issuer semantic
type proof (`provider_valid`). Multiple accepted writers are allowed for editing
proposals, not as a way to combine incompatible type guarantees. The JSON v1 contract is:

```json
{
  "version": 1,
  "complete": true,
  "candidates": [{
    "id": "document-alias",
    "label": "ExampleClass",
    "insertText": "ExampleClass",
    "kind": "value",
    "caseSensitive": true,
    "detail": "Imported class alias",
    "documentation": "An editing proposal, not a type guarantee."
  }]
}
```

`kind` is `value` or `expression`. Insertions are single plain identifier-like
tokens (Unicode letters/numbers, underscore and dollar sign), without snippets,
multiple words or source-spanning edits. Proposals have whole-document scope;
the v1 contract does not describe nested visibility or argument compatibility.
They are offered through the normal expression editing gate, outside strings,
comments and variables. Prefix/case, existing suffix, UTF-16 replacement ranges,
global ranking and item limits use common LSP machinery. They remain unverified
when a context has a known expected type; no type is inferred from a proposal.
The response carries `addonProposal.contextVerified: false` and
`semanticGuarantee: false`. Labels can differ from insertion text; matching and
ranking use insertion text.

There are at most 32 exports, 128 KiB each, 2 MiB combined and 256 candidates per
export. IDs are unique per owner; conflicting duplicates are invalid. Unknown
fields, including proposed type/evidence fields, are rejected. If any export is
invalid, addon proposals for that completion snapshot are suppressed and logged;
ordinary completion providers continue and the list reports incompleteness.
No proposal can make parsing succeed or create a definite diagnostic/type fact.
Exports are rebuilt from the current accepted result; prepared partial results,
new documents and closed documents cannot retain a previous result's proposals.

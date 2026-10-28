# Independent addon inputs and editing proposals

WIT `nlaocs:skript-parser-addon@0.48.0`, ABI `30.0`, adds the
`parser.addon-data` v1 capability. Rebuild the host, CoreLibrary and all external
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
Chunks can split UTF-8 and must be assembled before decoding. Missing or wrong
owner/binary pins are unavailable. Calls before manifest validation are denied.

These bytes are shared unchanged across safe epoch recreation. The restart-safe
promise permits immutable Catalog/profile and authorized addon-data queries;
other unobserved I/O or mutable global semantics still violate the promise.
Updating a dataset requires a new project initialization and matching pins, not
an unnoticed file reread during parsing. Native embedders can use
`AddonDataSource::from_file_with_budget` and `HostConfig.addon_data_sources`.

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

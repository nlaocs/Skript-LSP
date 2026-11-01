# Direct Structure entry scope plans, v1

This is an optional native extension; the wire world remains WIT 0.49 / ABI31.
An addon using it must require both `parser.structure-entry-plans` v1 and
`parser.context-updates` v1. A host without the former rejects initialization.
The existing `structure.entry-body-modes.v1` contract remains available.

For a bounded native EntryValidator, EnterBody carries ownerless
`host.structure-entry-layout.v1` metadata. Its JSON has version, a fresh string
token, structureNode, and root entries with index, key, kind, entryDataClass and
multiple. These are immutable declarations, not indexes into parsed/defaulted
entry values. Missing/oversized layouts cannot use the new extension.

A Transform semantic owner may publish `structure.entry-plans.v1` metadata:

```json
{"version":1,"token":"<current token>","structureNode":0,"entries":[
  {"node":2,"declarationIndex":1,"mode":"trigger","adoptUnknown":true,
   "context":{"eventClasses":["example.HandlerEvent"],"values":[
     {"key":"example.handler","value":"change"}]}}
]}
```

The host stamps component ownership. At most one component owns a plan; at most
32 current direct entries and 16 KiB of JSON are accepted. Executable entry plans
require Section nodes. Body modes
are trigger or conditions. Context patches permit up to eight event classes,
16 key/optional-string values, 128-byte keys and 1,024-byte values. Null removes
a value. `parser.event-classes` uses the separate eventClasses field. The native
syntax context and section stack cannot be supplied by metadata.

Native validation precedes body evaluation and editor preparation. Root identity,
lease, immutable declaration index and current direct raw node must agree.
Section/Trigger entries must already select that declaration; Trigger remains
trigger. Unknown Section adoption must be explicit and cannot take a different
native match, a duplicate single entry, a nested/foreign node or a default value.
A plan cannot also override that same entry through the old body-modes metadata.

Each body executes its native parser once under a cloned lexical context, then
restores the previous context before propagating the result, including failure
and cancellation. Prepared editor scopes use the same validated selection.
ExitBody cannot add, remove or change the accepted effective plan. Plan numbers
and tokens are invocation-scoped and are not persistent permissions.

The plan is a parser-context contract, not a promise of runtime values, callback
execution, purity, accepted storage results or initialization success. Its lexical
authorship uses the existing StructureContext receipts. Shared/durable writes,
dynamic changes and additional parsing retain their existing Unbound treatment;
this feature does not import or rewrite their receipts.


## Data-only entries

Requiring parser.structure-entry-data v1 in addition to the two capabilities
above enables entry plan mode raw. The same owner, lease, root, immutable
EntryData and direct Unknown Section adoption checks apply. A raw entry cannot
carry an event-class or context-value patch. Its original RawTree children are
retained as Section data without Trigger / Conditions parsing, and editor
preparation marks it Raw rather than offering statement completion. This is a
per-entry mode, not a new StructureBodyMode or permission to rewrite source.
Old hosts reject the required capability; WIT 0.49 / ABI31 remains unchanged.

## KeyValue source aliases

An entry record may include optional `keyAlias`, a nonempty UTF-8 string of at
most 256 bytes without control characters. This requires mandatory
`parser.structure-entry-aliases` v1, `parser.structure-entry-data` v1,
`parser.structure-entry-plans` v1 and `parser.context-updates` v1. Older hosts
reject the required capability at loading; they do not silently ignore aliases.
WIT0.49 and ABI31 are unchanged because this is an explicitly gated metadata
extension. Native Rust users constructing StructureEntryPlan must initialize the
new optional key_alias field; use None for existing plans.

An alias is limited to one current direct child Simple node with no children,
an unchanged KeyValue EntryData, Raw mode, adoptUnknown=true and no context
updates. It cannot claim a canonical match, another selected entry, a duplicate
single entry, a nested/foreign node or a stale lease. The existing separator is
required. Multiple=true keeps its original behavior. The canonical key and
EntryData identity survive parsing; key/value spans use the actual source
spelling's byte length. Source text/provenance is never rewritten.

Prepared and final editor scopes retain only aliases admitted for the current
selected node. Partial unowned text cannot guess aliases. New-key completion
continues to offer the canonical key; per-node alias plans do not declare a
catalog-wide alias list. Alias values remain raw data, not executable bodies.

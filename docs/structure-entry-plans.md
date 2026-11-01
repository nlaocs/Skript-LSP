# Current Structure entry plans

Component-owned `structure.entry-plans.v1` echoes the fresh host-issued
`host.structure-entry-layout.v1` token and current root node. Native validation
retains the frozen EntryData declaration index, class, kind and multiplicity.
These plans control parser layout/context; they are not runtime type evidence.
The existing Trigger, Conditions and data-only Raw Section modes are unchanged.

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

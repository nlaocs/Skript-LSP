# Explicit public project source types

`parser.project-public-source-types` v1 is an optional extension to the
owner-scoped `parser.project-declarations` carrier. It gives an opted-in
consumer a separate bounded view of source `Type` declarations that their
providers explicitly marked for cross-owner use. It does not widen the
existing callable-declaration view or change the WIT/ABI layout.

The host sends `native.project-public-source-types.v1` only to a declaration
consumer that claims the capability. The envelope contains `version`,
`project`, `complete`, and `types`. Each type has the provider's original
`localId`, one `pattern`, its declared `returnType`, an `origin` namespace/key,
the selected metadata map, and host-stamped `ownerComponentId` and source
identity (`uri`, `documentId`, generation, source epoch, and source digest).
The host never copies the guest's owner/source claims and never forwards a
handler or callback through this view.

A provider opts one declaration in with metadata
`parser.project-public-type.v1`, whose v1 payload lists the metadata keys it
intends to publish. The host accepts only nonlocal project `Type` definitions
with one nonempty pattern matching the origin key, a declared return type and
no multiplicity. It limits an envelope to 64 types and 64 KiB, allows at most
16 selected keys per type and 16 KiB per value, and clears the entire public
view when any marked entry is invalid, the source project is incomplete, or a
quota would be exceeded. No partial list is labeled complete.

Consumers still validate the selected metadata against their own semantic
schema and detect alias/binary-name collisions. The complete flag means the
host's current project snapshot and the set of explicitly marked declarations
were collected within those limits; it is not a claim about unmarked types,
runtime class generation, loading, linkage, access, or invocation. Consumers
that do not request the optional capability receive no cross-owner type data.

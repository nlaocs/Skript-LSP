# Project conflict-only keys, v1

An addon requiring parser.project-conflicts v1 must also require
parser.project-declarations v1. It receives a separate native.project-conflicts.v1
option alongside the unchanged declarations-v1 carrier. WIT 0.49 / ABI31 remains
unchanged; an older host rejects the new required capability.

An opted-in own export can carry optional projectConflictKey in its
parser.project-export.v1 draft. It is a nonempty control-free key of at most
1,024 bytes, bound to the same current owner, source, namespace and root as the
ordinary export. The host retains keys from private and noncallable own headers
as well as public headers; imported headers are never re-exported. Alias keys
are deduplicated only at the identical source root and namespace.

The new JSON option contains version:1, project, complete and conflicts. Each
conflict has uri, decimal root, namespace and key. It conveys only same-provider
foreign occupancy, never callable patterns, bodies, values, scopes, import
tokens or permission to access another provider. Public imports and private key
records share 256 items and 64 KiB of encoded options. Overflow/incomplete input
withdraws both complete lists instead of reporting truncated absence. A native
input fingerprint and body dependency key include these identities; source,
provider, Project membership and lifetime validation remains mandatory.

Use ordinary project declarations for callable imports. A conflict-only record
is an input to language-specific uniqueness rules, not authority to import or
execute private syntax. Existing addons that do not request this capability keep
the original v1 input shape and limits.

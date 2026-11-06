# Source semantic roles, version 1

An addon may attach owner-stamped `parser.source-roles.v1` metadata to an
adopted dynamic Structure, Expression, or Type node. This is an optional editor
projection, not evidence of runtime class generation or callable members.

The JSON record contains `version: 1` and `targets`. Each target has `text`
(the exact final AST node text), absolute virtual-byte `start` and `end`, and
`spans`. Each span has relative UTF-8-byte `start`, exclusive `end`, and a role:
`class`, `type`, `method`, `property`, `parameter`, or `keyword`. These map to
the corresponding standard LSP semantic token types.

One target must identify the declaring node itself, even when its span list is
empty. A Structure may additionally identify its actual direct native entry
children. Expression and Type records may identify only themselves. Roles may
not reach arbitrary document text or executable/capture children.

The consumer requires the current document/revision dynamic snapshot, matching
definition/registration/kind, a single ranked candidate, the dynamic provider's
host-stamped metadata owner, and the native current capture geometry seal.
Targets must match exact final text and virtual bounds with one exact original
source origin. Byte ranges must be valid UTF-8 boundaries, nonempty,
nonoverlapping, and in bounds. Unknown roles, versions, duplicate targets,
duplicate records, malformed JSON, and missing root fences add no roles.

Records are limited to 64 KiB, 1024 targets, and 1024 spans. Normal comment,
unresolved-source, and conflicting-ownership exclusions still apply. Child
statements use their own parsed semantic tokens. The host contains no addon-
specific declaration grammar or name-based coloring rule.

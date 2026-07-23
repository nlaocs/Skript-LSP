# syntaxes

[日本語](README.ja.md)

`syntaxes` is the format-independent runtime model for everything registered by
Skript and its addons. It is the boundary between serialized SSG data and
parsing or LSP consumers.

The crate does not read JSON or know which Skript version produced the data.
[`ssg`](../ssg/) owns those concerns and constructs a `Catalog`.

## Domain Model

`Syntax` contains the eight syntax categories in project order:

1. Event
2. Condition
3. Effect
4. Expression
5. Type
6. Function
7. Section
8. Structure

`CommonSyntax` holds registration order, documentation, parsed patterns, addon
ownership, definition and registration IDs, priority metadata, experimental
requirements, supported events, and optional return handlers.

Specialized models add category-specific data such as expression
multiplicity/changers, section flags, structure validators, EventValues,
function parameters, or type parser metadata.

The model also includes:

- Java class hierarchy and class kinds
- converters and comparators
- properties and property handlers
- arithmetic operators, operations, and differences
- aliases and normalized alias targets
- generated plural rules

Nullable fields and `ResolutionState` remain distinct. A missing value may mean
that data could not be resolved, while an empty collection may mean that the
registry was inspected and contains no entries.

## Catalog

`Catalog` owns normalized data and builds indexes for common parser queries:

- syntax by registration ID and by category
- type by code name
- function overloads by name
- EventValues inherited through event class hierarchy
- converters by source or destination class
- Java class and type assignability
- aliases, comparators, properties, arithmetic, and plural rules

Class and EventValue traversal is cycle-safe. EventValues follow generated
resolution order and honor exclusion classes.

`CatalogParts` is the explicit constructor input. It is useful for converters
such as `ssg` and for small isolated tests.

## IDs

Static syntax has two identities:

- `DefinitionId` groups the semantic syntax definition.
- `RegistrationId` identifies one concrete registration.

A definition may therefore map to multiple registrations. Overrides can target
either identity depending on whether they replace all forms or one exact
registration.

## Dynamic Syntax Registry

`DynamicSyntaxRegistry` overlays WASM-provided syntax on an immutable
`Arc<Catalog>`.

A dynamic ID is namespaced as:

```text
dynamic:<component-id>/<local-id>
```

Components can register definitions with patterns, kind, return metadata,
handler, free-form metadata, numeric priority, and `before`/`after`
constraints. They can also override static definitions or registrations.

### Lifecycle

- initialization updates become the baseline for future documents
- each document revision clones that baseline
- Document/Preprocess hooks may stage document-specific updates
- savepoints allow parser candidate rollback
- `freeze` validates references, detects cycles, and returns an immutable
  `DynamicSyntaxSnapshot`
- component removal affects baselines and mutable documents, but never mutates
  a frozen snapshot

Registration operations are transactional. A `DynamicSyntaxUpdate` changes no
registry state until `commit`.

### Ordering

The frozen candidate list combines static and dynamic syntax. Explicit
constraints form a graph and must stay within one `SyntaxKind`. A deterministic
topological sort uses kind order, numeric priority, static/dynamic class,
component load order, declaration order, and ID as stable tie breakers.

Unknown references, cross-kind constraints, and priority cycles are typed
errors. Overrides attached to a static candidate are sorted by priority, load
order, declaration order, and dynamic ID.

### Quotas

The registry currently limits each component to 256 items, each dynamic syntax
to 64 patterns and 64 KiB of pattern text, and metadata to 64 entries. Patterns
are parsed immediately with the Catalog's generated plural rules.

## Source Layout

| Module | Responsibility |
| --- | --- |
| `model` | normalized syntax and registry data structures |
| `catalog` | indexes and semantic queries |
| `dynamic` | dynamic registration, override, ranking, snapshots, and rollback |

All public model types are re-exported from the crate root.

## Testing

```sh
cargo test -p syntaxes --locked
```

Catalog tests cover class/type assignability, EventValue inheritance, overload
indexes, converters, and aliases. Dynamic tests cover invalid patterns,
duplicate IDs, deterministic ordering, override targets, cycles, freeze,
savepoints, stale revisions, and unload behavior.

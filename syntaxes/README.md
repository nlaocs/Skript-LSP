# syntaxes

[日本語](README.ja.md)

`syntaxes` is the format-independent runtime model for everything registered by
Skript and its addons. It is the boundary between serialized SSG data and
parsing or LSP consumers.

The crate does not load or validate the SSG snapshot format, and its normalized
model does not know which Skript version produced the data. [`ssg`](../ssg/)
owns those concerns and constructs a `Catalog`. The optional
`CatalogSource::from_json_documents` helper parses caller-provided JSON for
source retention and indexing, but it does not validate SSG digests.

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

- Java class hierarchy, class kinds, and optional declared method metadata
- converters and comparators
- properties and property handlers
- arithmetic operators, operations, and differences
- aliases and normalized alias targets
- generated plural rules
- effective runtime language key/value entries

`Option` and `ResolutionState` preserve availability distinctions where the
normalized model exposes them. In SSG raw data, an omitted or JSON `null`
optional list is `None`, while a present empty array remains an empty
collection. Conversion normalizes several optional collections to empty
vectors and uses unresolved states for legacy expression metadata. The source
view retains the original bytes, including unknown JSON fields; unknown typed
enum values are still rejected by `ssg` deserialization.

## Catalog

`Catalog` owns normalized data and provides indexes and queries for common parser
operations:

- syntax by registration ID and category iterators
- type by code name
- function overloads by name
- EventValues inherited through event class hierarchy
- converters by source or destination class
- Java class and type assignability
- exact declared Java method signatures
- aliases, comparators, properties, arithmetic, and plural rules
- case-sensitive language lookup and deterministic language entry iteration

Class and EventValue traversal is cycle-safe. EventValues follow generated
resolution order and honor exclusion classes.

`CatalogParts` is the explicit constructor input. It is useful for loaders such
as `ssg` and for small isolated tests.

`CatalogSource` retains exact source-document bytes and indexes top-level JSON
objects by `registrationId` and `definitionId`. `CatalogSource::from_json_documents`
parses the supplied documents and computes a source digest, but does not verify
an SSG manifest or content digest. `Catalog::source()` is populated for
catalogs returned by `ssg::load`.

For class method metadata, `Class.methods == None` means that the metadata is
unavailable (for example, an older SSG schema), while `Some(empty)` means it was
available and no methods were declared. `declared_method_exists` returns
`None` when the class or its method metadata is unavailable; otherwise it
returns whether the exact parameter and optional return signature is present.

Language entries are available through `language_value` and `language_entries`.
An absent key returns `None`, an empty value returns `Some("")`, and iteration
is deterministic by key.

## IDs

Static syntax has two identities:

- `DefinitionId` groups the semantic syntax definition.
- `RegistrationId` identifies one concrete registration.

A definition may therefore map to multiple registrations. Overrides can target
either identity depending on whether they replace all forms or one exact
registration.

## Dynamic Syntax Registry

`DynamicSyntaxRegistry` overlays component-provided syntax on an immutable
`Arc<Catalog>`. The `parser-wasm` host exposes this overlay to WASM components
through the WIT adapter.

A dynamic ID is namespaced as:

```text
dynamic:<component-id>/<local-id>
```

Components can register definitions with patterns, kind, return metadata,
handler, free-form metadata, numeric priority, and `before`/`after`
constraints. They can also override static definitions or registrations.

### Lifecycle

- initialization updates become the baseline for future documents
- each newly begun document revision clones that baseline
- the parser host's Document/Preprocess hook handling may stage document-specific updates
- savepoints plus `rollback_to` restore parser candidate state
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

Catalog tests cover class/type assignability, common-type selection, EventValue
inheritance, overload indexes, converters, aliases, language lookup, literal
matching, source retention, differences, and declared-method probes. Dynamic
tests cover invalid patterns, duplicate IDs, deterministic ordering, override
targets, cycles, freeze, structure metadata, savepoints, stale revisions, and
unload behavior.

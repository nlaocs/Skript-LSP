# Declared bounds and storage inference

Ordinary type inference assumes the selected Java/Skript declarations obey their
published type contracts. A complete public bound such as `Player` includes all
its subtypes; it is not an enumeration of concrete runtime classes. Custom getter
implementations do not, by themselves, invalidate that bound.

The parser's effective expression type and a local binding after SET remain
separate. Core's standard-contract adapter calculates default `beforeChange`
(including Slot to ItemStack), ClassInfo clone dispatch, then a fresh ClassInfo
dispatch for `serializeAs`. Exact registrations, including later entries, precede
ordered superclass selection. A typed cloner may widen the input subtype to its
declared ClassInfo class. Unknown output contracts stay unresolved. No getter,
cloner or converter is executed by the LSP.
Each storage dispatch branch also constrains the value to its selected ClassInfo
class. If that class is already assignable to `serializeAs`, the standard
converter returns the same value; a wider input bound does not force a new type.

The current adapter covers the common standard storage API in Skript 2.12-2.15.
It consumes complete TypeRuntime v3 ancestry and accessor/clone/serialization
metadata. Other versions and old/incomplete metadata do not gain this inference.
This range is an adapter selection, not a claim that every release has completed
runtime integration tests. Source hashes fence identity and invalidation; there
is no event, return-type or artifact allowlist for this path.

## Public contract and guarantees

Variable-change metadata version 5 adds optional `declaredStorage`:

```json
{"version":1,"basis":"skript.standard-storage-contract.v1",
 "types":["example.Value"],"complete":true,"maybeUnset":true,
 "preservesLocalBindings":false}
```

These are complete non-null public upper bounds, with absence kept separately.
The accepted provider and native semantic witness authenticate the metadata's
owner, subject and revision. The basis identifies its semantic assumptions; it
is not native-origin attestation. The existing `storageResult` takes precedence.
Neither a declared bound nor this record grants an identity/clone-input token or
purity. Unknown callback effects invalidate other local facts even when the
produced value's declared bound is useful. A scalar declaration is not a list,
global-storage, alias or persistent-state guarantee.
`preservesLocalBindings` describes only the calculated storage stage; it does
not certify that evaluating the getter is pure.

The internal expression input contract is bound to the source digest/snapshot and the
current selected summary's identity, effective bounds, completeness and scalar
cardinality. A later Transform cannot apply that input to changed bounds.
This supplements, and does not replace, the host's ownership/revision witnesses.
It records the checked default `beforeChange` path, not a computed storage result.
Storage inference runs only at the consuming scalar local SET when an existing
native `storageResult` is unavailable, keeping the two computations out of the
same expression hook's fuel budget.

A dynamic provider can instead supply the existing native-sealed callback-only
value-facts input (`classes: []`, `complete: false`, audited default-clone). Core
reads the current sealed summary's public upper bound independently and applies
the same prepared model. This does not promote that bound to a concrete runtime
class set, grant Core ownership to foreign metadata, or alter the Core-only
variable-transfer operation contract. Explicit malformed/foreign/duplicate Core
input records remain rejected rather than falling back to the callback input.

Write-site hints describe that write's post-state. Later reads also depend on
lookup/conversion/player-refresh semantics. Core publishes variable-read v2
`missingValue: "empty"` for the audited Skript 2.15.4 fixed-name scalar local
path: `VariableString.newInstance` creates `LiteralString`, whose MESSAGE mode
makes `getDefaultVariableNames` empty. An absent lookup therefore yields no
value, including when the script has default/global variables. Nullable bounds
may pass this lookup only if the existing refresh and requested-type checks
also pass; SET still applies its normal clone/storage contract. This preserves
nullability and never implies presence. Global/list/interpolated names,
unreviewed versions and v1 metadata retain conservative handling. General
default/global lookup is not closed merely by a stored nullable bound.
Branches keep unknown/uninitialized alternatives. Open
interfaces may overlap Slot or other registered cloner domains, so a computed
union can be wider than the advertised RHS type. This is not a guarantee that
Player alone is recovered on every environment.

## Bounded preparation and compatibility

Type rows are paged during the existing source-preparation lifecycle, before the
independent optional native-certification preparation. Complete hierarchy names
are fetched on demand from the normalized Catalog, without reading the entire
ClassHierarchy source document into that preparation budget. All existing fuel,
memory, byte, time and step limits remain. Failed/old preparation does not mint a
fact; source identity, preparation generation, component and current parse
witnesses continue to fence reuse.

Catalog-data capability **v6** adds `class-lineage(class-name)`. It returns the
complete declared ancestor closure including the queried class, or None for a
missing ancestor, cycle, more than 128 classes or more than 16 KiB of names.
Existing `declared-method-exists` distinguishes missing method metadata from an
empty declaration list. Core verifies the whole closure for the default storage
hook; a custom `beforeChange` whose output cannot be described by standard data
requires an addon contract.

The standard ExprEventExpression adapter also models WrapperExpression's
beforeChange forwarding to its internally constructed EventValueExpression.
It verifies the outer forwarding closure and the delegate's default hook. This
is a framework semantic adapter, not an event/type list: unknown event/value
registrations follow the same path. Other wrappers, subclasses with a different
delegate, or overridden hooks do not acquire that contract from a class suffix.
After the specific semantic resolvers, unchanged complete static declarations
can supply ordinary bounds without a custom resolver. This does not certify
arbitrary init/getter effects or promote a partial/dynamic bound. Native dynamic
syntax uses matching `dynamic:<component>/<local>` definition/registration IDs;
this fallback leaves those declarations to the registering provider. It does not
reassert the same bound as a second Core author. Specific standard resolvers,
static SSG declaration fallback and native single-author input seals are unchanged.

The declaration lookup was added without changing the WIT 0.48 / ABI30 record
layouts at that time. The current world is WIT 0.49 / ABI31: rebuild all components
together, including the mandatory semantic-transfer export. Older worlds are
incompatible. The current Core also requires its host imports; an old host fails
linking rather than silently substituting an older inference rule. Third-party semantics remain provider-owned, with no
Java- or addon-specific inference branches in the host.

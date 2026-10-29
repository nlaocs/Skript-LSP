# Current-flow semantic transfers

WIT **0.49.0 / ABI 31.0** adds the `semantic-transfer` guest export. The
`parser.semantic-transfer` v1 capability opts a component into these queries.
Every component must implement the export to match the world; a component that
does not provide transfer semantics can return `None` without declaring the
capability. Rebuild the host, Core and external addons together. Older worlds or
ABI manifests are rejected, rather than mixed with the new record layout.

This interface computes **complete public type upper bounds** for an operation
selected by the current parse. It does not execute Java, prove a runtime object's
origin or exact class, certify callback purity, or guarantee that an operation
will succeed. The provider owns its semantics; the host authenticates ownership,
current identity, lifetime and resource bounds. Editing suggestions and display
metadata do not grant this authority.

## Operation selection and inputs

An adopted node carries owner-authenticated `parser.semantic-transfer.v1`
metadata:

```json
{"version":1,"operation":"example.local-read.v1","parameters":"{}"}
```

The operation name and JSON parameter string are provider-owned. The host
requires a valid semantic issuer or a sealed selected-fact composition. Merely
copying the metadata key or an owner name does not make a query eligible.

Parameters must encode any document-dependent semantic assumptions needed by
the operation. A data-only query must not recover missing current context from
old parse-private memory: document IDs and revision numbers can repeat after a
close/reopen. Core's initial operations use the authenticated parameters,
current bound and immutable prepared source model.

The frontend supplies a current `type-bound`: a complete scalar set of at most
64 distinct nonempty class names, with a separate `maybe-unset` flag.
`java.lang.Object`, partial sets and plural inputs are not accepted by v1.
An empty set with `maybe-unset=true` represents a known empty value; it is not an
Unknown result. Each class name is limited to 1,024 bytes and cannot contain
control characters. These are public upper bounds, so a named interface can
include runtime subclasses that were not enumerated in the snapshot.

The host supplies the document ID/revision, node ID, syntax context, immutable
SSG source digest, snapshot ID and the selected provider's preparation generation. The
query is valid only for the exact retained parse and provider binaries. The
frontend never obtains an input type from a prior hover, an editing proposal or
an older revision.

## Four separate results

| Output | Meaning |
|---|---|
| `value` | Complete upper bound returned by this operation |
| `source` | Source binding's poststate across every branch: unchanged, known, or Unknown |
| `source-writeback` | Optional bound written on a writeback branch; not the source's unconditional poststate |
| `effects` | Whether all existing frontend binding facts, including the source, are preserved or invalidated |

An absent `source-writeback` means no complete writeback bound is available. It
does not prove that no write can occur. A known returned value may remain useful
even when callback effects make the source's poststate Unknown.

The frontend applies binding invalidation first, a known source poststate next,
and a newly computed destination SET last. Thus self-copy does not accidentally
restore the pre-read fact. A poststate type alone does not certify that a previous
authored definition still reaches later reads. Evaluation-context contracts
remain separate from these binding effects.

## Core's initial version adapter

Core implements the following operations for the audited **Skript 2.15.4**
standard path.

The adapter follows the versioned upstream implementations of
[`Variable.getRaw` / `convertIfOldPlayer`](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/ch/njol/skript/lang/Variable.java#L322-L405),
[`Expression.beforeChange`](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/ch/njol/skript/lang/Expression.java#L390-L415),
[`Variables.setVariable`](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/ch/njol/skript/variables/Variables.java#L543-L568)
and [`EffChange.execute`](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/ch/njol/skript/effects/EffChange.java#L282-L304).

Available operations:

- `skript.standard-variable-read.v1`: fixed-name local scalar lookup with an
  explicit empty missing-value contract and requested type `Object`.
- `skript.standard-local-set.v1`: local scalar SET whose evaluated delta carries
  the authenticated standard default-clone `beforeChange` contract.

The read adapter follows the standard `Variable.convertIfOldPlayer` operation.
Its `org.bukkit.entity.Player` domain is an API type referenced by that audited
implementation, not a list of permitted input types or events. All complete
input bounds use the same overlap rule. Known immutable non-refresh types retain
their input bound and presence. Where refresh may apply, the adapter joins
enabled/disabled settings, keeps the raw returned bound separate from the
conditional serialize-only writeback, and conservatively invalidates affected
bindings. An unknown writeback does not erase an otherwise complete returned
bound. This does not promise an exact CraftBukkit implementation class.

Destination storage is then recomputed using the existing ClassInfo,
`beforeChange`, clone and `serializeAs` model. It is not a copy of the RHS label.
An empty SET delta deletes the binding before clone processing; identity-only
storage preserves definite presence, and nullable or effectful paths retain the absence
allowed by their contracts.

The initial adapter does not extend these guarantees to globals/default
fallback, lists, interpolated names, unsupported versions, incomplete/old
metadata or opaque custom transforms. Such paths keep their conservative result.
Custom semantics can supply a separate provider-owned operation through the
same public interface; neither host nor Core selects them by an addon name.

## Lifetime and bounded execution

Transfer calls permit no StateStore, parse, dynamic-registration or publication
imports. The host records attempted forbidden imports even if a guest ignores
their error. Current source, document fence, provider availability/binary and
preparation generation are checked around execution. Close/reopen, source or
provider changes, restart and cancellation cannot rebase an old result onto a
new authority. Materialized semantic UI results carry the corresponding
revocable certificate.

The existing default per-call **20,000,000 fuel** budget is unchanged. A transfer
session additionally limits:

- 2,048 selected operations and 2 MiB of retained operation metadata;
- 32 KiB per metadata entry, 256-byte operation names and 8 KiB parameters;
- 2,048 calls and 5 seconds of accumulated query-call wall time, with each
  deadline clipped to the remaining allowance;
- 2 MiB of accounted input/output data and a 128 KiB accounted output per call.

The byte accounting sums retained class-name bytes and fixed record allowances;
it is not a measurement of physical WIT copies or process memory. The existing
Wasm store limits still apply. Unsupported, stale or quota-exhausted queries
return no transfer result. Cancellation and guest/host faults retain their
separate failure paths; they are not known-empty values or successful type
certification.

Source preparation still owns bounded source validation and generation changes;
see [its lifecycle](source-preparation.md). Ordinary declaration/storage bounds
remain distinct from the stronger
[standard native storage evidence](standard-native-storage.md).

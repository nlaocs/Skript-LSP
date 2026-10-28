# Storage type guarantees

A value's advertised expression type and its type after assignment are separate facts. Skript may run `beforeChange`, clone through the selected ClassInfo, and apply `serializeAs` even for a local variable. Core only supplies a post-storage type when these operations have an authenticated guarantee. LSP consumes that provider result; it does not recognize expression names or substitute the right-hand-side type automatically.

## Supported native-reference profile

The initial implementation-domain model is source-audited for Skript 2.15.4 and Paper 1.21.11 build 132, commit c5eb079. It requires the captured implementation JAR identity and the complete matching Skript/SSG plugin inventory. A different server, generator build, extra plugin, old snapshot or runtime version does not inherit the guarantee.

SSG's optional TypeRuntime version 3 records guarded ClassInfo accessor provenance, the represented class, class kind, and a bounded declared superclass/interface closure. The server implementation class and JAR digest participate in snapshot identity. Raw snapshot identity is also checked separately from the generator's data-file digest.

Audited Player/Entity command parser paths and the initial PlayerEvent/CommandEvent getters produce native server wrappers or no value. Their non-null domain has a concrete CraftEntity upper bound intersected with the public expression type. This is a return-path model derived from the implementation, not a sample of CraftPlayer objects or a promise about every implementation of Player. Arbitrary command defaults and unverified event getters do not gain this model.

The common checker rejects possible Slot interception, checks exact runtime-class registrations before the ordered superclass search, and verifies all potentially selected clone and serialization dispatches. Even registrations after the first guaranteed ordered match matter for exact-class lookup. Missing accessor evidence and overlapping unknown interfaces fail closed. Serialization to a supertype is identity only when the value is already guaranteed to satisfy that type; it does not widen the published type.

Both the modern and legacy property families use the common storage checker. A fallback resolver handles only an authenticated fixed declared class with one consistent, complete type candidate and known multiplicity. It does not overwrite a preceding provider's different effective result, replace a specific semantic resolver, or certify getter effects. Every class between the concrete property and the default storage hook must have complete method evidence with no beforeChange override. Dynamic/incomplete bounds and unknown or overridden methods remain unsupported. UUID properties in the captured profile can consequently retain `uuid?` after local assignment, including offline-player inputs; presence is not inferred from the declared class.

An authenticated property or native function with a concrete declared class bound may use the same dispatch checker. Open interfaces need additional producer evidence. Existing independent closed-family guarantees remain available.

## Presence, scope and presentation

These new reference guarantees describe a complete scalar type bound, independently of presence. A Player assignment is currently rendered as `player?` until presence is separately proved, including required arguments and player-only command contexts. This work does not add branch-presence narrowing.

Write-site hints describe the state after that particular assignment. A later assignment does not merge backwards into earlier hints. Ordinary reads have hover information only. Invalidation history is not a current type guarantee. Unknown callbacks are not assumed pure.

## Conservative limits

- Unsupported addons/providers, different runtime artifacts, custom cloners or serialization conversions, and incomplete provenance retain Unknown unless another existing contract proves their result.
- The backend model covers local scalar storage. It does not certify global writes, lists, arbitrary command defaults, or interprocedural/variable-copy propagation of native implementation domains.
- Declared Java superclass/interface closure does not establish general array covariance. A registry containing Array or Primitive TypeRuntime records disables this initial compact model rather than assuming complete assignability.
- Proof inputs are prepared once per source/snapshot in an accepted structure hook. Consumers recheck runtime/source identity and use bounded caches. A standalone body parse or fresh Store without preparation receives no additional guarantee from this model; independent primitive/closed-family contracts may still prove a type. It does not transfer the full registry inside that body hook. No production fuel ceiling is increased.
- The matching new snapshot is required along with Core and host. Merely replacing the executable while retaining an old snapshot cannot activate this guarantee.

## Macro source boundaries

Native facts are bound privately to both the accepted AST and its source map before AST addon phases. Original locations require an exact, unambiguous projection in the active witnessed forest; unused virtual fragments from earlier tree edits do not count as duplicate execution. Missing raw statements, reordered bodies, unavailable regions and generated writes still invalidate affected knowledge conservatively. Macro expansion does not grant editable rename coverage.

An option expansion no longer makes every unaffected sibling statement opaque. The expanded line itself is still generated: the current TreeEdit contract replaces the whole line and assigns a new syntax context, without a certificate for preserved source slices or caller context. Such a target receives no source inlay, and its write cannot preserve unproved reaching facts. Matching text alone does not restore original binding identity. A generic source/context preservation contract is needed before this limitation can be removed.

## Mixed-registry evidence contract checkpoint

`core-library/src/storage/evidence.rs` implements a version-1 **structural and
conditional** contract. It is not a supported addon-data input or a new WIT
capability. Neither a checked capture nor a conditional storage result can issue
storage metadata, `ResultType`, a native seal, or a native-event origin token.
The existing audited standard profile remains the only backend model enabled by
this work. Current mixed captures with missing evidence still produce Unknown.

The contract separates:

- The retained Catalog source/snapshot identity and an independently supplied
  collection generation. A capture must declare a complete ordered registry,
  with equal start/end generations and one entry for every retained Type record.
- Registration identity, stable base-getC geometry, exact-map/ClassInfo instance
  correspondence, and explicit Java Class/loader/code-origin identities. The
  current name-based Catalog cannot represent same-name classes from different
  loaders; those captures are rejected. Complete ancestry is checked against the
  retained type-runtime-v3 projection, without inventing missing relations.
- A producer claim's public bound P and concrete implementation upper bound U.
  Its source-model/artifact/route reference and current owner, source, generation,
  document, revision, analysis lifetime, syntax context, expansion and exact byte range remain
  separate. Missing origin, opaque beforeChange, incomplete/multiple domains and
  stale subjects cannot produce even a conditional success. Nullable state is
  retained rather than inferred from type names.
- The conditional `P intersection subtypes(U)` storage calculation. It checks
  every potentially applicable exact registration, including later entries, and
  ordered candidates through a guaranteed match. Unrelated concrete cloners may
  be excluded; overlapping interfaces, custom applicable cloners and real
  serializeAs conversions require additional evidence and remain unsupported.

The shared `RegistryGeometry` checks are also used by the legacy source-audited
path. The shared borrowed scope helper is used by current private context
receipts without adding string allocations or changing their lifetime. Neither
reuse turns a legacy snapshot ID into a collection generation, establishes
ClassInfo instance/loader observations, or authenticates an event's origin.

These checks assume honest, independently authenticated inputs. A `complete`
flag, a digest-shaped string, matching opaque IDs, private state ownership or a
component pin is not proof of Java behavior. In particular, the normal Paper
join source route's CraftPlayer bound does not prove that every same-class event
in a mixed environment came from that route. Synthetic positive tests exercise
the conditional checker only; they do not certify a real mixed server.

The collector authentication integration is deliberately pending. The SSG
observation checkpoint below adds actual identity correspondence, but does not
establish the needed mutation-generation/getter-source closure, and the current
context has no native-origin attestation. No server/JAR run, data recollection or
normal/demo deployment is included in these offline checkpoints.
Before a future integration can publish results, it must authenticate both the
capture and producer origin, preserve source/revision invalidation and rollback,
and validate the user's actual mixed environment. The current strict gate must
not be replaced by structural success alone.

### SSG boundary observations

The modern SSG exporter optionally includes `Manifest.registryCapture`,
observation schema version 1, while retaining snapshot schema 7 and its 21 files.
`evidence::observation` accepts that distinct schema from a source-bound raw
Manifest, validates Class/loader/reference limits and correspondence with the
retained Types/EventValues rows, and binds any cached result to both the source
identity and the exact raw Manifest SHA-256. The semantic snapshotId algorithm
does not include optional Manifest observations, so it is insufficient alone.

The observer retains actual ordered ClassInfo and exact-map instance identities,
loader parentage, Class-valued runtime fields, code-source-file/module metadata
and getter implementation Class/method signatures. Identity tokens are local
to one capture and are not persistent object addresses. Hidden lambdas and
unavailable origins stay unknown. Code-source hashes are not active transformed
bytecode attestations. Events and EventValues share one collected row list.

Its generation mode is explicitly `boundary-observed-no-mutation-epoch`. Equal
start/end observations do not exclude intervening mutations. Getter coverage is
only `retained-event-value-rows`, not the entire live event registry. This schema
has no conversion into `CheckedCaptureRegistry`'s stronger expected_generation,
producer authority, ResultType or native seals. The cross-project test uses
SSG-generated self-authored fixtures, not an actual Paper capture. No new WIT/API
capability or automatic mixed-storage publication is enabled by these tests.

Observation payloads are capped at 512 KiB by SSG and raw Manifests at 2 MiB in
this offline receiver. Consumers with an older 64-KiB Manifest read limit may
not inspect larger observations; they must not treat a truncated read as a
complete observation. Existing storage support still requires the matching
audited exporter/runtime profile and is not automatically widened to a new JAR.

## Source references

- [Legacy SimplePropertyExpression](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/ch/njol/skript/expressions/base/SimplePropertyExpression.java)
- [Legacy PropertyExpression](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/ch/njol/skript/expressions/base/PropertyExpression.java)
- [Skript Expression.beforeChange](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/ch/njol/skript/lang/Expression.java)
- [Skript Classes dispatch and clone](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/ch/njol/skript/registrations/Classes.java)
- [Skript variable storage](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/ch/njol/skript/variables/Variables.java)
- [Skript converters](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/org/skriptlang/skript/lang/converter/Converters.java)
- [Player parser](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/org/skriptlang/skript/bukkit/types/PlayerClassInfo.java)
- [Entity parser](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/org/skriptlang/skript/bukkit/types/EntityClassInfo.java)
- [Event getter registrations](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/ch/njol/skript/classes/data/BukkitEventValues.java)
- [Paper CraftServer](https://github.com/PaperMC/Paper/blob/c5eb079/paper-server/src/main/java/org/bukkit/craftbukkit/CraftServer.java)
- [Paper CraftEntity](https://github.com/PaperMC/Paper/blob/c5eb079/paper-server/src/main/java/org/bukkit/craftbukkit/entity/CraftEntity.java)

## Scalar change dispatch

The provider change contract (version 4) distinguishes definite binding operations from runtime-dependent dispatch. The semantic layer resolves conditional rules after evaluating the delta and before invalidating target knowledge. It uses current, inferred, complete, present scalar inputs; historical candidates, nullable/default paths and unknown effects cannot satisfy those rules. Unresolved dispatch creates neither a binding-write hint nor an assignment definition. It still invalidates unproved current knowledge; changing an object is not a promise that callbacks are pure. SET/DELETE and list binding roles remain independent of local storage precision, including global navigation.

The captured standard profile currently certifies scalar ADD for closed boxed numeric inputs when the actual arithmetic registration is uncontested and delta cloning and result storage are identity-preserving. The implementation follows direct operation lookup without converters. Integer overflow can promote the result, so the complete result is Long or Double, not always Long. Repeated updates preserve that union. Static rules, including negative results, are cached only within the immutable source/snapshot model. Abstract/custom Number, missing values, global arithmetic, unknown callbacks and unsupported operation registrations remain unresolved. No spelling-based distinction between add and give is made.

The source basis is Skript 2.15.4 Variable.change, EffChange.execute and DefaultOperations, under the same exact runtime profile described above. This is static type/effect evidence, not runtime execution or general interprocedural analysis.

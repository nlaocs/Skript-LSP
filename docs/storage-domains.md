# Storage type guarantees

A value's advertised expression type and its type after assignment are separate facts. Skript may run `beforeChange`, clone through the selected ClassInfo, and apply `serializeAs` even for a local variable. Core only supplies a post-storage type when these operations have an authenticated guarantee. LSP consumes that provider result; it does not recognize expression names or substitute the right-hand-side type automatically.

## Supported native-reference profile

The initial implementation-domain model is source-audited for Skript 2.15.4 and Paper 1.21.11 build 132, commit c5eb079. It requires the captured implementation JAR identity and the complete matching Skript/SSG plugin inventory. A different server, generator build, extra plugin, old snapshot or runtime version does not inherit the guarantee.

SSG's optional TypeRuntime version 3 records guarded ClassInfo accessor provenance, the represented class, class kind, and a bounded declared superclass/interface closure. The server implementation class and JAR digest participate in snapshot identity. Raw snapshot identity is also checked separately from the generator's data-file digest.

Audited Player/Entity command parser paths and the initial PlayerEvent/CommandEvent getters produce native server wrappers or no value. Their non-null domain has a concrete CraftEntity upper bound intersected with the public expression type. This is a return-path model derived from the implementation, not a sample of CraftPlayer objects or a promise about every implementation of Player. Arbitrary command defaults and unverified event getters do not gain this model.

The common checker rejects possible Slot interception, checks exact runtime-class registrations before the ordered superclass search, and verifies all potentially selected clone and serialization dispatches. Even registrations after the first guaranteed ordered match matter for exact-class lookup. Missing accessor evidence and overlapping unknown interfaces fail closed. Serialization to a supertype is identity only when the value is already guaranteed to satisfy that type; it does not widen the published type.

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

## Source references

- [Skript Expression.beforeChange](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/ch/njol/skript/lang/Expression.java)
- [Skript Classes dispatch and clone](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/ch/njol/skript/registrations/Classes.java)
- [Skript variable storage](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/ch/njol/skript/variables/Variables.java)
- [Skript converters](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/org/skriptlang/skript/lang/converter/Converters.java)
- [Player parser](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/org/skriptlang/skript/bukkit/types/PlayerClassInfo.java)
- [Entity parser](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/org/skriptlang/skript/bukkit/types/EntityClassInfo.java)
- [Event getter registrations](https://github.com/SkriptLang/Skript/blob/2.15.4/src/main/java/ch/njol/skript/classes/data/BukkitEventValues.java)
- [Paper CraftServer](https://github.com/PaperMC/Paper/blob/c5eb079/paper-server/src/main/java/org/bukkit/craftbukkit/CraftServer.java)
- [Paper CraftEntity](https://github.com/PaperMC/Paper/blob/c5eb079/paper-server/src/main/java/org/bukkit/craftbukkit/entity/CraftEntity.java)

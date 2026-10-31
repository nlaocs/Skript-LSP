#![doc = include_str!("../README.md")]
#![warn(rustdoc::broken_intra_doc_links)]

use std::collections::{BTreeMap, BTreeSet};

/// Language preprocessing shared by native editing and guest ClassInfo parsing.
pub mod type_language;

#[cfg(feature = "host")]
/// Wasmtime-generated Component Model bindings.
///
/// This module is generated from the WIT package and is intentionally not a
/// hand-authored API. Prefer the stable host types re-exported from this crate.
#[allow(missing_docs)]
pub mod bindings;
#[cfg(feature = "host")]
/// Native Wasmtime host, component registry, hook dispatch, and macro pipelines.
pub mod host;
#[cfg(feature = "host")]
/// Transactional state shared by parser hooks and addon components.
pub mod state;

#[cfg(feature = "host")]
pub use host::{
    AstMacroCall, AstMacroRequest, AstMacroResult, DependencyUnavailableRegion,
    DocumentCancellationToken, DocumentExecutionStatus, DocumentParseError, DocumentParseRequest,
    DocumentParseResult, DocumentParseStage, DocumentParserConfig, ExtensionFaultRegion,
    DeclarationInputs, DeclarationSource, DeclarationExports, DeclarationLocations,
    HostConfig, HostError, ParserHost, RuntimeAliasCapabilities, RuntimePlugin, RuntimeProfile,
    RuntimeSnapshotCapabilities, RuntimeSyntaxKindCapabilities, TreeMacroCall, TreeMacroRequest,
    TreeMacroResult, WasmConditionParseResult, WasmEffectParseResult, WasmExpressionParseResult,
    WasmPatternMatchResult, WasmSectionParseResult, WasmStructureParseResult,
};
#[cfg(feature = "host")]
pub use state::{ParseTransaction, StateError, StateStore};

/// Exact host/guest handshake version implemented by this crate.
pub const ABI_VERSION: AbiVersion = AbiVersion::new(31, 0);

/// Owner-authenticated, data-only transfers of current public type bounds.
pub const CAPABILITY_SEMANTIC_TRANSFER: &str = "parser.semantic-transfer";

/// Authenticated guard predicates, conditional read identity and binding-value effects.
pub const CAPABILITY_TYPE_GUARDS: &str = "parser.semantic-type-guards";

/// Bounded owner-private source preparation, independent of document revision.
pub const CAPABILITY_SOURCE_PREPARATION: &str = "parser.source-preparation";

/// Opt-in v1 entry facts bound to an accepted current Structure header. Only
/// explicitly selected owner metadata is independent of later body writers;
/// this grants neither whole-body purity nor general cross-provider authority.
pub const CAPABILITY_SEMANTIC_ENTRY_FACTS: &str = "parser.semantic-entry-facts";

/// Read-only, component-binary-bound immutable dataset inputs. Version 2 keeps
/// the v1 wire API and additionally rejects any explicitly configured source
/// for the loading component whose binary pin differs, before initialization.
/// Omitted sources remain optional; other components' sources are unaffected.
pub const CAPABILITY_ADDON_DATA: &str = "parser.addon-data";

/// Accepted root-declaration exports of bounded, unverified identifier proposals.
/// The JSON contract is documented in the public SDK README.
pub const DOCUMENT_COMPLETIONS_METADATA: &str = "skript.editor.document-completions";

/// Explicit immutable/local semantic hook result contract for incremental analysis.
pub const CAPABILITY_INCREMENTAL_LOCAL_SYNTAX: &str = "parser.incremental-local-syntax";
/// Explicit immutable Structure Enter hook reuse contract, separate from local syntax v1.
pub const CAPABILITY_INCREMENTAL_STRUCTURE_HEADER: &str = "parser.incremental-structure-header";
/// Explicit opt-in native body reuse contract; unknown subscriptions stay cold.
pub const CAPABILITY_INCREMENTAL_STRUCTURE_BODY: &str = "parser.incremental-structure-body";

/// Component-wide deterministic project semantic promise. Version 1 forbids
/// post-initialization dynamic changes. Version 2 additionally allows bounded,
/// document-private declarations rebuilt deterministically from the current
/// complete document pipeline; mutable cross-document state and hidden guest
/// history remain forbidden. Unknown future versions imply neither promise.
pub const CAPABILITY_PROJECT_STABLE_SEMANTICS: &str = "parser.project-stable-semantics";

/// Opt-in final-RawTree declaration discovery before the document registry freezes.
/// Version 1 uses a Document/Parser carrier with an invocation-owned paged tree
/// view, immutable input and diagnostics-only output effects, plus private Parse
/// state and document-scoped dynamic registration.
/// Providers must require project-stable-semantics v2. A declaration prototype
/// does not certify an executable, pure or error-free implementation body.
pub const CAPABILITY_DOCUMENT_DECLARATIONS: &str = "parser.document-declarations";

/// Opt-in native project header export/import with owner, source and provider fences.
/// Version 1 extends only the document-declarations carrier; it does not certify
/// executable getter bodies, purity or runtime values. Old hosts reject this requirement.
pub const CAPABILITY_PROJECT_DECLARATIONS: &str = "parser.project-declarations";

/// Opt-in v1 stability of authenticated ASCII identifier leaves edited together
/// at Function declaration/call names or static scalar local Variable names.
/// After the editor verifies complete bindings, source geometry and collisions,
/// changing these leaves may only rename those identities. Macros, matching and
/// semantic hooks must not activate new transformations, reinterpret surrounding
/// structure or create/remove unrelated declarations/references based on either
/// spelling. This includes previously no-op macros. Already expanded sources,
/// dynamic/list names and parameter-label edits are outside this contract.
/// Every enabled component must promise exactly v1; project stability and a
/// current revocable runtime certificate are separate requirements.
pub const CAPABILITY_STATIC_NAME_RENAME_STABILITY: &str = "parser.static-name-rename-stability";

/// Opt-in v1 editing contract for Expression/Condition/Effect/Section grammars.
/// For exact unchanged source, the initialized Catalog exhausts registered
/// pattern languages and native typed-slot requirements of these four kinds.
/// Matching hooks, capture routing, macros and additional parses must not
/// broaden/reinterpret them or introduce document-dependent patterns/overrides.
/// Native variable/literal/function/group/list/arithmetic parsing is unchanged;
/// opaque/intrinsic expression forms still require an independent editing proof.
/// Structure registrations and ordinary Function declarations are independent.
/// Static expression types/cardinality may change only via declared registered
/// semantic handlers; Dynamic/Unresolved metadata is never a negative proof.
/// Every loaded component must opt in. Absence/future versions mean Unknown.
pub const CAPABILITY_COMPLETION_CATALOG_PATTERNS: &str = "parser.completion-catalog-patterns";

/// Opt-in v1 closure of the child positions belonging to an editing proposal.
/// A proposal names its own definition, not another leaf which happens to consume
/// identical source. Nested proposal nodes in registered expressions can occur
/// only in native typed captures or declared capture_parsers. Other expression
/// leaves consume their whole range; Custom/additional-parse hooks must not add
/// undeclared proposal child positions. Native intrinsic/group/list/arithmetic
/// and interpolation paths still require independent evidence. This is not a
/// promise that a different whole-range interpretation rejects the source.
/// Every component must opt in; provider failure/unknown versions revoke proof.
pub const CAPABILITY_COMPLETION_CANDIDATE_BINDINGS: &str = "parser.completion-candidate-bindings";

/// Opt-in v1 editing contract for changing only an authenticated static scalar
/// local Variable identity within a fixed native registration/pattern/capture.
/// Its read syntax context, leaf type/shape and parent evaluation prefix up to
/// that lookup do not depend on the scalar identity. Only identity/value may
/// change. The host must separately prove that replacement cannot select another
/// Catalog parent, literal/regex route or capture. This does not promise a
/// type or preservation after the lookup, and never covers lists, interpolation
/// or source transformations. Every loaded component must opt in; unknown/future
/// versions provide no evidence. A current authenticated lookup and current flow
/// facts are still required; header types alone are insufficient.
pub const CAPABILITY_COMPLETION_STATIC_LOCAL_SUBSTITUTION: &str =
    "parser.completion-static-local-substitution";

/// Permits discarding the entire guest Store after observed cancellation.
/// Initialization is deterministic and uses immutable Catalog queries only;
/// StateStore/dynamic call attempts are rejected. Losing guest memory/globals
/// and reinitializing must preserve all future semantics. Host state is retained.
/// Without this opt-in cancellation waits for normal guest return; fuel and
/// ordinary addon deadlines remain enforced.
pub const CAPABILITY_EPOCH_CANCELLATION_RESTART_SAFE: &str =
    "parser.epoch-cancellation-restart-safe";

/// Capability ID for typed parser hook subscription and dispatch.
pub const CAPABILITY_HOOKS: &str = "parser.hooks";
/// Capability ID for transactional scoped addon state.
pub const CAPABILITY_STATE_STORE: &str = "parser.state-store";
/// Capability ID for read-only source snapshot and Catalog relation queries.
pub const CAPABILITY_CATALOG_DATA: &str = "parser.catalog-data";
/// Capability ID for runtime syntax registration and override.
pub const CAPABILITY_DYNAMIC_SYNTAX: &str = "parser.dynamic-syntax";
/// Capability ID for source-text preprocessing macros.
pub const CAPABILITY_TEXT_MACRO: &str = "parser.macro.text";
/// Capability ID for lossless RawTree transformation macros.
pub const CAPABILITY_TREE_MACRO: &str = "parser.macro.tree";
/// Capability ID for hygienic parsed-AST macros.
pub const CAPABILITY_AST_MACRO: &str = "parser.macro.ast";
/// Capability ID for parser context updates emitted by hooks.
pub const CAPABILITY_CONTEXT_UPDATES: &str = "parser.context-updates";
/// Capability ID for addon-requested recursive parse work.
pub const CAPABILITY_ADDITIONAL_PARSE: &str = "parser.additional-parse";
/// Capability ID for CoreLibrary and addon Expression leaf parsers.
pub const CAPABILITY_EXPRESSION_PARSER: &str = "parser.expression";
/// Root-only registered Expression family selection for recursive parse requests.
pub const CAPABILITY_EXPRESSION_ROOT_SELECTION: &str = "parser.expression-root-selection";
/// Capability ID for omitted typed capture default providers.
pub const CAPABILITY_DEFAULT_EXPRESSION: &str = "parser.default-expression";
/// Capability ID for Condition semantic hooks.
pub const CAPABILITY_CONDITION_PARSER: &str = "parser.condition";
/// Capability ID for Effect lifecycle hooks and candidate replacement.
pub const CAPABILITY_EFFECT_PARSER: &str = "parser.effect";
/// Capability ID for Section lifecycle hooks and scoped child context.
pub const CAPABILITY_SECTION_PARSER: &str = "parser.section";
/// Capability ID for top-level Structure lifecycle and EntryValidator hooks.
pub const CAPABILITY_STRUCTURE_PARSER: &str = "parser.structure";
/// Owned Structure metadata containing a JSON list of {"path":["entry"],"mode":"trigger"|"conditions"}.
/// Paths select Section entries in the registration's EntryValidator, never source spelling guesses.
pub const STRUCTURE_ENTRY_BODY_MODES_METADATA: &str = "structure.entry-body-modes.v1";
/// Owned outline metadata on an accepted AST node. JSON v1 contains a nonempty
/// name, an LSP SymbolKind number (1..=26), optional detail, and selection
/// {start,end} UTF-8 offsets relative to the node's native header. Consumers
/// require the current unique provider witness and exact source projection.
/// Full extent and hierarchy come from native RawTree/AST, never this record.
/// Missing, conflicting or unknown versions make no outline claim.
pub const DOCUMENT_SYMBOL_METADATA: &str = "skript.semantic.document-symbol";
/// Requests every SSG type option in a registered Expression payload.
pub const REGISTERED_CONTEXT_ALL_TYPE_OPTIONS: &str = "expression.type-options.all";
/// Immutable native annotation mapping positive SSG input-cardinality evidence to a capture slot.
pub const REGISTERED_METADATA_MULTIPLICITY_SOURCE: &str = "host.expression.multiplicity-source";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Major/minor ABI version compared exactly during component initialization.
pub struct AbiVersion {
    /// Breaking-contract generation.
    pub major: u16,
    /// Compatible feature generation within the same major ABI.
    pub minor: u16,
}

impl AbiVersion {
    /// Creates an ABI version pair.
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }
}

impl std::fmt::Display for AbiVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Host capability and independently versioned feature contract.
pub struct Capability {
    /// Stable dotted capability identifier.
    pub id: String,
    /// Feature contract version for this identifier.
    pub version: u32,
}

impl Capability {
    /// Creates an advertised capability.
    pub fn new(id: impl Into<String>, version: u32) -> Self {
        Self {
            id: id.into(),
            version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Component requirement checked against the host profile.
pub struct CapabilityRequirement {
    /// Stable dotted capability identifier.
    pub id: String,
    /// Lowest feature contract version understood by the component.
    pub minimum_version: u32,
    /// Whether absence or an older version must reject initialization.
    pub required: bool,
}

impl CapabilityRequirement {
    /// Creates a requirement that must be satisfied.
    pub fn required(id: impl Into<String>, minimum_version: u32) -> Self {
        Self {
            id: id.into(),
            minimum_version,
            required: true,
        }
    }

    /// Creates a requirement that may be absent or older.
    pub fn optional(id: impl Into<String>, minimum_version: u32) -> Self {
        Self {
            id: id.into(),
            minimum_version,
            required: false,
        }
    }
}

/// Validates one side of the ABI negotiation.
///
/// The same function is used by the host for an addon manifest and by a guest
/// for the host profile passed to its initialization hook. ABI versions must
/// match exactly. Required capabilities must exist at or above their minimum
/// version; optional requirements never reject negotiation.
///
/// # Examples
///
/// ~~~
/// use parser_wasm::{
///     validate_compatibility, AbiVersion, Capability,
///     CapabilityRequirement, CompatibilityError, CAPABILITY_HOOKS,
/// };
///
/// let available = [Capability::new(CAPABILITY_HOOKS, 2)];
/// let requirements = [
///     CapabilityRequirement::required(CAPABILITY_HOOKS, 1),
///     CapabilityRequirement::optional("parser.future-feature", 1),
/// ];
///
/// validate_compatibility(
///     AbiVersion::new(1, 5),
///     AbiVersion::new(1, 5),
///     &requirements,
///     &available,
/// )?;
///
/// let error = validate_compatibility(
///     AbiVersion::new(1, 5),
///     AbiVersion::new(2, 0),
///     &[],
///     &available,
/// ).unwrap_err();
/// assert!(matches!(error, CompatibilityError::AbiVersionMismatch { .. }));
/// # Ok::<(), CompatibilityError>(())
/// ~~~
///
/// # Errors
///
/// Returns [CompatibilityError] for an ABI mismatch, blank or duplicate
/// capability IDs, a missing required capability, or an insufficient required
/// capability version.
pub fn validate_compatibility(
    expected_abi: AbiVersion,
    actual_abi: AbiVersion,
    requirements: &[CapabilityRequirement],
    available: &[Capability],
) -> Result<(), CompatibilityError> {
    if expected_abi != actual_abi {
        return Err(CompatibilityError::AbiVersionMismatch {
            expected: expected_abi,
            actual: actual_abi,
        });
    }

    let mut available_by_id = BTreeMap::new();
    for capability in available {
        validate_capability_id(&capability.id)?;
        if available_by_id
            .insert(capability.id.as_str(), capability.version)
            .is_some()
        {
            return Err(CompatibilityError::DuplicateCapability {
                id: capability.id.clone(),
            });
        }
    }

    let mut requirement_ids = BTreeSet::new();
    for requirement in requirements {
        validate_capability_id(&requirement.id)?;
        if !requirement_ids.insert(requirement.id.as_str()) {
            return Err(CompatibilityError::DuplicateCapability {
                id: requirement.id.clone(),
            });
        }

        match available_by_id.get(requirement.id.as_str()).copied() {
            Some(actual) if actual < requirement.minimum_version && requirement.required => {
                return Err(CompatibilityError::CapabilityVersionTooOld {
                    id: requirement.id.clone(),
                    minimum: requirement.minimum_version,
                    actual,
                });
            }
            Some(_) => {}
            None if requirement.required => {
                return Err(CompatibilityError::MissingRequiredCapability {
                    id: requirement.id.clone(),
                    minimum: requirement.minimum_version,
                });
            }
            None => {}
        }
    }
    Ok(())
}

fn validate_capability_id(id: &str) -> Result<(), CompatibilityError> {
    if id.trim().is_empty() {
        Err(CompatibilityError::BlankCapabilityId)
    } else {
        Ok(())
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
/// Failure to negotiate an ABI or capability profile.
#[allow(missing_docs)] // Variant messages describe the exact handshake failure.
pub enum CompatibilityError {
    #[error("ABI version mismatch: expected {expected}, found {actual}")]
    AbiVersionMismatch {
        expected: AbiVersion,
        actual: AbiVersion,
    },
    #[error("capability ID must not be blank")]
    BlankCapabilityId,
    #[error("capability {id} is declared more than once")]
    DuplicateCapability { id: String },
    #[error("required capability {id} version {minimum} is unavailable")]
    MissingRequiredCapability { id: String, minimum: u32 },
    #[error(
        "capability {id} requires at least version {minimum}, but version {actual} is available"
    )]
    CapabilityVersionTooOld {
        id: String,
        minimum: u32,
        actual: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_abi_version_mismatches_explicitly() {
        for actual in [
            AbiVersion::new(2, 0),
            AbiVersion::new(21, 0),
            AbiVersion::new(22, 0),
            AbiVersion::new(24, 0),
        ] {
            let error = validate_compatibility(ABI_VERSION, actual, &[], &[]).unwrap_err();
            assert_eq!(
                error,
                CompatibilityError::AbiVersionMismatch {
                    expected: ABI_VERSION,
                    actual
                }
            );
            assert_eq!(
                error.to_string(),
                format!("ABI version mismatch: expected {ABI_VERSION}, found {actual}")
            );
        }
    }

    #[test]
    fn rejects_unknown_required_capabilities_but_allows_optional_ones() {
        let required = CapabilityRequirement::required("future.required", 1);
        assert!(matches!(
            validate_compatibility(ABI_VERSION, ABI_VERSION, &[required], &[]),
            Err(CompatibilityError::MissingRequiredCapability { .. })
        ));

        let optional = CapabilityRequirement::optional("future.optional", 1);
        assert_eq!(
            validate_compatibility(ABI_VERSION, ABI_VERSION, &[optional], &[]),
            Ok(())
        );

        let optional_newer = CapabilityRequirement::optional("future.optional", 2);
        let older = Capability::new("future.optional", 1);
        assert_eq!(
            validate_compatibility(ABI_VERSION, ABI_VERSION, &[optional_newer], &[older]),
            Ok(())
        );
    }

    #[test]
    fn validates_capability_versions_and_duplicate_ids() {
        let requirements = [CapabilityRequirement::required(CAPABILITY_HOOKS, 2)];
        let available = [Capability::new(CAPABILITY_HOOKS, 1)];
        assert!(matches!(
            validate_compatibility(ABI_VERSION, ABI_VERSION, &requirements, &available),
            Err(CompatibilityError::CapabilityVersionTooOld { .. })
        ));

        let duplicate = [
            Capability::new(CAPABILITY_HOOKS, 1),
            Capability::new(CAPABILITY_HOOKS, 1),
        ];
        assert!(matches!(
            validate_compatibility(ABI_VERSION, ABI_VERSION, &[], &duplicate),
            Err(CompatibilityError::DuplicateCapability { .. })
        ));
    }
}

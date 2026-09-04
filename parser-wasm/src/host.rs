//! Native Wasmtime host for CoreLibrary and parser addon Components.
//!
//! The host negotiates capabilities, orders hooks, enforces resource limits,
//! coordinates macros and dynamic syntax, and commits only accepted side effects.
#![allow(missing_docs)] // WIT transport fields are documented as aggregate contracts.

mod public_data;

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    mem,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use fancy_regex::Regex;
use sha2::{Digest, Sha256};
use skript_parser::{
    CandidateFailure, CandidateMatches, ConditionMatches, ConditionNode, ConditionNodeKind,
    ConditionParseError, ConditionParseRequest, ConditionParserConfig, ConditionSemanticDecision,
    ConditionSemanticRequest, EffectCandidate, EffectCandidateFailure, EffectMatches,
    EffectParseError, EffectParseRequest, EffectParserConfig, EffectSemanticDecision,
    EffectSemanticRequest, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionLeafParse,
    ExpressionLeafRequest, ExpressionMatches, ExpressionNode, ExpressionNodeKind,
    ExpressionParseContext, ExpressionParseEnvironment, ExpressionParseError,
    ExpressionParseRequest, ExpressionParserConfig, ExpressionRootMode, FailureTrace,
    FunctionDeclaration, FunctionLookupRequest, FunctionParameterDeclaration,
    FunctionRegistrySavepoint, FunctionRegistrySnapshot, FunctionRegistryTransaction,
    FunctionReturnContract, FunctionScope, FunctionVersionPolicy, MatchInput, MatchPattern,
    MatchSpan, MatchSyntaxKind, NoopPatternMatchHooks, ParsedCapture as ParserParsedCapture,
    ParsedCaptureStatus as ParserParsedCaptureStatus, PatternCandidate, PatternCapture,
    PatternFailure, PatternFailureReason, PatternHookControl, PatternHookEvent, PatternHookOutcome,
    PatternHookScope, PatternHookTiming, PatternMatchEnvironment, PatternMatchError,
    PatternMatchHooks, PatternMatcherConfig, PatternPathSegment, RankedFailures,
    RegisteredCaptureBinding, RegisteredExpressionDecision, RegisteredExpressionRequest,
    RegisteredSyntaxIdentity, RejectTypeExpressions, SectionBodyMode as ParserSectionBodyMode,
    SectionChildrenDecision, SectionChildrenRequest, SectionExitDecision, SectionMatches,
    SectionParseError, SectionParseRequest, SectionParserConfig,
    SectionRawNodeSummary as ParserSectionRawNodeSummary,
    SectionSiblingSummary as ParserSectionSiblingSummary,
    SemanticDiagnostic as ParserSemanticDiagnostic,
    SemanticDiagnosticSeverity as ParserSemanticDiagnosticSeverity,
    SemanticRelatedSpan as ParserSemanticRelatedSpan, StructureBody, StructureBodyMode,
    StructureDocument, StructureDocumentNode, StructureEntry, StructureEntryValue,
    StructureExitDecision, StructureHookDecision, StructureHookRequest, StructureHookTiming,
    StructureParseError, StructureParseRequest, StructureParserConfig, TypeExpressionOutcome,
    TypeExpressionRequest, TypeExpressionResolver, UnknownEffectNode,
    match_pattern_candidates as run_pattern_matcher,
    parse_condition_with_snapshot as run_condition_parser,
    parse_effect_with_snapshot as run_effect_parser,
    parse_expression_with_snapshot as run_expression_parser,
    parse_section_with_snapshot as run_section_parser,
    parse_structures_with_snapshot as run_structure_parser,
};
use skript_parser::{
    ExpansionId, GeneratedRawNode as ParserGeneratedRawNode,
    GeneratedRawNodeId as ParserGeneratedRawNodeId,
    GeneratedRawNodeKind as ParserGeneratedRawNodeKind, GeneratedRawTree as ParserGeneratedRawTree,
    IndentKind as ParserIndentKind, LineEnding as ParserLineEnding, MappedSource,
    OriginKind as ParserOriginKind, RawDiagnosticCode as ParserRawDiagnosticCode,
    RawDiagnosticSeverity as ParserRawDiagnosticSeverity,
    RawInvalidReason as ParserRawInvalidReason, RawNodeId as ParserRawNodeId,
    RawNodeKind as ParserRawNodeKind, RawTree as ParserRawTree,
    RawTriviaKind as ParserRawTriviaKind, RetainedChildren as ParserRetainedChildren,
    RetainedChildrenPlacement as ParserRetainedChildrenPlacement, TextEdit as ParserTextEdit,
    TextExpansion, TextRange as ParserTextRange, TreeEdit as ParserTreeEdit, TreeEditMetadata,
    apply_tree_edit,
};
use syntaxes::{
    Catalog, CatalogSourceRecord, ChangeMode as CatalogChangeMode, ClassName, DefinitionId,
    DynamicMultiplicity, DynamicRegistryError, DynamicStructureBodyMode, DynamicSyntaxId,
    DynamicSyntaxInput, DynamicSyntaxOverrideInput, DynamicSyntaxRegistry, DynamicSyntaxSnapshot,
    DynamicSyntaxUpdate, EntryData, EntryKind, EntryValidator, Multiplicity, NodeType,
    PossibleReturnTypesState, RegistrationId, ResolutionState, ReturnTypeState, Syntax,
    SyntaxKind as CatalogSyntaxKind, SyntaxOverrideTarget, SyntaxReference,
};
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Engine, ResourceLimiter, Store, Trap};

use crate::bindings::ParserAddon;
use crate::bindings::nlaocs::skript_parser_addon::catalog_data as wit_catalog_data;
use crate::bindings::nlaocs::skript_parser_addon::dynamic_syntax_registry as wit_dynamic_registry;
use crate::bindings::nlaocs::skript_parser_addon::state_store as wit_state_store;
use crate::bindings::nlaocs::skript_parser_addon::types::{
    AcceptedChangeMode as WitAcceptedChangeMode, AddonAttachment as WitAddonAttachment,
    CatalogRecordRef as WitCatalogRecordRef, ConditionCandidate as WitConditionCandidate,
    ConditionCapture as WitConditionCapture,
    ConditionExpressionCapture as WitConditionExpressionCapture, ConditionMark as WitConditionMark,
    ConditionPayload as WitConditionPayload, ConditionRegexCapture as WitConditionRegexCapture,
    ConditionTag as WitConditionTag, DynamicMultiplicity as WitDynamicMultiplicity,
    DynamicRegistryError as WitDynamicRegistryError,
    DynamicRegistryErrorKind as WitDynamicRegistryErrorKind,
    DynamicSyntaxDefinition as WitDynamicSyntaxDefinition,
    DynamicSyntaxOverride as WitDynamicSyntaxOverride,
    DynamicSyntaxOverrideTarget as WitDynamicSyntaxOverrideTarget,
    DynamicSyntaxReference as WitDynamicSyntaxReference, EffectCandidate as WitEffectCandidate,
    EffectCapture as WitEffectCapture, EffectExpressionCapture as WitEffectExpressionCapture,
    EffectFailure as WitEffectFailure, EffectMark as WitEffectMark,
    EffectNearMatch as WitEffectNearMatch, EffectPayload as WitEffectPayload,
    EffectRegexCapture as WitEffectRegexCapture, EffectTag as WitEffectTag,
    EffectTiming as WitEffectTiming, ExpressionExpectedType as WitExpressionExpectedType,
    ExpressionLeafCandidate as WitExpressionLeafCandidate,
    ExpressionLeafKind as WitExpressionLeafKind,
    ExpressionLiteralOption as WitExpressionLiteralOption,
    ExpressionLiteralSource as WitExpressionLiteralSource,
    ExpressionPayload as WitExpressionPayload,
    ExpressionPossibleReturnTypesState as WitPossibleReturnTypesState,
    ExpressionPublicData as WitExpressionPublicData,
    ExpressionReturnTypeState as WitReturnTypeState,
    ExpressionTypeOption as WitExpressionTypeOption, FunctionDeclaration as WitFunctionDeclaration,
    FunctionDeclarationScope as WitFunctionDeclarationScope,
    GeneratedRawNodeKind as WitGeneratedRawNodeKind, IndentKind as WitIndentKind,
    Indentation as WitIndentation, LineEnding as WitLineEnding, MetadataEntry as WitMetadataEntry,
    MetadataResolutionState as WitMetadataResolutionState, OriginKind as WitOriginKind,
    ParseContext as WitParseContext, ParseContextValue as WitParseContextValue,
    ParseResultStatus as WitParseResultStatus, ParseSummary as WitParseSummary,
    ParsedCapture as WitParsedCapture, ParserDeclaration as WitParserDeclaration,
    RawDiagnostic as WitRawDiagnostic, RawDiagnosticCode as WitRawDiagnosticCode,
    RawDiagnosticSeverity as WitRawDiagnosticSeverity, RawInvalidReason as WitRawInvalidReason,
    RawLine as WitRawLine, RawNodeKind as WitRawNodeKind, RawRelatedSpan as WitRawRelatedSpan,
    RawTreeNode as WitRawTreeNode, RawTrivia as WitRawTrivia, RawTriviaKind as WitRawTriviaKind,
    RegisteredExpressionChild as WitRegisteredExpressionChild,
    RegisteredExpressionPayload as WitRegisteredExpressionPayload,
    RegisteredExpressionPropertyOption as WitRegisteredExpressionPropertyOption,
    RegisteredExpressionTag as WitRegisteredExpressionTag,
    RegisteredHandlerBinding as WitRegisteredHandlerBinding,
    RetainedChildrenPlacement as WitRetainedChildrenPlacement,
    SectionBodyMode as WitSectionBodyMode, SectionCandidate as WitSectionCandidate,
    SectionPayload as WitSectionPayload, SectionRawNode as WitSectionRawNode,
    SectionRawNodeKind as WitSectionRawNodeKind, SectionSibling as WitSectionSibling,
    SectionTiming as WitSectionTiming, SourceOrigin as WitSourceOrigin,
    StateEncoding as WitStateEncoding, StateEntry as WitStateEntry, StateError as WitStateError,
    StateErrorKind as WitStateErrorKind, StateNamespaceVisibility as WitNamespaceVisibility,
    StateScope as WitStateScope, StateValue as WitStateValue,
    StructureBodyMode as WitStructureBodyMode, StructureCandidate as WitStructureCandidate,
    StructureEntry as WitStructureEntry, StructureEntryData as WitStructureEntryData,
    StructureEntryKind as WitStructureEntryKind,
    StructureEntryValidator as WitStructureEntryValidator,
    StructureEntryValueKind as WitStructureEntryValueKind,
    StructureNodeType as WitStructureNodeType, StructurePayload as WitStructurePayload,
    StructureTiming as WitStructureTiming, SyntaxMark as WitSyntaxMark, SyntaxTag as WitSyntaxTag,
    TextEdit as WitTextEdit, TextRange as WitTextRange, TreeEdit as WitTreeEdit,
};
use crate::state::{
    InvocationTransaction, NamespaceDeclaration, NamespaceVisibility, ParseTransaction,
    StateEncoding, StateError, StateReadWriteSet, StateSavepoint, StateScope, StateStore,
    StateStoreConfig, StateValue,
};
use crate::{
    ABI_VERSION, AbiVersion, CAPABILITY_ADDITIONAL_PARSE, CAPABILITY_CATALOG_DATA,
    CAPABILITY_CONDITION_PARSER, CAPABILITY_CONTEXT_UPDATES, CAPABILITY_DYNAMIC_SYNTAX,
    CAPABILITY_EFFECT_PARSER, CAPABILITY_EXPRESSION_PARSER, CAPABILITY_HOOKS,
    CAPABILITY_SECTION_PARSER, CAPABILITY_STATE_STORE, CAPABILITY_STRUCTURE_PARSER,
    CAPABILITY_TEXT_MACRO, CAPABILITY_TREE_MACRO, Capability, CapabilityRequirement,
    CompatibilityError, REGISTERED_CONTEXT_ALL_TYPE_OPTIONS, validate_compatibility,
};

pub use crate::bindings::nlaocs::skript_parser_addon::types::{
    AstNode, AstTree, Capture, CaptureValue, CatalogAnnotationTarget, ComponentManifest,
    ConditionPayload, ContextUpdate, Diagnostic, DiagnosticSeverity, ExpressionExpectedType,
    ExpressionLiteralOption, ExpressionPossibleReturnTypesState, ExpressionReturnTypeState,
    ExpressionTypeOption, HookDecision, HookEffects, HookMode, HookOutput, HookPayload, HookPhase,
    HookSelector, HookSubscription, HookTarget, InvocationContext, MappedSpan, MatchingPathSegment,
    MatchingPayload, MatchingScope, MatchingStatus, MatchingTiming, ParseRequest, ParseResult,
    ParseResultNode, PatternRef, RawTree, RawTreeNode, RegisteredExpressionChild,
    RegisteredExpressionPayload, RegisteredExpressionPropertyOption, RegisteredExpressionTag,
    RegisteredSyntaxHandlerTarget, Rejection, RelatedSpan, ReturnTypeSelector,
    SelectorTypeRelation, SyntaxKind, TextMacroInput, TextMacroOutput, TreeMacroInput,
    TreeMacroOutput, TypeParserUnresolved,
};

/// Reserved component ID required for the first host component.
pub const CORE_LIBRARY_COMPONENT_ID: &str = "nlaocs.core-library";
const CORE_LIBRARY_DEADLINE_TICKS: u64 = u64::MAX / 2;

#[derive(Debug, Clone)]
/// Execution, memory, pipeline, StateStore, and catalog configuration.
///
/// Defaults are intentionally bounded for untrusted addon components. Callers
/// may tune individual budgets and attach an SSG [Catalog] to enable dynamic
/// syntax registration, while zero-valued quotas and durations are rejected.
/// The bundled CoreLibrary remains fuel- and resource-bounded but is exempt
/// from the addon epoch timeout because its hooks perform trusted catalog work.
/// When both carry source identity, `syntax_catalog` and `runtime_profile` must
/// identify the same snapshot and schema.
///
/// # Examples
///
/// ~~~
/// use std::{sync::Arc, time::Duration};
/// use parser_wasm::HostConfig;
/// use syntaxes::Catalog;
///
/// fn project_config(catalog: Arc<Catalog>) -> HostConfig {
///     HostConfig {
///         call_timeout: Duration::from_millis(50),
///         max_memory_bytes: 32 * 1024 * 1024,
///         syntax_catalog: Some(catalog),
///         ..HostConfig::default()
///     }
/// }
/// # let _ = project_config;
/// ~~~
pub struct HostConfig {
    pub fuel_per_call: u64,
    pub call_timeout: Duration,
    pub epoch_tick: Duration,
    pub max_memory_bytes: usize,
    pub max_table_elements: usize,
    pub max_instances_per_component: usize,
    pub max_tables_per_component: usize,
    pub max_memories_per_component: usize,
    pub max_calls_per_dispatch: usize,
    pub max_generated_output_bytes: usize,
    pub max_parser_rounds: usize,
    pub max_parse_requests_per_hook: usize,
    pub max_parse_result_nodes: usize,
    pub max_text_macro_expansions: usize,
    pub max_text_macro_generated_bytes: usize,
    pub max_virtual_source_bytes: usize,
    pub max_raw_tree_depth: usize,
    pub max_tree_macro_expansion_depth: usize,
    pub max_tree_macro_nodes: usize,
    pub max_tree_macro_calls: usize,
    pub max_catalog_response_bytes: usize,
    pub state_store: StateStoreConfig,
    pub syntax_catalog: Option<Arc<Catalog>>,
    pub runtime_profile: RuntimeProfile,
}

/// Versioned server environment associated with the loaded SSG snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeProfile {
    pub snapshot_schema_version: Option<u32>,
    pub snapshot_id: Option<String>,
    pub server_name: Option<String>,
    pub server_version: Option<String>,
    pub minecraft_version: Option<String>,
    pub java_version: Option<String>,
    pub language: Option<String>,
    pub skript_version: Option<String>,
    pub plugins: Vec<RuntimePlugin>,
}

/// One enabled plugin in deterministic server load order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePlugin {
    pub load_order: usize,
    pub name: String,
    pub version: String,
    pub main: String,
}

fn function_policy_for_runtime(version: Option<&str>) -> FunctionVersionPolicy {
    let Some(version) = version else {
        return FunctionVersionPolicy::default();
    };
    let mut numbers = version
        .split(|character: char| !character.is_ascii_digit())
        .filter(|component| !component.is_empty())
        .filter_map(|component| component.parse::<u32>().ok());
    let (Some(major), Some(minor)) = (numbers.next(), numbers.next()) else {
        return FunctionVersionPolicy::default();
    };
    FunctionVersionPolicy::for_skript_version(major, minor, numbers.next().unwrap_or(0))
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            fuel_per_call: 20_000_000,
            call_timeout: Duration::from_millis(100),
            epoch_tick: Duration::from_millis(10),
            max_memory_bytes: 64 * 1024 * 1024,
            max_table_elements: 100_000,
            max_instances_per_component: 32,
            max_tables_per_component: 32,
            max_memories_per_component: 32,
            max_calls_per_dispatch: 1_024,
            max_generated_output_bytes: 8 * 1024 * 1024,
            max_parser_rounds: 16,
            max_parse_requests_per_hook: 256,
            max_parse_result_nodes: 4_096,
            max_text_macro_expansions: 256,
            max_text_macro_generated_bytes: 8 * 1024 * 1024,
            max_virtual_source_bytes: 16 * 1024 * 1024,
            max_raw_tree_depth: 256,
            max_tree_macro_expansion_depth: 64,
            max_tree_macro_nodes: 100_000,
            max_tree_macro_calls: 4_096,
            max_catalog_response_bytes: 32 * 1024 * 1024,
            state_store: StateStoreConfig::default(),
            syntax_catalog: None,
            runtime_profile: RuntimeProfile::default(),
        }
    }
}

impl HostConfig {
    fn inherit_catalog_runtime(&mut self) {
        let Some(source) = self.syntax_catalog.as_deref().and_then(Catalog::source) else {
            return;
        };
        self.runtime_profile
            .snapshot_id
            .get_or_insert_with(|| source.snapshot_id.clone());
        self.runtime_profile
            .snapshot_schema_version
            .get_or_insert(source.schema_version);
        let Some(runtime) = source.runtime.as_ref() else {
            return;
        };
        self.runtime_profile
            .server_name
            .get_or_insert_with(|| runtime.server_name.clone());
        self.runtime_profile
            .server_version
            .get_or_insert_with(|| runtime.server_version.clone());
        self.runtime_profile
            .minecraft_version
            .get_or_insert_with(|| runtime.minecraft_version.clone());
        self.runtime_profile
            .java_version
            .get_or_insert_with(|| runtime.java_version.clone());
        self.runtime_profile
            .language
            .get_or_insert_with(|| runtime.language.clone());
        if self.runtime_profile.skript_version.is_none() {
            self.runtime_profile.skript_version = runtime
                .plugins
                .iter()
                .find(|plugin| plugin.enabled && plugin.name.eq_ignore_ascii_case("Skript"))
                .map(|plugin| plugin.version.clone());
        }
        if self.runtime_profile.plugins.is_empty() {
            self.runtime_profile.plugins = runtime
                .plugins
                .iter()
                .filter(|plugin| plugin.enabled)
                .map(|plugin| RuntimePlugin {
                    load_order: plugin.load_order,
                    name: plugin.name.clone(),
                    version: plugin.version.clone(),
                    main: plugin.main.clone(),
                })
                .collect();
        }
    }

    fn validate(&self) -> Result<(), HostError> {
        let invalid = self.fuel_per_call == 0
            || self.call_timeout.is_zero()
            || self.epoch_tick.is_zero()
            || self.max_memory_bytes == 0
            || self.max_table_elements == 0
            || self.max_instances_per_component == 0
            || self.max_tables_per_component == 0
            || self.max_memories_per_component == 0
            || self.max_calls_per_dispatch == 0
            || self.max_generated_output_bytes == 0
            || self.max_parser_rounds == 0
            || self.max_parse_requests_per_hook == 0
            || self.max_parse_result_nodes == 0
            || self.max_text_macro_expansions == 0
            || self.max_text_macro_generated_bytes == 0
            || self.max_virtual_source_bytes == 0
            || self.max_raw_tree_depth == 0
            || self.max_tree_macro_expansion_depth == 0
            || self.max_tree_macro_nodes == 0
            || self.max_tree_macro_calls == 0
            || self.max_catalog_response_bytes == 0;
        if invalid {
            return Err(HostError::InvalidConfiguration);
        }
        if let Some(source) = self.syntax_catalog.as_deref().and_then(Catalog::source) {
            if let Some(profile_snapshot_id) = self.runtime_profile.snapshot_id.as_deref()
                && profile_snapshot_id != source.snapshot_id
            {
                return Err(HostError::CatalogProfileMismatch {
                    field: "snapshot ID",
                    profile: profile_snapshot_id.to_owned(),
                    catalog: source.snapshot_id.clone(),
                });
            }
            if let Some(profile_schema_version) = self.runtime_profile.snapshot_schema_version
                && profile_schema_version != source.schema_version
            {
                return Err(HostError::CatalogProfileMismatch {
                    field: "schema version",
                    profile: profile_schema_version.to_string(),
                    catalog: source.schema_version.to_string(),
                });
            }
        }
        Ok(())
    }

    fn deadline_ticks(&self, component_id: &str) -> u64 {
        if component_id == CORE_LIBRARY_COMPONENT_ID {
            return CORE_LIBRARY_DEADLINE_TICKS;
        }
        let timeout = self.call_timeout.as_nanos();
        let tick = self.epoch_tick.as_nanos();
        timeout.div_ceil(tick).clamp(1, u64::MAX as u128) as u64
    }
}

#[derive(Debug, Clone, thiserror::Error)]
/// Host setup, component execution, output validation, or quota failure.
pub enum HostError {
    #[error("CoreLibrary component is missing")]
    CoreLibraryMissing,
    #[error("invalid parser host configuration: every quota and duration must be non-zero")]
    InvalidConfiguration,
    #[error(
        "runtime profile {field} {profile:?} does not match source Catalog {field} {catalog:?}"
    )]
    CatalogProfileMismatch {
        field: &'static str,
        profile: String,
        catalog: String,
    },
    #[error("failed to create the Wasmtime engine: {message}")]
    Engine { message: String },
    #[error("failed to compile component {component_id}: {message}")]
    ComponentCompile {
        component_id: String,
        message: String,
    },
    #[error("failed to instantiate component {component_id}: {message}")]
    ComponentInstantiation {
        component_id: String,
        message: String,
    },
    #[error("component manifest is invalid: {message}")]
    InvalidManifest { message: String },
    #[error("StateStore operation failed: {0}")]
    StateStore(#[from] StateError),
    #[error("pattern matching failed: {0}")]
    PatternMatcher(#[from] PatternMatchError),
    #[error("Expression parsing failed: {0}")]
    ExpressionParser(#[from] ExpressionParseError),
    #[error("Condition parsing failed: {0}")]
    ConditionParser(#[from] ConditionParseError),
    #[error("Effect parsing failed: {0}")]
    EffectParser(#[from] EffectParseError),
    #[error("Section parsing failed: {0}")]
    SectionParser(#[from] SectionParseError),
    #[error("Structure parsing failed: {0}")]
    StructureParser(#[from] StructureParseError),
    #[error("Function registry failed: {0}")]
    FunctionRegistry(#[from] skript_parser::FunctionRegistryError),
    #[error("syntax parsing requires an SSG syntax Catalog")]
    SyntaxCatalogUnavailable,
    #[error("dynamic syntax registry is unavailable without an SSG Catalog")]
    DynamicSyntaxUnavailable,
    #[error("dynamic syntax registry operation failed: {0}")]
    DynamicSyntax(#[from] DynamicRegistryError),
    #[error("component {component_id} is already loaded")]
    DuplicateComponent { component_id: String },
    #[error("the mandatory component must be {expected}, found {actual}")]
    InvalidCoreLibrary { expected: String, actual: String },
    #[error("component {component_id} is incompatible: {source}")]
    Compatibility {
        component_id: String,
        #[source]
        source: CompatibilityError,
    },
    #[error("component {component_id} rejected initialization: {message}")]
    InitializationRejected {
        component_id: String,
        message: String,
    },
    #[error("component {component_id} returned an addon error: {message}")]
    AddonFailure {
        component_id: String,
        message: String,
    },
    #[error("component {component_id} failed during {operation}: {message}")]
    Runtime {
        component_id: String,
        operation: &'static str,
        message: String,
    },
    #[error("component {component_id} trapped during {operation}: {message}")]
    Trap {
        component_id: String,
        operation: &'static str,
        message: String,
    },
    #[error("component {component_id} timed out during {operation}")]
    Timeout {
        component_id: String,
        operation: &'static str,
    },
    #[error("component {component_id} exhausted its fuel during {operation}")]
    FuelExhausted {
        component_id: String,
        operation: &'static str,
    },
    #[error("component {component_id} exceeded a resource limit during {operation}: {message}")]
    ResourceLimit {
        component_id: String,
        operation: &'static str,
        message: String,
    },
    #[error("Effect hook returned invalid output: {message}")]
    InvalidEffectHookOutput { message: String },
    #[error("Condition hook returned invalid output: {message}")]
    InvalidConditionHookOutput { message: String },
    #[error("component {component_id} returned invalid output for {subscription_id}: {message}")]
    InvalidHookOutput {
        component_id: String,
        subscription_id: String,
        message: String,
    },
    #[error("dispatch exceeded the call quota of {limit}")]
    CallQuotaExceeded { limit: usize },
    #[error("dispatch exceeded the generated output quota of {limit} bytes")]
    GeneratedOutputQuotaExceeded { limit: usize },
    #[error("hook exceeded the parser continuation quota of {limit} rounds")]
    ParserRoundQuotaExceeded { limit: usize },
    #[error("hook exceeded the parse request quota of {limit}")]
    ParseRequestQuotaExceeded { limit: usize },
    #[error("parse result exceeded the node quota of {limit}")]
    ParseResultNodeQuotaExceeded { limit: usize },
    #[error("host parse-result token space is exhausted")]
    ParseResultTokenExhausted,
    #[error("parser {parser_id} returned an invalid result: {message}")]
    InvalidParseResult { parser_id: String, message: String },
    #[error("text macro pipeline exceeded the expansion quota of {limit}")]
    TextMacroExpansionQuotaExceeded { limit: usize },
    #[error("text macro pipeline exceeded the generated text quota of {limit} bytes")]
    TextMacroGeneratedBytesQuotaExceeded { limit: usize },
    #[error("text macro pipeline exceeded the virtual source quota of {limit} bytes")]
    VirtualSourceQuotaExceeded { limit: usize },
    #[error(
        "component {component_id} returned invalid text macro output for {subscription_id}: {message}"
    )]
    InvalidTextMacroOutput {
        component_id: String,
        subscription_id: String,
        message: String,
    },
    #[error(
        "component {component_id} returned invalid tree macro output for {subscription_id}: {message}"
    )]
    InvalidTreeMacroOutput {
        component_id: String,
        subscription_id: String,
        message: String,
    },
    #[error("raw tree exceeded the structural depth quota of {limit}")]
    RawTreeDepthQuotaExceeded { limit: usize },
    #[error("tree macro pipeline exceeded the expansion depth quota of {limit}")]
    TreeMacroExpansionDepthQuotaExceeded { limit: usize },
    #[error("tree macro pipeline exceeded the node quota of {limit}")]
    TreeMacroNodeQuotaExceeded { limit: usize },
    #[error("tree macro pipeline exceeded the hook call quota of {limit}")]
    TreeMacroCallQuotaExceeded { limit: usize },
    #[error("tree macro expansion cycle detected in {component_id}:{subscription_id}")]
    TreeMacroCycleDetected {
        component_id: String,
        subscription_id: String,
    },
    #[error("the mandatory CoreLibrary component cannot be unloaded")]
    CannotUnloadCoreLibrary,
}

impl HostError {
    fn disables_component(&self) -> bool {
        matches!(
            self,
            Self::Trap { .. }
                | Self::Timeout { .. }
                | Self::FuelExhausted { .. }
                | Self::ResourceLimit { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Observable identity and runtime status of a loaded component.
pub struct ComponentInfo {
    pub component_id: String,
    pub component_version: String,
    pub load_order: usize,
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Registry scope selected for a generic hook dispatch.
pub enum DispatchTarget {
    ParseStage,
    SyntaxKind(SyntaxKind),
    Definition {
        definition_id: String,
        syntax_kind: SyntaxKind,
    },
    Parser(String),
    Registration {
        definition_id: String,
        registration_id: String,
        syntax_kind: SyntaxKind,
    },
    Pattern {
        definition_id: String,
        registration_id: String,
        pattern_index: u64,
        syntax_kind: SyntaxKind,
    },
}

/// Context, target, phase, and payload for one generic dispatch.
pub struct DispatchRequest {
    pub context: InvocationContext,
    pub target: DispatchTarget,
    pub phase: HookPhase,
    pub payload: HookPayload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Trace record for one attempted component subscription.
pub struct HookCall {
    pub component_id: String,
    pub subscription_id: String,
}

#[derive(Debug, Clone)]
/// Recoverable per-component failure retained in a pipeline result.
pub struct ComponentFailure {
    pub component_id: String,
    pub subscription_id: String,
    pub error: HostError,
}

#[derive(Debug)]
/// Accepted decision, payload, effects, call trace, and recoverable failures.
pub struct DispatchResult {
    pub decision: HookDecision,
    pub payload: HookPayload,
    pub effects: HookEffects,
    pub calls: Vec<HookCall>,
    pub failures: Vec<ComponentFailure>,
    available_parse_results: BTreeMap<u64, ExecutedParseResult>,
}

#[derive(Debug, Clone)]
struct ExecutedParseResult {
    wire: ParseResult,
    expression_roots: BTreeMap<u64, ExpressionNode>,
}

#[derive(Debug)]
/// Native candidate results plus accepted matching-hook side effects.
pub struct WasmPatternMatchResult {
    pub matches: CandidateMatches,
    pub effects: HookEffects,
    pub calls: Vec<HookCall>,
    pub failures: Vec<ComponentFailure>,
}

#[derive(Debug)]
/// Recursive Expression results plus accepted WASM side effects and trace data.
pub struct WasmExpressionParseResult {
    pub matches: ExpressionMatches,
    pub effects: HookEffects,
    pub calls: Vec<HookCall>,
    pub failures: Vec<ComponentFailure>,
}
#[derive(Debug)]
/// Condition results plus accepted matching/Expression hook side effects.
pub struct WasmConditionParseResult {
    pub matches: ConditionMatches,
    pub effects: HookEffects,
    pub calls: Vec<HookCall>,
    pub failures: Vec<ComponentFailure>,
}
/// Effect results plus accepted matching/Expression/Effect hook side effects.
pub struct WasmEffectParseResult {
    pub matches: EffectMatches,
    pub effects: HookEffects,
    pub calls: Vec<HookCall>,
    pub failures: Vec<ComponentFailure>,
}
#[derive(Debug)]
/// Recursive Section results plus scoped lifecycle hook side effects.
pub struct WasmSectionParseResult {
    pub matches: SectionMatches,
    pub effects: HookEffects,
    pub calls: Vec<HookCall>,
    pub failures: Vec<ComponentFailure>,
}
#[derive(Debug)]
/// Top-level Structure results plus EntryValidator and lifecycle hook effects.
pub struct WasmStructureParseResult {
    pub document: StructureDocument,
    pub functions: FunctionRegistrySnapshot,
    pub effects: HookEffects,
    pub calls: Vec<HookCall>,
    pub failures: Vec<ComponentFailure>,
}
/// Invocation context and mapped input for a Text macro pipeline.
pub struct TextMacroRequest {
    pub context: InvocationContext,
    pub source: MappedSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Per-subscription Text macro acceptance, provenance, and state dependencies.
pub struct TextMacroCall {
    pub component_id: String,
    pub subscription_id: String,
    pub accepted: bool,
    pub expansion: Option<ExpansionId>,
    pub state_accesses: StateReadWriteSet,
}

#[derive(Debug)]
/// Final mapped source and transactional metadata from Text preprocessing.
pub struct TextMacroResult {
    pub decision: HookDecision,
    pub source: MappedSource,
    pub effects: HookEffects,
    pub calls: Vec<TextMacroCall>,
    pub failures: Vec<ComponentFailure>,
}

/// Mapped source and lossless input tree for recursive Tree expansion.
pub struct TreeMacroRequest {
    pub context: InvocationContext,
    pub source: MappedSource,
    pub tree: ParserRawTree,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Per-node Tree macro acceptance, expansion identity, and state dependencies.
pub struct TreeMacroCall {
    pub component_id: String,
    pub subscription_id: String,
    pub target: ParserRawNodeId,
    pub accepted: bool,
    pub expansion: Option<ExpansionId>,
    pub state_accesses: StateReadWriteSet,
}

#[derive(Debug)]
/// Final tree/source provenance and transactional Tree macro metadata.
pub struct TreeMacroResult {
    pub decision: HookDecision,
    pub source: MappedSource,
    pub tree: ParserRawTree,
    pub effects: HookEffects,
    pub calls: Vec<TreeMacroCall>,
    pub failures: Vec<ComponentFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct TreeMacroCycleKey {
    component_id: String,
    subscription_id: String,
    original_ranges: Vec<(usize, usize)>,
    fingerprint: Vec<u8>,
}

struct TreeMacroPipeline {
    effects: HookEffects,
    calls: Vec<TreeMacroCall>,
    failures: Vec<ComponentFailure>,
    output_bytes: usize,
    active: Vec<TreeMacroCycleKey>,
    handled: bool,
}

impl TreeMacroPipeline {
    fn new() -> Self {
        Self {
            effects: empty_effects(),
            calls: Vec::new(),
            failures: Vec::new(),
            output_bytes: 0,
            active: Vec::new(),
            handled: false,
        }
    }
}

enum TreeWalk {
    Continue { sibling_count: usize },
    Reject(HookDecision),
}
#[derive(Debug, thiserror::Error)]
#[error("{resource} request of {requested} exceeds the host limit of {limit}")]
struct GuestResourceLimit {
    resource: &'static str,
    requested: usize,
    limit: usize,
}

struct HostResourceLimiter {
    memory_bytes: usize,
    table_elements: usize,
    instances: usize,
    tables: usize,
    memories: usize,
}

impl ResourceLimiter for HostResourceLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.memory_bytes {
            return Err(GuestResourceLimit {
                resource: "linear memory",
                requested: desired,
                limit: self.memory_bytes,
            }
            .into());
        }
        Ok(maximum.is_none_or(|maximum| desired <= maximum))
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.table_elements {
            return Err(GuestResourceLimit {
                resource: "table",
                requested: desired,
                limit: self.table_elements,
            }
            .into());
        }
        Ok(maximum.is_none_or(|maximum| desired <= maximum))
    }

    fn instances(&self) -> usize {
        self.instances
    }

    fn tables(&self) -> usize {
        self.tables
    }

    fn memories(&self) -> usize {
        self.memories
    }
}

struct StoreData {
    limits: HostResourceLimiter,
    invocation: Option<InvocationTransaction>,
    dynamic_syntax_update: Option<DynamicSyntaxUpdate>,
    dynamic_syntax_available: bool,
    catalog: Option<Arc<Catalog>>,
    registered_handler_bindings: Vec<WitRegisteredHandlerBinding>,
    language_patterns: HashMap<String, Option<Regex>>,
    type_user_input_matchers: Arc<[TypeUserInputMatcher]>,
    max_catalog_response_bytes: usize,
}

struct TypeUserInputMatcher {
    option: WitExpressionTypeOption,
    patterns: Vec<Regex>,
}

struct CachedTypeUserInputMatchers {
    options: Arc<[WitExpressionTypeOption]>,
    matchers: Arc<[TypeUserInputMatcher]>,
}

static TYPE_USER_INPUT_MATCHER_CACHE: LazyLock<
    Mutex<HashMap<String, Vec<CachedTypeUserInputMatchers>>>,
> = LazyLock::new(|| Mutex::new(HashMap::new()));

impl crate::bindings::nlaocs::skript_parser_addon::types::Host for StoreData {}

impl wit_catalog_data::Host for StoreData {
    fn source(
        &mut self,
    ) -> Result<Option<wit_catalog_data::CatalogSource>, wit_catalog_data::CatalogError> {
        let catalog = self.catalog()?;
        Ok(catalog
            .source()
            .map(|source| wit_catalog_data::CatalogSource {
                format: source.format.clone(),
                schema_version: source.schema_version,
                snapshot_id: source.snapshot_id.clone(),
                source_digest: source.source_digest.clone(),
            }))
    }

    fn documents(
        &mut self,
        offset: u64,
        limit: u32,
    ) -> Result<wit_catalog_data::CatalogDocumentPage, wit_catalog_data::CatalogError> {
        let catalog = self.catalog()?;
        let documents = catalog.source().map_or_else(Vec::new, |source| {
            source
                .document_names()
                .map(|name| (name, source.document(name).map_or(0, <[u8]>::len)))
                .collect()
        });
        catalog_document_page(&documents, offset, limit, self.max_catalog_response_bytes)
    }

    fn read_document(
        &mut self,
        name: String,
        offset: u64,
        max_bytes: u32,
    ) -> Result<Option<wit_catalog_data::CatalogChunk>, wit_catalog_data::CatalogError> {
        let catalog = self.catalog()?;
        let Some(bytes) = catalog.source().and_then(|source| source.document(&name)) else {
            return Ok(None);
        };
        catalog_chunk(bytes, offset, max_bytes, self.max_catalog_response_bytes).map(Some)
    }

    fn records_by_registration_id(
        &mut self,
        id: String,
        offset: u64,
        limit: u32,
    ) -> Result<wit_catalog_data::CatalogRecordPage, wit_catalog_data::CatalogError> {
        let catalog = self.catalog()?;
        let records = catalog
            .source()
            .map_or(&[][..], |source| source.records_by_registration_id(&id));
        let snapshot_id = catalog
            .source()
            .map(|source| source.snapshot_id.as_str())
            .unwrap_or_default();
        let source_digest = catalog
            .source()
            .map(|source| source.source_digest.as_str())
            .unwrap_or_default();
        catalog_record_page(
            records,
            source_digest,
            snapshot_id,
            offset,
            limit,
            self.max_catalog_response_bytes,
        )
    }

    fn records_by_definition_id(
        &mut self,
        id: String,
        offset: u64,
        limit: u32,
    ) -> Result<wit_catalog_data::CatalogRecordPage, wit_catalog_data::CatalogError> {
        let catalog = self.catalog()?;
        let records = catalog
            .source()
            .map_or(&[][..], |source| source.records_by_definition_id(&id));
        let snapshot_id = catalog
            .source()
            .map(|source| source.snapshot_id.as_str())
            .unwrap_or_default();
        let source_digest = catalog
            .source()
            .map(|source| source.source_digest.as_str())
            .unwrap_or_default();
        catalog_record_page(
            records,
            source_digest,
            snapshot_id,
            offset,
            limit,
            self.max_catalog_response_bytes,
        )
    }

    fn read_record(
        &mut self,
        source_digest: String,
        snapshot_id: String,
        document: String,
        index: u64,
        offset: u64,
        max_bytes: u32,
    ) -> Result<Option<wit_catalog_data::CatalogChunk>, wit_catalog_data::CatalogError> {
        let catalog = self.catalog()?;
        let Some(source) = catalog.source() else {
            return Ok(None);
        };
        if source.source_digest != source_digest {
            return Err(invalid_catalog_input(
                "catalog record reference belongs to a different retained source",
            ));
        }
        if source.snapshot_id != snapshot_id {
            return Err(invalid_catalog_input(
                "catalog record reference belongs to a different snapshot",
            ));
        }
        let Some(index) = usize::try_from(index).ok() else {
            return Ok(None);
        };
        let Some(record) = source.record(&document, index) else {
            return Ok(None);
        };
        catalog_chunk(
            &record.json,
            offset,
            max_bytes,
            self.max_catalog_response_bytes,
        )
        .map(Some)
    }

    fn class_known(&mut self, class_name: String) -> Result<bool, wit_catalog_data::CatalogError> {
        Ok(self.catalog()?.class(&class_name).is_some())
    }

    fn declared_method_exists(
        &mut self,
        class_name: String,
        method_name: String,
        parameter_types: Vec<String>,
        return_type: Option<String>,
    ) -> Result<Option<bool>, wit_catalog_data::CatalogError> {
        let parameter_types = parameter_types
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        Ok(self.catalog()?.declared_method_exists(
            &class_name,
            &method_name,
            &parameter_types,
            return_type.as_deref(),
        ))
    }

    fn container_element_type(
        &mut self,
        class_name: String,
    ) -> Result<Option<String>, wit_catalog_data::CatalogError> {
        Ok(self
            .catalog()?
            .class(&class_name)
            .and_then(|class| class.container_element_type.as_ref())
            .map(|class_name| class_name.as_str().to_owned()))
    }

    fn event_values_for(
        &mut self,
        event_class: String,
    ) -> Result<Vec<wit_catalog_data::EventValueOption>, wit_catalog_data::CatalogError> {
        Ok(self
            .catalog()?
            .event_value_candidates_for(&event_class)
            .into_iter()
            .map(wit_event_value_option)
            .collect())
    }

    fn event_values_for_input(
        &mut self,
        event_class: String,
        input: String,
    ) -> Result<Vec<wit_catalog_data::EventValueOption>, wit_catalog_data::CatalogError> {
        let catalog = self.catalog()?;
        Ok(catalog
            .event_value_candidates_for(&event_class)
            .into_iter()
            .filter(|value| event_value_matches_input(catalog, value, &input))
            .map(wit_event_value_option)
            .collect())
    }

    fn language_value(
        &mut self,
        key: String,
    ) -> Result<Option<String>, wit_catalog_data::CatalogError> {
        Ok(self.catalog()?.language_value(&key).map(str::to_owned))
    }

    fn experiments(
        &mut self,
    ) -> Result<Vec<wit_catalog_data::CatalogExperiment>, wit_catalog_data::CatalogError> {
        let mut experiments = BTreeSet::new();
        for syntax in self.catalog()?.syntaxes() {
            let Some(experimental) = syntax
                .common()
                .and_then(|common| common.experimental_syntax.as_ref())
            else {
                continue;
            };
            for experiment in experimental.required.iter().chain(&experimental.disallowed) {
                experiments.insert((
                    experiment.code_name.clone(),
                    experiment.phase.clone(),
                    experiment.known,
                ));
            }
        }
        Ok(experiments
            .into_iter()
            .map(
                |(code_name, phase, known)| wit_catalog_data::CatalogExperiment {
                    code_name,
                    phase,
                    known,
                },
            )
            .collect())
    }

    fn registered_handler_matches(
        &mut self,
        handler_id: String,
        registration_id: String,
    ) -> Result<bool, wit_catalog_data::CatalogError> {
        Ok(self.registered_handler_bindings.iter().any(|binding| {
            binding.handler_id == handler_id
                && binding
                    .registration_ids
                    .iter()
                    .any(|resolved| resolved == &registration_id)
        }))
    }

    fn language_pattern_matches(
        &mut self,
        key: String,
        fallback_pattern: String,
        input: String,
    ) -> Result<bool, wit_catalog_data::CatalogError> {
        let pattern = self
            .catalog()?
            .language_value(&key)
            .unwrap_or(&fallback_pattern)
            .to_owned();
        let anchored = format!("^(?:{pattern})$");
        let compiled = self
            .language_patterns
            .entry(anchored.clone())
            .or_insert_with(|| Regex::new(&anchored).ok())
            .as_ref()
            .ok_or_else(|| invalid_catalog_input(format!("invalid Language regex for {key}")))?;
        compiled
            .is_match(&input)
            .map_err(|error| invalid_catalog_input(format!("Language regex failed: {error}")))
    }

    fn type_for_user_input(
        &mut self,
        input: String,
    ) -> Result<Option<WitExpressionTypeOption>, wit_catalog_data::CatalogError> {
        self.catalog()?;
        for matcher in self.type_user_input_matchers.iter() {
            for pattern in &matcher.patterns {
                if pattern.is_match(&input).map_err(|error| {
                    invalid_catalog_input(format!("type user-input regex failed: {error}"))
                })? {
                    return Ok(Some(matcher.option.clone()));
                }
            }
        }
        Ok(None)
    }

    fn change_contract_for_type(
        &mut self,
        class_name: String,
    ) -> Result<Option<wit_catalog_data::TypeChangeContract>, wit_catalog_data::CatalogError> {
        let catalog = self.catalog()?;
        // Classes.getSuperClassInfo first tries an exact class and then the first
        // registered assignable ClassInfo. Catalog type order preserves registration order.
        let type_info = catalog
            .types()
            .find(|type_info| type_info.original_class.as_str() == class_name)
            .or_else(|| {
                catalog.types().find(|type_info| {
                    catalog.is_class_assignable(&class_name, type_info.original_class.as_str())
                })
            });
        Ok(
            type_info.map(|type_info| wit_catalog_data::TypeChangeContract {
                type_class: type_info.original_class.as_str().to_owned(),
                has_changer: type_info.changer.is_some(),
                modes: type_info
                    .changer
                    .as_ref()
                    .into_iter()
                    .flat_map(|modes| modes.iter())
                    .map(|(mode, accepted_types)| WitAcceptedChangeMode {
                        mode: catalog_change_mode_name(*mode).to_owned(),
                        accepted_types: accepted_types
                            .iter()
                            .map(|class_name| class_name.as_str().to_owned())
                            .collect(),
                    })
                    .collect(),
            }),
        )
    }

    fn serialization_contract_for_type(
        &mut self,
        class_name: String,
    ) -> Result<Option<wit_catalog_data::TypeSerializationContract>, wit_catalog_data::CatalogError>
    {
        let catalog = self.catalog()?;
        // Mirrors Classes.getSuperClassInfo: exact ClassInfo first, followed by
        // the first registered assignable type in registration order.
        let type_info = catalog
            .types()
            .find(|type_info| type_info.original_class.as_str() == class_name)
            .or_else(|| {
                catalog.types().find(|type_info| {
                    catalog.is_class_assignable(&class_name, type_info.original_class.as_str())
                })
            });
        Ok(
            type_info.map(|type_info| wit_catalog_data::TypeSerializationContract {
                type_class: type_info.original_class.as_str().to_owned(),
                has_serializer: type_info.has_serializer,
                serialize_as: type_info
                    .serialize_as
                    .as_ref()
                    .map(|class_name| class_name.as_str().to_owned()),
            }),
        )
    }

    fn type_literal_matches(
        &mut self,
        input: String,
    ) -> Result<Vec<WitExpressionLiteralOption>, wit_catalog_data::CatalogError> {
        let catalog = self.catalog()?;
        let end = u64::try_from(input.len()).map_err(|_| {
            invalid_catalog_input("literal input length does not fit the WIT range")
        })?;
        Ok(catalog
            .type_literal_matches(&input)
            .map(|matched| {
                let value = matched.type_info;
                let literal = matched.literal;
                let alias = matches!(matched.source, syntaxes::TypeLiteralSource::Alias)
                    .then(|| catalog.alias(matched.canonical_value))
                    .flatten();
                WitExpressionLiteralOption {
                    source_record: catalog_record_ref(
                        catalog,
                        "Types.json",
                        value.source_index,
                        value.registration_id.as_str(),
                    ),
                    literal_index: matched.literal_index.map(|index| index as u64),
                    code_name: value.code_name.as_str().to_owned(),
                    class_name: value.original_class.as_str().to_owned(),
                    type_parse_order: u64::try_from(value.type_parse_order).unwrap_or(u64::MAX),
                    range: WitTextRange { start: 0, end },
                    canonical_value: matched.canonical_value.to_owned(),
                    source: match matched.source {
                        syntaxes::TypeLiteralSource::ParserPattern => {
                            WitExpressionLiteralSource::ParserPattern
                        }
                        syntaxes::TypeLiteralSource::Supplier => {
                            WitExpressionLiteralSource::Supplier
                        }
                        syntaxes::TypeLiteralSource::EnumConstant => {
                            WitExpressionLiteralSource::EnumConstant
                        }
                        syntaxes::TypeLiteralSource::Alias => WitExpressionLiteralSource::Alias,
                    },
                    plural: matched.plural,
                    addon_name: value.addon.name.clone(),
                    addon_version: value.addon.version.clone(),
                    parser_class: value
                        .parser_class
                        .as_ref()
                        .map(|class| class.as_str().to_owned()),
                    parse_contexts: value.parse_contexts.clone(),
                    value_class: literal.map(|literal| literal.value_class.as_str().to_owned()),
                    represented_class: literal
                        .and_then(|literal| literal.represented_class.as_ref())
                        .map(|class| class.as_str().to_owned()),
                    variable_name: literal.and_then(|literal| literal.variable_name.clone()),
                    debug_text: literal.and_then(|literal| literal.debug_text.clone()),
                    enum_constant: literal.and_then(|literal| literal.enum_constant.clone()),
                    alias_all: alias.map(|target| target.all),
                    alias_type_count: alias
                        .map(|target| u64::try_from(target.types.len()).unwrap_or(u64::MAX)),
                }
            })
            .collect())
    }

    fn is_class_assignable(
        &mut self,
        source_class: String,
        target_class: String,
    ) -> Result<wit_catalog_data::TypeRelation, wit_catalog_data::CatalogError> {
        let catalog = self.catalog()?;
        Ok(catalog_type_relation(
            catalog,
            &source_class,
            &target_class,
            catalog.is_class_assignable(&source_class, &target_class),
        ))
    }

    fn hierarchy_distance(
        &mut self,
        superclass: String,
        subclass: String,
    ) -> Result<Option<u64>, wit_catalog_data::CatalogError> {
        Ok(self.catalog()?.hierarchy_distance(&superclass, &subclass))
    }

    fn difference_options_for_type(
        &mut self,
        input_class: String,
    ) -> Result<Vec<wit_catalog_data::DifferenceOption>, wit_catalog_data::CatalogError> {
        let catalog = self.catalog()?;
        Ok(catalog
            .difference_options_for_type(&input_class)
            .into_iter()
            .map(|difference| wit_catalog_data::DifferenceOption {
                input_class: difference.input_type.as_str().to_owned(),
                return_class: difference.return_type.as_str().to_owned(),
                registration_id: difference.registration_id.as_str().to_owned(),
                registration_order: u64::try_from(difference.registration_order)
                    .unwrap_or(u64::MAX),
                hierarchy_distance: catalog
                    .hierarchy_distance(difference.input_type.as_str(), &input_class),
            })
            .collect())
    }

    fn common_assignable_class(
        &mut self,
        classes: Vec<String>,
    ) -> Result<Option<String>, wit_catalog_data::CatalogError> {
        if classes.is_empty() {
            return Err(invalid_catalog_input(
                "common assignable class requires at least one input class",
            ));
        }
        let catalog = self.catalog()?;
        let classes = classes.into_iter().map(ClassName).collect::<Vec<_>>();
        Ok(catalog
            .common_assignable_classes(&classes)
            .map(|class_name| class_name.0))
    }

    fn can_convert(
        &mut self,
        source_class: String,
        target_class: String,
    ) -> Result<wit_catalog_data::TypeRelation, wit_catalog_data::CatalogError> {
        let catalog = self.catalog()?;
        Ok(catalog_type_relation(
            catalog,
            &source_class,
            &target_class,
            catalog.can_convert(&source_class, &target_class),
        ))
    }

    fn comparator_for_types(
        &mut self,
        first_class: String,
        second_class: String,
    ) -> Result<wit_catalog_data::ComparatorContract, wit_catalog_data::CatalogError> {
        Ok(resolve_comparator_contract(
            self.catalog()?,
            &first_class,
            &second_class,
        ))
    }

    fn property_handlers_for_type(
        &mut self,
        property_name: String,
        source_class: String,
    ) -> Result<Vec<wit_catalog_data::PropertyHandlerContract>, wit_catalog_data::CatalogError>
    {
        let catalog = self.catalog()?;
        if catalog.class(&source_class).is_none() {
            return Ok(Vec::new());
        }
        Ok(catalog
            .properties()
            .iter()
            .filter(|property| property.name == property_name)
            .flat_map(|property| {
                property
                    .related_types
                    .iter()
                    .filter(|handler| {
                        catalog.is_class_assignable(&source_class, handler.type_class.as_str())
                    })
                    .map(|handler| wit_catalog_data::PropertyHandlerContract {
                        property_registration_id: property.registration_id.as_str().to_owned(),
                        property_name: property.name.clone(),
                        input_class: handler.type_class.as_str().to_owned(),
                        handler_class: handler.handler_class.as_str().to_owned(),
                        handler_kind: property_handler_kind_name(&handler.handler_kind).to_owned(),
                        element_types: handler
                            .element_types
                            .clone()
                            .unwrap_or_default()
                            .into_iter()
                            .map(|class| class.as_str().to_owned())
                            .collect(),
                    })
            })
            .collect())
    }
}

impl wit_state_store::Host for StoreData {
    fn get(
        &mut self,
        scope: WitStateScope,
        visibility: WitNamespaceVisibility,
        namespace: String,
        key: String,
    ) -> Result<Option<WitStateValue>, WitStateError> {
        self.invocation()?
            .get(
                state_scope(scope),
                namespace_visibility(visibility),
                &namespace,
                &key,
            )
            .map(|value| value.map(wit_state_value))
            .map_err(wit_state_error)
    }

    fn scan_prefix(
        &mut self,
        scope: WitStateScope,
        visibility: WitNamespaceVisibility,
        namespace: String,
        prefix: String,
        limit: u32,
    ) -> Result<Vec<WitStateEntry>, WitStateError> {
        self.invocation()?
            .scan_prefix(
                state_scope(scope),
                namespace_visibility(visibility),
                &namespace,
                &prefix,
                limit as usize,
            )
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|entry| WitStateEntry {
                        key: entry.key,
                        value: wit_state_value(entry.value),
                    })
                    .collect()
            })
            .map_err(wit_state_error)
    }

    fn put(
        &mut self,
        scope: WitStateScope,
        visibility: WitNamespaceVisibility,
        namespace: String,
        key: String,
        value: WitStateValue,
    ) -> Result<(), WitStateError> {
        self.invocation()?
            .put(
                state_scope(scope),
                namespace_visibility(visibility),
                &namespace,
                &key,
                state_value(value),
            )
            .map_err(wit_state_error)
    }

    fn delete(
        &mut self,
        scope: WitStateScope,
        visibility: WitNamespaceVisibility,
        namespace: String,
        key: String,
    ) -> Result<bool, WitStateError> {
        self.invocation()?
            .delete(
                state_scope(scope),
                namespace_visibility(visibility),
                &namespace,
                &key,
            )
            .map_err(wit_state_error)
    }

    fn compare_and_swap(
        &mut self,
        scope: WitStateScope,
        visibility: WitNamespaceVisibility,
        namespace: String,
        key: String,
        expected: Option<WitStateValue>,
        replacement: Option<WitStateValue>,
    ) -> Result<bool, WitStateError> {
        let expected = expected.map(state_value);
        let replacement = replacement.map(state_value);
        self.invocation()?
            .compare_and_swap(
                state_scope(scope),
                namespace_visibility(visibility),
                &namespace,
                &key,
                expected.as_ref(),
                replacement,
            )
            .map_err(wit_state_error)
    }
}

impl wit_dynamic_registry::Host for StoreData {
    fn register(
        &mut self,
        definition: WitDynamicSyntaxDefinition,
    ) -> Result<(), WitDynamicRegistryError> {
        let component_id = self.dynamic_update()?.component_id().to_owned();
        let metadata = dynamic_metadata(definition.metadata)?;
        let before = definition
            .before
            .into_iter()
            .map(|reference| dynamic_reference(reference, &component_id))
            .collect();
        let after = definition
            .after
            .into_iter()
            .map(|reference| dynamic_reference(reference, &component_id))
            .collect();
        let structure_node_type = definition
            .structure_node_type
            .map(dynamic_structure_node_type);
        let structure_body_mode = definition
            .structure_body_mode
            .map(dynamic_structure_body_mode);
        let entry_validator = dynamic_entry_validator(definition.entry_validator)?;
        let input = DynamicSyntaxInput {
            local_id: definition.local_id,
            kind: catalog_syntax_kind(definition.kind),
            patterns: definition.patterns,
            priority: definition.priority,
            before,
            after,
            return_type: definition.return_type,
            return_multiplicity: definition.return_multiplicity.map(|value| match value {
                WitDynamicMultiplicity::Single => DynamicMultiplicity::Single,
                WitDynamicMultiplicity::Multiple => DynamicMultiplicity::Multiple,
                WitDynamicMultiplicity::Both => DynamicMultiplicity::Both,
            }),
            structure_node_type,
            structure_body_mode,
            entry_validator,
            handler: definition.handler,
            metadata,
        };
        self.dynamic_update()?
            .register(input)
            .map_err(wit_dynamic_registry_error)
    }

    fn register_override(
        &mut self,
        syntax_override: WitDynamicSyntaxOverride,
    ) -> Result<(), WitDynamicRegistryError> {
        let metadata = dynamic_metadata(syntax_override.metadata)?;
        let target = match syntax_override.target {
            WitDynamicSyntaxOverrideTarget::DefinitionId(value) => {
                SyntaxOverrideTarget::Definition(DefinitionId(value))
            }
            WitDynamicSyntaxOverrideTarget::RegistrationId(value) => {
                SyntaxOverrideTarget::Registration(RegistrationId(value))
            }
        };
        self.dynamic_update()?
            .register_override(DynamicSyntaxOverrideInput {
                local_id: syntax_override.local_id,
                target,
                priority: syntax_override.priority,
                handler: syntax_override.handler,
                metadata,
            })
            .map_err(wit_dynamic_registry_error)
    }

    fn remove(&mut self, local_id: String) -> Result<bool, WitDynamicRegistryError> {
        self.dynamic_update()?
            .remove(&local_id)
            .map_err(wit_dynamic_registry_error)
    }
}

fn dynamic_metadata(
    entries: Vec<crate::bindings::nlaocs::skript_parser_addon::types::MetadataEntry>,
) -> Result<BTreeMap<String, String>, WitDynamicRegistryError> {
    let mut metadata = BTreeMap::new();
    for entry in entries {
        if metadata.insert(entry.key.clone(), entry.value).is_some() {
            return Err(WitDynamicRegistryError {
                kind: WitDynamicRegistryErrorKind::InvalidInput,
                message: format!(
                    "dynamic syntax metadata key {} is declared twice",
                    entry.key
                ),
            });
        }
    }
    Ok(metadata)
}

fn dynamic_reference(
    reference: WitDynamicSyntaxReference,
    own_component_id: &str,
) -> SyntaxReference {
    match reference {
        WitDynamicSyntaxReference::Dynamic(id) => SyntaxReference::Dynamic(DynamicSyntaxId::new(
            id.component_id
                .unwrap_or_else(|| own_component_id.to_owned()),
            id.local_id,
        )),
        WitDynamicSyntaxReference::DefinitionId(value) => {
            SyntaxReference::Definition(DefinitionId(value))
        }
        WitDynamicSyntaxReference::RegistrationId(value) => {
            SyntaxReference::Registration(RegistrationId(value))
        }
    }
}

fn catalog_syntax_kind(kind: SyntaxKind) -> CatalogSyntaxKind {
    match kind {
        SyntaxKind::Event => CatalogSyntaxKind::Event,
        SyntaxKind::Condition => CatalogSyntaxKind::Condition,
        SyntaxKind::Effect => CatalogSyntaxKind::Effect,
        SyntaxKind::Expression => CatalogSyntaxKind::Expression,
        SyntaxKind::Type => CatalogSyntaxKind::Type,
        SyntaxKind::Function => CatalogSyntaxKind::Function,
        SyntaxKind::Section => CatalogSyntaxKind::Section,
        SyntaxKind::Structure => CatalogSyntaxKind::Structure,
    }
}

fn dynamic_structure_node_type(value: WitStructureNodeType) -> NodeType {
    match value {
        WitStructureNodeType::Simple => NodeType::Simple,
        WitStructureNodeType::Section => NodeType::Section,
        WitStructureNodeType::Both => NodeType::Both,
    }
}

fn dynamic_structure_body_mode(value: WitStructureBodyMode) -> DynamicStructureBodyMode {
    match value {
        WitStructureBodyMode::None => DynamicStructureBodyMode::None,
        WitStructureBodyMode::Raw => DynamicStructureBodyMode::Raw,
        WitStructureBodyMode::Entries => DynamicStructureBodyMode::Entries,
        WitStructureBodyMode::Trigger => DynamicStructureBodyMode::Trigger,
    }
}

fn dynamic_entry_validator(
    value: Option<WitStructureEntryValidator>,
) -> Result<Option<EntryValidator>, WitDynamicRegistryError> {
    value.map(dynamic_entry_validator_value).transpose()
}

fn dynamic_entry_validator_value(
    value: WitStructureEntryValidator,
) -> Result<EntryValidator, WitDynamicRegistryError> {
    let entries = value.entry_data;
    for (index, entry) in entries.iter().enumerate() {
        let Some(parent) = entry.parent_entry_index else {
            continue;
        };
        let parent = usize::try_from(parent).map_err(|_| {
            dynamic_structure_input_error(format!(
                "Structure EntryData index {index} has an invalid parent index {parent}"
            ))
        })?;
        if parent >= entries.len() || parent == index {
            return Err(dynamic_structure_input_error(format!(
                "Structure EntryData index {index} has an invalid parent index {parent}"
            )));
        }
    }

    let mut visited = vec![false; entries.len()];
    let entry_data = build_dynamic_entry_data(None, &entries, &mut visited, &mut Vec::new())?;
    if let Some((index, _)) = visited.iter().enumerate().find(|(_, visited)| !**visited) {
        return Err(dynamic_structure_input_error(format!(
            "Structure EntryData index {index} is unreachable from the root validator"
        )));
    }
    Ok(EntryValidator { entry_data })
}

fn build_dynamic_entry_data(
    parent: Option<usize>,
    entries: &[WitStructureEntryData],
    visited: &mut [bool],
    path: &mut Vec<usize>,
) -> Result<Vec<EntryData>, WitDynamicRegistryError> {
    let mut output = Vec::new();
    for (index, value) in entries.iter().enumerate() {
        let value_parent = value.parent_entry_index.map(|parent| parent as usize);
        if value_parent != parent {
            continue;
        }
        if visited[index] || path.contains(&index) {
            return Err(dynamic_structure_input_error(format!(
                "Structure EntryData contains a parent cycle at index {index}"
            )));
        }
        visited[index] = true;
        path.push(index);
        let children = build_dynamic_entry_data(Some(index), entries, visited, path)?;
        path.pop();
        let nested_validator = if value.nested_validator_present {
            Some(EntryValidator {
                entry_data: children,
            })
        } else {
            if !children.is_empty() {
                return Err(dynamic_structure_input_error(format!(
                    "Structure EntryData index {index} has nested entries without a nested validator"
                )));
            }
            None
        };
        output.push(dynamic_entry_data(value, nested_validator)?);
    }
    Ok(output)
}

fn dynamic_entry_data(
    value: &WitStructureEntryData,
    nested_validator: Option<EntryValidator>,
) -> Result<EntryData, WitDynamicRegistryError> {
    let default_value = value
        .default_value
        .as_deref()
        .map(|raw| {
            syntaxes::parse_json_value(raw).map_err(|error| WitDynamicRegistryError {
                kind: WitDynamicRegistryErrorKind::InvalidInput,
                message: format!(
                    "Structure EntryData {:?} has an invalid JSON default value: {error}",
                    value.key
                ),
            })
        })
        .transpose()?;
    Ok(EntryData {
        key: value.key.clone(),
        default_value,
        optional: value.optional,
        multiple: value.multiple,
        entry_data_class: ClassName(value.entry_data_class.clone()),
        kind: dynamic_entry_kind(value.kind),
        separator: value.separator.clone(),
        value_type: value.value_type.clone().map(ClassName),
        string_mode: value.string_mode.clone(),
        return_types: value.return_types.iter().cloned().map(ClassName).collect(),
        flags: value.flags,
        nested_validator,
    })
}

fn dynamic_structure_input_error(message: impl Into<String>) -> WitDynamicRegistryError {
    WitDynamicRegistryError {
        kind: WitDynamicRegistryErrorKind::InvalidInput,
        message: message.into(),
    }
}

fn dynamic_entry_kind(value: WitStructureEntryKind) -> EntryKind {
    match value {
        WitStructureEntryKind::Literal => EntryKind::Literal,
        WitStructureEntryKind::VariableString => EntryKind::VariableString,
        WitStructureEntryKind::Expression => EntryKind::Expression,
        WitStructureEntryKind::Trigger => EntryKind::Trigger,
        WitStructureEntryKind::Container => EntryKind::Container,
        WitStructureEntryKind::Section => EntryKind::Section,
        WitStructureEntryKind::KeyValue => EntryKind::KeyValue,
        WitStructureEntryKind::Unknown => EntryKind::Unknown,
    }
}

fn wit_dynamic_registry_error(error: DynamicRegistryError) -> WitDynamicRegistryError {
    let kind = match error {
        DynamicRegistryError::InvalidInput { .. } => WitDynamicRegistryErrorKind::InvalidInput,
        DynamicRegistryError::DuplicateId { .. } => WitDynamicRegistryErrorKind::DuplicateId,
        DynamicRegistryError::UnknownId { .. } => WitDynamicRegistryErrorKind::UnknownId,
        DynamicRegistryError::InvalidPattern { .. } => WitDynamicRegistryErrorKind::InvalidPattern,
        DynamicRegistryError::UnknownDocument { .. } => WitDynamicRegistryErrorKind::NoActiveUpdate,
        DynamicRegistryError::StaleDocumentRevision { .. } => {
            WitDynamicRegistryErrorKind::StaleDocumentRevision
        }
        DynamicRegistryError::Frozen { .. } => WitDynamicRegistryErrorKind::Frozen,
        DynamicRegistryError::UnknownReference { .. }
        | DynamicRegistryError::CrossKindReference { .. } => {
            WitDynamicRegistryErrorKind::UnknownReference
        }
        DynamicRegistryError::PriorityCycle { .. } => WitDynamicRegistryErrorKind::PriorityCycle,
        DynamicRegistryError::Internal { .. } => WitDynamicRegistryErrorKind::Internal,
    };
    WitDynamicRegistryError {
        kind,
        message: error.to_string(),
    }
}

impl StoreData {
    fn catalog(&self) -> Result<&Catalog, wit_catalog_data::CatalogError> {
        self.catalog
            .as_deref()
            .ok_or_else(|| wit_catalog_data::CatalogError {
                kind: wit_catalog_data::CatalogErrorKind::Unavailable,
                message: "catalog data requires an SSG Catalog".to_owned(),
            })
    }

    fn invocation(&mut self) -> Result<&mut InvocationTransaction, WitStateError> {
        self.invocation
            .as_mut()
            .ok_or_else(|| wit_state_error(StateError::NoActiveTransaction))
    }

    fn dynamic_update(&mut self) -> Result<&mut DynamicSyntaxUpdate, WitDynamicRegistryError> {
        if !self.dynamic_syntax_available {
            return Err(WitDynamicRegistryError {
                kind: WitDynamicRegistryErrorKind::Unavailable,
                message: "dynamic syntax registry requires an SSG Catalog".to_owned(),
            });
        }
        self.dynamic_syntax_update
            .as_mut()
            .ok_or_else(|| WitDynamicRegistryError {
                kind: WitDynamicRegistryErrorKind::NoActiveUpdate,
                message: "dynamic syntax updates are only available during initialization and document prepass hooks".to_owned(),
            })
    }
}

fn invalid_catalog_input(message: impl Into<String>) -> wit_catalog_data::CatalogError {
    wit_catalog_data::CatalogError {
        kind: wit_catalog_data::CatalogErrorKind::InvalidInput,
        message: message.into(),
    }
}

fn catalog_type_relation(
    catalog: &Catalog,
    source_class: &str,
    target_class: &str,
    compatible: bool,
) -> wit_catalog_data::TypeRelation {
    if compatible {
        wit_catalog_data::TypeRelation::Compatible
    } else if catalog.class(source_class).is_none() || catalog.class(target_class).is_none() {
        wit_catalog_data::TypeRelation::Unknown
    } else {
        wit_catalog_data::TypeRelation::Incompatible
    }
}

fn resolve_comparator_contract(
    catalog: &Catalog,
    first_class: &str,
    second_class: &str,
) -> wit_catalog_data::ComparatorContract {
    use wit_catalog_data::{ComparatorContract, TypeRelation};

    let unresolved = || ComparatorContract {
        relation: TypeRelation::Unknown,
        supports_ordering: None,
        supports_inversion: None,
        registration_id: None,
        reversed: false,
    };
    if catalog.class(first_class).is_none() || catalog.class(second_class).is_none() {
        return unresolved();
    }
    let default_equals = || ComparatorContract {
        relation: TypeRelation::Compatible,
        supports_ordering: Some(false),
        supports_inversion: Some(true),
        registration_id: None,
        reversed: false,
    };
    let resolved = |comparator: &syntaxes::Comparator, reversed| ComparatorContract {
        relation: TypeRelation::Compatible,
        supports_ordering: comparator.supports_ordering,
        supports_inversion: comparator.supports_inversion,
        registration_id: Some(comparator.registration_id.as_str().to_owned()),
        reversed,
    };
    let comparators = catalog.comparators();
    if let Some(comparator) = comparators.iter().find(|comparator| {
        comparator.first_type.as_str() == first_class
            && comparator.second_type.as_str() == second_class
    }) {
        return resolved(comparator, false);
    }
    if let Some(comparator) = comparators.iter().find(|comparator| {
        catalog.is_class_assignable(first_class, comparator.first_type.as_str())
            && catalog.is_class_assignable(second_class, comparator.second_type.as_str())
    }) {
        return resolved(comparator, false);
    }
    if let Some(comparator) = comparators.iter().find(|comparator| {
        comparator.supports_inversion != Some(false)
            && catalog.is_class_assignable(first_class, comparator.second_type.as_str())
            && catalog.is_class_assignable(second_class, comparator.first_type.as_str())
    }) {
        return resolved(comparator, true);
    }
    if first_class == second_class && first_class != "java.lang.Object" {
        return default_equals();
    }

    let unrelated = catalog
        .common_assignable_class(first_class, second_class)
        .is_some_and(|class| class.as_str() == "java.lang.Object");
    if unrelated
        && (catalog.can_convert(first_class, second_class)
            || catalog.can_convert(second_class, first_class))
    {
        return default_equals();
    }
    if let Some(comparator) = comparators.iter().find(|comparator| {
        (catalog.is_class_assignable(first_class, comparator.first_type.as_str())
            && catalog.can_convert(second_class, comparator.second_type.as_str()))
            || (catalog.can_convert(first_class, comparator.first_type.as_str())
                && catalog.is_class_assignable(second_class, comparator.second_type.as_str()))
            || (catalog.can_convert(first_class, comparator.first_type.as_str())
                && catalog.can_convert(second_class, comparator.second_type.as_str()))
    }) {
        return resolved(comparator, false);
    }
    if let Some(comparator) = comparators.iter().find(|comparator| {
        comparator.supports_inversion != Some(false)
            && ((catalog.is_class_assignable(first_class, comparator.second_type.as_str())
                && catalog.can_convert(second_class, comparator.first_type.as_str()))
                || (catalog.can_convert(first_class, comparator.second_type.as_str())
                    && catalog.is_class_assignable(second_class, comparator.first_type.as_str()))
                || (catalog.can_convert(first_class, comparator.second_type.as_str())
                    && catalog.can_convert(second_class, comparator.first_type.as_str())))
    }) {
        return resolved(comparator, true);
    }

    let first_type = catalog
        .types()
        .find(|ty| ty.original_class.as_str() == first_class)
        .or_else(|| {
            catalog
                .types()
                .find(|ty| catalog.is_class_assignable(first_class, ty.original_class.as_str()))
        });
    let second_type = catalog
        .types()
        .find(|ty| ty.original_class.as_str() == second_class)
        .or_else(|| {
            catalog
                .types()
                .find(|ty| catalog.is_class_assignable(second_class, ty.original_class.as_str()))
        });
    if first_type.is_some_and(|first| {
        first.original_class.as_str() != "java.lang.Object"
            && second_type.is_some_and(|second| second.code_name == first.code_name)
    }) {
        return default_equals();
    }
    ComparatorContract {
        relation: TypeRelation::Incompatible,
        supports_ordering: None,
        supports_inversion: None,
        registration_id: None,
        reversed: false,
    }
}

fn catalog_record_ref(
    catalog: &Catalog,
    document: &str,
    index: usize,
    registration_id: &str,
) -> Option<WitCatalogRecordRef> {
    let source = catalog.source()?;
    let record = source
        .records_by_registration_id(registration_id)
        .iter()
        .find(|record| record.document == document && record.index == index)?;
    Some(WitCatalogRecordRef {
        source_digest: source.source_digest.clone(),
        snapshot_id: source.snapshot_id.clone(),
        document: record.document.clone(),
        index: record.index as u64,
        byte_length: record.json.len() as u64,
    })
}

fn catalog_chunk(
    bytes: &[u8],
    offset: u64,
    max_bytes: u32,
    response_limit: usize,
) -> Result<wit_catalog_data::CatalogChunk, wit_catalog_data::CatalogError> {
    if max_bytes == 0 {
        return Err(invalid_catalog_input("catalog chunk size must be non-zero"));
    }
    let offset = usize::try_from(offset)
        .map_err(|_| invalid_catalog_input("catalog chunk offset is too large"))?;
    if offset > bytes.len() {
        return Err(invalid_catalog_input(format!(
            "catalog chunk offset {offset} exceeds the {}-byte value",
            bytes.len()
        )));
    }
    let length = (max_bytes as usize)
        .min(response_limit)
        .min(bytes.len() - offset);
    Ok(wit_catalog_data::CatalogChunk {
        media_type: "application/json".to_owned(),
        offset: offset as u64,
        total_length: bytes.len() as u64,
        bytes: bytes[offset..offset + length].to_vec(),
    })
}

fn catalog_record_page(
    records: &[CatalogSourceRecord],
    source_digest: &str,
    snapshot_id: &str,
    offset: u64,
    limit: u32,
    response_limit: usize,
) -> Result<wit_catalog_data::CatalogRecordPage, wit_catalog_data::CatalogError> {
    let (offset, end) = catalog_page_bounds(records.len(), offset, limit)?;
    let mut bytes = 0usize;
    let mut items = Vec::new();
    for record in &records[offset..end] {
        let item_bytes = source_digest.len() + snapshot_id.len() + record.document.len() + 24;
        if bytes.saturating_add(item_bytes) > response_limit {
            if items.is_empty() {
                return Err(wit_catalog_data::CatalogError {
                    kind: wit_catalog_data::CatalogErrorKind::ResponseTooLarge,
                    message: "one catalog record reference exceeds the response limit".to_owned(),
                });
            }
            break;
        }
        bytes += item_bytes;
        items.push(WitCatalogRecordRef {
            source_digest: source_digest.to_owned(),
            snapshot_id: snapshot_id.to_owned(),
            document: record.document.clone(),
            index: record.index as u64,
            byte_length: record.json.len() as u64,
        });
    }
    let next = offset + items.len();
    Ok(wit_catalog_data::CatalogRecordPage {
        items,
        next_offset: (next < records.len()).then_some(next as u64),
    })
}

fn catalog_document_page(
    documents: &[(&str, usize)],
    offset: u64,
    limit: u32,
    response_limit: usize,
) -> Result<wit_catalog_data::CatalogDocumentPage, wit_catalog_data::CatalogError> {
    let (offset, end) = catalog_page_bounds(documents.len(), offset, limit)?;
    let mut bytes = 0usize;
    let mut items = Vec::new();
    for (name, byte_length) in &documents[offset..end] {
        let item_bytes = name.len() + 32;
        if bytes.saturating_add(item_bytes) > response_limit {
            if items.is_empty() {
                return Err(wit_catalog_data::CatalogError {
                    kind: wit_catalog_data::CatalogErrorKind::ResponseTooLarge,
                    message: "one catalog document descriptor exceeds the response limit"
                        .to_owned(),
                });
            }
            break;
        }
        bytes += item_bytes;
        items.push(wit_catalog_data::CatalogDocumentInfo {
            name: (*name).to_owned(),
            media_type: "application/json".to_owned(),
            byte_length: *byte_length as u64,
        });
    }
    let next = offset + items.len();
    Ok(wit_catalog_data::CatalogDocumentPage {
        items,
        next_offset: (next < documents.len()).then_some(next as u64),
    })
}

fn catalog_page_bounds(
    length: usize,
    offset: u64,
    limit: u32,
) -> Result<(usize, usize), wit_catalog_data::CatalogError> {
    if limit == 0 {
        return Err(invalid_catalog_input("catalog page size must be non-zero"));
    }
    let offset = usize::try_from(offset)
        .map_err(|_| invalid_catalog_input("catalog page offset is too large"))?;
    if offset > length {
        return Err(invalid_catalog_input(format!(
            "catalog page offset {offset} exceeds the {length}-item collection"
        )));
    }
    Ok((offset, offset.saturating_add(limit as usize).min(length)))
}

struct ComponentEntry {
    manifest: ComponentManifest,
    registered_handler_bindings: Vec<WitRegisteredHandlerBinding>,
    store: Store<StoreData>,
    bindings: ParserAddon,
    load_order: usize,
    disabled: bool,
    unloaded: bool,
}

#[derive(Clone)]
struct RegisteredSubscription {
    component_index: usize,
    load_order: usize,
    declaration_order: usize,
    subscription: HookSubscription,
}

#[derive(Default)]
struct SubscriptionRegistry {
    subscriptions: Vec<RegisteredSubscription>,
}

impl SubscriptionRegistry {
    fn register(
        &mut self,
        component_index: usize,
        load_order: usize,
        subscriptions: &[HookSubscription],
    ) {
        self.subscriptions
            .extend(
                subscriptions
                    .iter()
                    .enumerate()
                    .map(|(declaration_order, subscription)| RegisteredSubscription {
                        component_index,
                        load_order,
                        declaration_order,
                        subscription: subscription.clone(),
                    }),
            );
    }

    fn matching(&self, target: &DispatchTarget, phase: HookPhase) -> Vec<RegisteredSubscription> {
        let mut matching = self
            .subscriptions
            .iter()
            .filter(|entry| entry.subscription.phase == phase)
            .filter_map(|entry| {
                target_specificity(&entry.subscription.target, target)
                    .map(|specificity| (specificity, entry.clone()))
            })
            .collect::<Vec<_>>();
        matching.sort_by(|(left_specificity, left), (right_specificity, right)| {
            right_specificity
                .cmp(left_specificity)
                .then_with(|| left.subscription.priority.cmp(&right.subscription.priority))
                .then_with(|| left.load_order.cmp(&right.load_order))
                .then_with(|| left.declaration_order.cmp(&right.declaration_order))
        });
        matching.into_iter().map(|(_, entry)| entry).collect()
    }

    fn matching_capability(
        &self,
        target: &DispatchTarget,
        phase: HookPhase,
        capability_id: &str,
    ) -> Vec<RegisteredSubscription> {
        self.matching(target, phase)
            .into_iter()
            .filter(|entry| entry.subscription.capability_id == capability_id)
            .collect()
    }

    fn has_active_matching_capability(
        &self,
        components: &[ComponentEntry],
        target: &DispatchTarget,
        phase: HookPhase,
        capability_id: &str,
    ) -> bool {
        self.subscriptions.iter().any(|entry| {
            entry.subscription.phase == phase
                && entry.subscription.capability_id == capability_id
                && target_specificity(&entry.subscription.target, target).is_some()
                && components
                    .get(entry.component_index)
                    .is_some_and(|component| !component.disabled && !component.unloaded)
        })
    }

    fn has_active_more_specific_target(
        &self,
        components: &[ComponentEntry],
        target: &DispatchTarget,
        phase: HookPhase,
    ) -> bool {
        self.subscriptions.iter().any(|entry| {
            entry.subscription.phase == phase
                && target_specificity(&entry.subscription.target, target)
                    .is_some_and(|specificity| specificity >= 2)
                && components
                    .get(entry.component_index)
                    .is_some_and(|component| !component.disabled && !component.unloaded)
        })
    }

    fn has_active_matching_handler_for_registration(
        &self,
        components: &[ComponentEntry],
        syntax_kind: SyntaxKind,
        definition_id: Option<&str>,
        registration_id: &str,
        pattern_index: usize,
    ) -> bool {
        self.subscriptions.iter().any(|entry| {
            entry.subscription.phase == HookPhase::Matching
                && entry.subscription.capability_id == CAPABILITY_HOOKS
                && !matches!(entry.subscription.mode, HookMode::Observe)
                && match &entry.subscription.target {
                    HookTarget::SyntaxKind(kind) => *kind == syntax_kind,
                    HookTarget::Definition(id) => definition_id == Some(id.as_str()),
                    HookTarget::Registration(id) => id == registration_id,
                    HookTarget::Pattern(pattern) => {
                        pattern.registration_id == registration_id
                            && pattern.pattern_index == pattern_index as u64
                    }
                    HookTarget::ParseStage | HookTarget::Parser(_) => false,
                }
                && components
                    .get(entry.component_index)
                    .is_some_and(|component| !component.disabled && !component.unloaded)
        })
    }

    fn has_matching_hooks(&self) -> bool {
        self.subscriptions.iter().any(|entry| {
            entry.subscription.phase == HookPhase::Matching
                && entry.subscription.capability_id == CAPABILITY_HOOKS
        })
    }
}

fn target_specificity(subscription: &HookTarget, requested: &DispatchTarget) -> Option<u8> {
    match (subscription, requested) {
        (HookTarget::ParseStage, DispatchTarget::ParseStage) => Some(0),
        (HookTarget::Parser(subscription_id), DispatchTarget::Parser(requested_id))
            if subscription_id == requested_id =>
        {
            Some(5)
        }
        (HookTarget::SyntaxKind(subscription_kind), requested)
            if dispatch_syntax_kind(requested) == Some(*subscription_kind) =>
        {
            Some(1)
        }
        (HookTarget::Definition(subscription_id), requested)
            if dispatch_definition_id(requested) == Some(subscription_id.as_str()) =>
        {
            Some(2)
        }
        (HookTarget::Registration(subscription_id), requested)
            if dispatch_registration_id(requested) == Some(subscription_id.as_str()) =>
        {
            Some(3)
        }
        (
            HookTarget::Pattern(subscription_pattern),
            DispatchTarget::Pattern {
                registration_id,
                pattern_index,
                ..
            },
        ) if subscription_pattern.registration_id == *registration_id
            && subscription_pattern.pattern_index == *pattern_index =>
        {
            Some(4)
        }
        _ => None,
    }
}

fn dispatch_syntax_kind(target: &DispatchTarget) -> Option<SyntaxKind> {
    match target {
        DispatchTarget::SyntaxKind(kind)
        | DispatchTarget::Definition {
            syntax_kind: kind, ..
        }
        | DispatchTarget::Registration {
            syntax_kind: kind, ..
        }
        | DispatchTarget::Pattern {
            syntax_kind: kind, ..
        } => Some(*kind),
        DispatchTarget::ParseStage | DispatchTarget::Parser(_) => None,
    }
}

fn dispatch_definition_id(target: &DispatchTarget) -> Option<&str> {
    match target {
        DispatchTarget::Definition { definition_id, .. }
        | DispatchTarget::Registration { definition_id, .. }
        | DispatchTarget::Pattern { definition_id, .. } => Some(definition_id),
        DispatchTarget::ParseStage | DispatchTarget::SyntaxKind(_) | DispatchTarget::Parser(_) => {
            None
        }
    }
}

fn dispatch_registration_id(target: &DispatchTarget) -> Option<&str> {
    match target {
        DispatchTarget::Registration {
            registration_id, ..
        }
        | DispatchTarget::Pattern {
            registration_id, ..
        } => Some(registration_id),
        DispatchTarget::ParseStage
        | DispatchTarget::SyntaxKind(_)
        | DispatchTarget::Definition { .. }
        | DispatchTarget::Parser(_) => None,
    }
}

fn apply_catalog_annotations(
    components: &[ComponentEntry],
    target: &DispatchTarget,
    payload: &mut HookPayload,
) {
    let Some(metadata) = payload_metadata_mut(payload) else {
        return;
    };
    let mut matching = components
        .iter()
        .filter(|component| !component.disabled && !component.unloaded)
        .flat_map(|component| {
            component
                .manifest
                .catalog_annotations
                .iter()
                .enumerate()
                .filter(move |(_, annotation)| annotation_matches(&annotation.target, target))
                .map(move |(declaration_order, annotation)| {
                    (
                        catalog_annotation_specificity(&annotation.target),
                        component.load_order,
                        declaration_order,
                        annotation,
                    )
                })
        })
        .collect::<Vec<_>>();
    matching.sort_by_key(|(specificity, load_order, declaration_order, _)| {
        (*specificity, *load_order, *declaration_order)
    });
    for (_, _, _, annotation) in matching {
        for entry in &annotation.metadata {
            if let Some(existing) = metadata.iter_mut().find(|existing| {
                existing.owner_component_id == entry.owner_component_id && existing.key == entry.key
            }) {
                *existing = entry.clone();
            } else {
                metadata.push(entry.clone());
            }
        }
    }
}

fn annotation_matches(target: &CatalogAnnotationTarget, requested: &DispatchTarget) -> bool {
    match target {
        CatalogAnnotationTarget::Definition(id) => {
            dispatch_definition_id(requested) == Some(id.as_str())
        }
        CatalogAnnotationTarget::Registration(id) => {
            dispatch_registration_id(requested) == Some(id.as_str())
        }
        CatalogAnnotationTarget::Pattern(pattern) => matches!(
            requested,
            DispatchTarget::Pattern {
                registration_id,
                pattern_index,
                ..
            } if registration_id == &pattern.registration_id
                && pattern_index == &pattern.pattern_index
        ),
    }
}

fn catalog_annotation_specificity(target: &CatalogAnnotationTarget) -> u8 {
    match target {
        CatalogAnnotationTarget::Definition(_) => 0,
        CatalogAnnotationTarget::Registration(_) => 1,
        CatalogAnnotationTarget::Pattern(_) => 2,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectorMatch {
    Match,
    NoMatch,
    Unknown,
}

impl SelectorMatch {
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::NoMatch, _) | (_, Self::NoMatch) => Self::NoMatch,
            (Self::Unknown, _) | (_, Self::Unknown) => Self::Unknown,
            (Self::Match, Self::Match) => Self::Match,
        }
    }
}

fn selector_match(
    selector: &HookSelector,
    payload: &HookPayload,
    catalog: Option<&Catalog>,
) -> SelectorMatch {
    let mut result = SelectorMatch::Match;
    if let Some(expected) = selector.pattern_index {
        result = result.and(match payload_pattern_index(payload) {
            Some(actual) if actual == expected => SelectorMatch::Match,
            Some(_) => SelectorMatch::NoMatch,
            None => SelectorMatch::Unknown,
        });
    }
    if let Some(expected) = selector.pattern_source.as_deref() {
        result = result.and(match payload_pattern_source(payload) {
            Some(actual) if actual == expected => SelectorMatch::Match,
            Some(_) => SelectorMatch::NoMatch,
            None => SelectorMatch::Unknown,
        });
    }
    if let Some(expected) = selector.mark {
        result = result.and(match payload_mark(payload) {
            Some(actual) if actual == expected => SelectorMatch::Match,
            Some(_) => SelectorMatch::NoMatch,
            None => SelectorMatch::Unknown,
        });
    }
    if !selector.tags.is_empty() {
        result = result.and(match payload_tags(payload) {
            Some(actual)
                if selector
                    .tags
                    .iter()
                    .all(|expected| actual.iter().any(|value| *value == expected)) =>
            {
                SelectorMatch::Match
            }
            Some(_) => SelectorMatch::NoMatch,
            None => SelectorMatch::Unknown,
        });
    }
    if !selector.captures.is_empty() {
        result = result.and(match payload_parsed_captures(payload) {
            Some(captures)
                if selector.captures.iter().all(|expected| {
                    captures.iter().any(|capture| {
                        capture.capture_index == expected.capture_index
                            && expected.status.as_ref().is_none_or(|status| {
                                mem::discriminant(&capture.status) == mem::discriminant(status)
                            })
                    })
                }) =>
            {
                SelectorMatch::Match
            }
            Some(_) => SelectorMatch::NoMatch,
            None => SelectorMatch::Unknown,
        });
    }
    if let Some(expected) = selector.return_type.as_ref() {
        result = result.and(match payload_effective_return_type(payload) {
            Some(actual) => type_selector_match(actual, expected, catalog),
            None => SelectorMatch::Unknown,
        });
    }
    if let Some(expected) = selector.multiplicity.as_ref() {
        result = result.and(match payload_effective_multiplicity(payload) {
            Some(actual) if mem::discriminant(actual) == mem::discriminant(expected) => {
                SelectorMatch::Match
            }
            Some(_) => SelectorMatch::NoMatch,
            None => SelectorMatch::Unknown,
        });
    }
    if !selector.metadata.is_empty() {
        result =
            result.and(match payload_metadata(payload) {
                Some(actual)
                    if selector.metadata.iter().all(|expected| {
                        actual.iter().any(|entry| {
                            expected.owner_component_id.as_ref().is_none_or(|owner| {
                                entry.owner_component_id.as_ref() == Some(owner)
                            }) && entry.key == expected.key
                                && entry.value == expected.value
                        })
                    }) =>
                {
                    SelectorMatch::Match
                }
                Some(_) => SelectorMatch::NoMatch,
                None => SelectorMatch::Unknown,
            });
    }
    result
}

fn payload_pattern_index(payload: &HookPayload) -> Option<u64> {
    match payload {
        HookPayload::Matching(value) => value.pattern_index,
        HookPayload::RegisteredExpression(value) => Some(value.pattern_index),
        HookPayload::Condition(value) => Some(value.candidate.pattern_index),
        HookPayload::Effect(value) => value
            .candidate
            .as_ref()
            .map(|candidate| candidate.pattern_index),
        HookPayload::Section(value) => Some(value.candidate.pattern_index),
        HookPayload::Structure(value) => Some(value.candidate.pattern_index),
        _ => None,
    }
}

fn payload_pattern_source(payload: &HookPayload) -> Option<&str> {
    match payload {
        HookPayload::Matching(value) => value.pattern.as_deref(),
        HookPayload::RegisteredExpression(value) => Some(&value.pattern),
        HookPayload::Condition(value) => Some(&value.candidate.pattern),
        HookPayload::Effect(value) => value
            .candidate
            .as_ref()
            .map(|candidate| candidate.pattern.as_str()),
        HookPayload::Structure(value) => Some(&value.candidate.pattern),
        _ => None,
    }
}

fn payload_mark(payload: &HookPayload) -> Option<i32> {
    match payload {
        HookPayload::RegisteredExpression(value) => Some(value.mark),
        HookPayload::Condition(value) => Some(value.candidate.mark),
        HookPayload::Effect(value) => value.candidate.as_ref().map(|candidate| candidate.mark),
        _ => None,
    }
}

fn payload_tags(payload: &HookPayload) -> Option<Vec<&str>> {
    match payload {
        HookPayload::RegisteredExpression(value) => {
            Some(value.tags.iter().map(|tag| tag.value.as_str()).collect())
        }
        HookPayload::Condition(value) => Some(
            value
                .candidate
                .tags
                .iter()
                .map(|tag| tag.value.as_str())
                .collect(),
        ),
        HookPayload::Effect(value) => value.candidate.as_ref().map(|candidate| {
            candidate
                .tags
                .iter()
                .map(|tag| tag.value.as_str())
                .collect()
        }),
        _ => None,
    }
}

fn payload_parsed_captures(payload: &HookPayload) -> Option<&[WitParsedCapture]> {
    match payload {
        HookPayload::RegisteredExpression(value) => Some(&value.parsed_captures),
        HookPayload::Effect(value) => value
            .candidate
            .as_ref()
            .map(|candidate| candidate.parsed_captures.as_slice()),
        HookPayload::Section(value) => Some(&value.candidate.parsed_captures),
        HookPayload::Structure(value) => Some(&value.candidate.parsed_captures),
        _ => None,
    }
}

fn payload_effective_return_type(payload: &HookPayload) -> Option<&str> {
    match payload {
        HookPayload::RegisteredExpression(value) => value.effective_return_type.as_deref(),
        _ => None,
    }
}

fn payload_effective_multiplicity(payload: &HookPayload) -> Option<&WitDynamicMultiplicity> {
    match payload {
        HookPayload::RegisteredExpression(value) => value.effective_multiplicity.as_ref(),
        _ => None,
    }
}

fn payload_metadata(payload: &HookPayload) -> Option<&[WitMetadataEntry]> {
    match payload {
        HookPayload::Matching(value) => Some(&value.metadata),
        HookPayload::RegisteredExpression(value) => Some(&value.metadata),
        HookPayload::Condition(value) => Some(&value.candidate.metadata),
        HookPayload::Effect(value) => value
            .candidate
            .as_ref()
            .map(|candidate| candidate.metadata.as_slice())
            .or_else(|| {
                value
                    .near_match
                    .as_ref()
                    .map(|candidate| candidate.metadata.as_slice())
            }),
        HookPayload::Section(value) => Some(&value.candidate.metadata),
        HookPayload::Structure(value) => Some(&value.candidate.metadata),
        _ => None,
    }
}

fn payload_metadata_mut(payload: &mut HookPayload) -> Option<&mut Vec<WitMetadataEntry>> {
    match payload {
        HookPayload::Matching(value) => Some(&mut value.metadata),
        HookPayload::RegisteredExpression(value) => Some(&mut value.metadata),
        HookPayload::Condition(value) => Some(&mut value.candidate.metadata),
        HookPayload::Effect(value) => {
            if let Some(candidate) = value.candidate.as_mut() {
                Some(&mut candidate.metadata)
            } else {
                value
                    .near_match
                    .as_mut()
                    .map(|candidate| &mut candidate.metadata)
            }
        }
        HookPayload::Section(value) => Some(&mut value.candidate.metadata),
        HookPayload::Structure(value) => Some(&mut value.candidate.metadata),
        _ => None,
    }
}

fn type_selector_match(
    actual: &str,
    expected: &crate::bindings::nlaocs::skript_parser_addon::types::ReturnTypeSelector,
    catalog: Option<&Catalog>,
) -> SelectorMatch {
    if actual == expected.class_name {
        return SelectorMatch::Match;
    }
    match expected.relation {
        SelectorTypeRelation::Exact => SelectorMatch::NoMatch,
        SelectorTypeRelation::Assignable | SelectorTypeRelation::Convertible => {
            let Some(catalog) = catalog else {
                return SelectorMatch::Unknown;
            };
            let matched = match expected.relation {
                SelectorTypeRelation::Assignable => {
                    catalog.is_class_assignable(actual, &expected.class_name)
                }
                SelectorTypeRelation::Convertible => {
                    catalog.can_convert(actual, &expected.class_name)
                }
                SelectorTypeRelation::Exact => unreachable!(),
            };
            if matched {
                SelectorMatch::Match
            } else if catalog.class(actual).is_none()
                || catalog.class(&expected.class_name).is_none()
            {
                SelectorMatch::Unknown
            } else {
                SelectorMatch::NoMatch
            }
        }
    }
}

#[derive(Debug, Clone)]
struct HookEffectsCheckpoint {
    effects: HookEffects,
    calls: Vec<HookCall>,
    failures: Vec<ComponentFailure>,
}

impl HookEffectsCheckpoint {
    fn capture(effects: &HookEffects, calls: &[HookCall], failures: &[ComponentFailure]) -> Self {
        Self {
            effects: effects.clone(),
            calls: calls.to_vec(),
            failures: failures.to_vec(),
        }
    }

    fn restore(
        &self,
        effects: &mut HookEffects,
        calls: &mut Vec<HookCall>,
        failures: &mut Vec<ComponentFailure>,
    ) {
        effects.clone_from(&self.effects);
        calls.clone_from(&self.calls);
        failures.clone_from(&self.failures);
    }
}

struct PatternMatchFrame {
    base: StateSavepoint,
    base_effects: HookEffectsCheckpoint,
    selected: Option<StateSavepoint>,
    selected_effects: Option<HookEffectsCheckpoint>,
    candidate_range: Option<ParserTextRange>,
    scope_depth: usize,
    branch_count: usize,
}

struct PatternScopeFrame {
    scope: PatternHookScope,
    state: StateSavepoint,
    effects: HookEffectsCheckpoint,
}

struct PatternBranchState {
    state: StateSavepoint,
    effects: HookEffectsCheckpoint,
}

#[derive(Debug, Clone)]
struct SavedPatternCandidate {
    state: StateSavepoint,
    effects: HookEffectsCheckpoint,
}

fn hook_outcome_accepted(outcome: &PatternHookOutcome, control: &PatternHookControl) -> bool {
    match control {
        PatternHookControl::Continue => matches!(outcome, PatternHookOutcome::Matched { .. }),
        PatternHookControl::Match(range) => {
            matches!(outcome, PatternHookOutcome::Matched { range: matched } if matched == range)
                || matches!(outcome, PatternHookOutcome::Failed { .. })
        }
        PatternHookControl::Fail(_) => false,
    }
}

struct WasmPatternHooks<'a> {
    host: &'a mut ParserHost,
    transaction: &'a ParseTransaction,
    dynamic_snapshot: Option<&'a DynamicSyntaxSnapshot>,
    matching_hooks_registered: bool,
    context: InvocationContext,
    input: String,
    frames: Vec<PatternMatchFrame>,
    scope_frames: Vec<PatternScopeFrame>,
    branch_states: Vec<PatternBranchState>,
    last_candidate: Option<SavedPatternCandidate>,
    effects: HookEffects,
    calls: Vec<HookCall>,
    failures: Vec<ComponentFailure>,
}

impl WasmPatternHooks<'_> {
    fn begin_match_frame(&mut self) -> Result<(), String> {
        self.frames.push(PatternMatchFrame {
            base: self
                .transaction
                .savepoint()
                .map_err(|error| error.to_string())?,
            base_effects: HookEffectsCheckpoint::capture(
                &self.effects,
                &self.calls,
                &self.failures,
            ),
            selected: None,
            selected_effects: None,
            candidate_range: None,
            scope_depth: self.scope_frames.len(),
            branch_count: self.branch_states.len(),
        });
        Ok(())
    }

    fn finish_match_frame(&mut self, accepted: bool) -> Result<(), String> {
        let frame = self
            .frames
            .pop()
            .ok_or_else(|| "matcher frame finished without a matching begin".to_owned())?;
        let (savepoint, effects) = if accepted {
            let savepoint = frame
                .selected
                .ok_or_else(|| "accepted matcher frame has no selected candidate".to_owned())?;
            let effects = frame.selected_effects.ok_or_else(|| {
                "accepted matcher frame has no selected effects checkpoint".to_owned()
            })?;
            (savepoint, effects)
        } else {
            (frame.base, frame.base_effects)
        };
        self.transaction
            .rollback_to(&savepoint)
            .map_err(|error| error.to_string())?;
        effects.restore(&mut self.effects, &mut self.calls, &mut self.failures);
        self.scope_frames.truncate(frame.scope_depth);
        self.branch_states.truncate(frame.branch_count);
        Ok(())
    }

    fn checkpoint_branch_state(&mut self) -> Result<Option<u64>, String> {
        let checkpoint = u64::try_from(self.branch_states.len())
            .map_err(|_| "matcher branch checkpoint index does not fit u64".to_owned())?;
        self.branch_states.push(PatternBranchState {
            state: self
                .transaction
                .savepoint()
                .map_err(|error| error.to_string())?,
            effects: HookEffectsCheckpoint::capture(&self.effects, &self.calls, &self.failures),
        });
        Ok(Some(checkpoint))
    }

    fn restore_branch_state(&mut self, checkpoint: u64) -> Result<(), String> {
        let checkpoint = usize::try_from(checkpoint)
            .map_err(|_| "matcher branch checkpoint index does not fit usize".to_owned())?;
        let state = self
            .branch_states
            .get(checkpoint)
            .ok_or_else(|| "matcher branch checkpoint is no longer available".to_owned())?;
        self.transaction
            .rollback_to(&state.state)
            .map_err(|error| error.to_string())?;
        state
            .effects
            .restore(&mut self.effects, &mut self.calls, &mut self.failures);
        Ok(())
    }

    fn prepare_nested_scope(&mut self, scope: PatternHookScope) -> Result<(), String> {
        if !self.matching_hooks_registered
            || !matches!(
                scope,
                PatternHookScope::Registration | PatternHookScope::Pattern
            )
        {
            return Ok(());
        }
        self.scope_frames.push(PatternScopeFrame {
            scope,
            state: self
                .transaction
                .savepoint()
                .map_err(|error| error.to_string())?,
            effects: HookEffectsCheckpoint::capture(&self.effects, &self.calls, &self.failures),
        });
        Ok(())
    }

    fn finish_nested_scope(
        &mut self,
        scope: PatternHookScope,
        outcome: &PatternHookOutcome,
        control: &PatternHookControl,
    ) -> Result<(), String> {
        if !self.matching_hooks_registered
            || !matches!(
                scope,
                PatternHookScope::Registration | PatternHookScope::Pattern
            )
        {
            return Ok(());
        }
        let frame = self
            .scope_frames
            .pop()
            .ok_or_else(|| "matching scope finished without a checkpoint".to_owned())?;
        if frame.scope != scope {
            return Err("matching scopes finished out of order".to_owned());
        }
        if hook_outcome_accepted(outcome, control) {
            return Ok(());
        }
        self.transaction
            .rollback_to(&frame.state)
            .map_err(|error| error.to_string())?;
        frame
            .effects
            .restore(&mut self.effects, &mut self.calls, &mut self.failures);
        Ok(())
    }

    fn restore_candidate_state(
        &mut self,
        scope: PatternHookScope,
        timing: PatternHookTiming,
        outcome: &PatternHookOutcome,
        control: &PatternHookControl,
    ) -> Result<(), String> {
        if scope != PatternHookScope::Definition || timing == PatternHookTiming::Before {
            return Ok(());
        }

        let frame = self
            .frames
            .last_mut()
            .ok_or_else(|| "definition hook ran outside a matcher frame".to_owned())?;
        let accepted = match control {
            PatternHookControl::Continue => {
                matches!(outcome, PatternHookOutcome::Matched { .. })
            }
            PatternHookControl::Match(range) => Some(*range) == frame.candidate_range,
            PatternHookControl::Fail(_) => false,
        };
        if accepted {
            self.last_candidate = Some(SavedPatternCandidate {
                state: self
                    .transaction
                    .savepoint()
                    .map_err(|error| error.to_string())?,
                effects: HookEffectsCheckpoint::capture(&self.effects, &self.calls, &self.failures),
            });
        }
        if accepted && frame.selected.is_none() {
            frame.selected = Some(
                self.transaction
                    .savepoint()
                    .map_err(|error| error.to_string())?,
            );
            frame.selected_effects = Some(HookEffectsCheckpoint::capture(
                &self.effects,
                &self.calls,
                &self.failures,
            ));
            return Ok(());
        }

        let savepoint = frame.selected.as_ref().unwrap_or(&frame.base).clone();
        let effects = frame
            .selected_effects
            .as_ref()
            .unwrap_or(&frame.base_effects);
        self.transaction
            .rollback_to(&savepoint)
            .map_err(|error| error.to_string())?;
        effects.restore(&mut self.effects, &mut self.calls, &mut self.failures);
        Ok(())
    }

    fn finish_hook_state(
        &mut self,
        scope: PatternHookScope,
        timing: PatternHookTiming,
        outcome: &PatternHookOutcome,
        control: &PatternHookControl,
    ) -> Result<(), String> {
        self.restore_candidate_state(scope, timing, outcome, control)?;
        if timing == PatternHookTiming::After {
            self.finish_nested_scope(scope, outcome, control)?;
        }
        Ok(())
    }

    fn prepare_definition_candidate(&mut self, range: ParserTextRange) -> Result<(), String> {
        self.last_candidate = None;
        let frame = self
            .frames
            .last_mut()
            .ok_or_else(|| "definition hook ran outside a matcher frame".to_owned())?;
        let effects = &frame.base_effects;
        self.transaction
            .rollback_to(&frame.base)
            .map_err(|error| error.to_string())?;
        effects.restore(&mut self.effects, &mut self.calls, &mut self.failures);
        frame.candidate_range = Some(range);
        Ok(())
    }

    fn into_parts(self) -> (HookEffects, Vec<HookCall>, Vec<ComponentFailure>) {
        (self.effects, self.calls, self.failures)
    }

    fn resolve_condition_candidate(
        &mut self,
        request: ConditionSemanticRequest<'_>,
    ) -> Result<ConditionSemanticDecision, String> {
        let catalog = self
            .host
            .config
            .syntax_catalog
            .clone()
            .ok_or_else(|| "syntax catalog is unavailable".to_owned())?;
        let payload = condition_hook_payload(
            request.input,
            request.context,
            request.candidate,
            catalog.as_ref(),
        )?;
        let target = DispatchTarget::Pattern {
            definition_id: payload.candidate.definition_id.clone(),
            registration_id: payload.candidate.registration_id.clone(),
            pattern_index: payload.candidate.pattern_index,
            syntax_kind: SyntaxKind::Condition,
        };
        let result = self
            .host
            .dispatch_in_parse(
                self.transaction,
                DispatchRequest {
                    context: self.context.clone(),
                    target,
                    phase: HookPhase::Condition,
                    payload: HookPayload::Condition(payload.clone()),
                },
            )
            .map_err(|error| error.to_string())?;
        let rejection_diagnostics =
            semantic_rejection_diagnostics(&result.decision, &result.effects)?;
        let updates = result.effects.context_updates.clone();
        merge_effects(&mut self.effects, result.effects);
        self.calls.extend(result.calls);
        self.failures.extend(result.failures);
        let HookPayload::Condition(output) = result.payload else {
            return Err("Condition hook returned a different payload kind".to_owned());
        };
        validate_condition_payload_identity(&payload, &output)
            .map_err(|error| error.to_string())?;
        if let HookDecision::Reject(rejection) = result.decision {
            return Ok(ConditionSemanticDecision::Reject {
                reason: rejection.reason,
                diagnostics: rejection_diagnostics,
            });
        }
        Ok(ConditionSemanticDecision::Accepted {
            context: apply_context_updates(request.context, updates, "Condition")?,
            handler: output.candidate.handler,
            metadata: metadata_entries(output.candidate.metadata)?,
        })
    }

    fn resolve_effect_candidate(
        &mut self,
        request: EffectSemanticRequest<'_>,
    ) -> Result<EffectSemanticDecision, String> {
        let catalog = self
            .host
            .config
            .syntax_catalog
            .clone()
            .ok_or_else(|| "syntax catalog is unavailable".to_owned())?;
        let payload = effect_hook_payload(EffectHookPayloadView {
            input: request.input,
            context: request.context,
            raw_node_id: request.candidate.raw_node_id,
            span: &request.candidate.matched.matched.span.mapped,
            timing: WitEffectTiming::After,
            candidate: Some(request.candidate),
            alternatives: &[],
            failure: None,
            near_match: None,
            catalog: catalog.as_ref(),
        });
        let target = DispatchTarget::Pattern {
            definition_id: request.candidate.matched.definition_id.clone(),
            registration_id: request.candidate.matched.registration_id.clone(),
            pattern_index: u64::try_from(request.candidate.matched.pattern_index)
                .unwrap_or(u64::MAX),
            syntax_kind: wit_syntax_kind(request.candidate.matched.kind),
        };
        let result = self
            .host
            .dispatch_in_parse(
                self.transaction,
                DispatchRequest {
                    context: self.context.clone(),
                    target,
                    phase: HookPhase::Effect,
                    payload: HookPayload::Effect(payload.clone()),
                },
            )
            .map_err(|error| error.to_string())?;
        let rejection_diagnostics =
            semantic_rejection_diagnostics(&result.decision, &result.effects)?;
        let updates = result.effects.context_updates.clone();
        merge_effects(&mut self.effects, result.effects);
        self.calls.extend(result.calls);
        self.failures.extend(result.failures);
        let HookPayload::Effect(output) = result.payload else {
            return Err("Effect hook returned a different payload kind".to_owned());
        };
        validate_effect_payload_identity(&payload, &output, true)
            .map_err(|error| error.to_string())?;
        if let HookDecision::Reject(rejection) = result.decision {
            return Ok(EffectSemanticDecision::Reject {
                reason: rejection.reason,
                diagnostics: rejection_diagnostics,
            });
        }
        let output = output
            .candidate
            .ok_or_else(|| "Effect hook removed the selected candidate".to_owned())?;
        Ok(EffectSemanticDecision::Accepted {
            context: apply_context_updates(request.context, updates, "Effect")?,
            handler: output.handler,
            metadata: metadata_entries(output.metadata)?,
        })
    }
}
impl PatternMatchHooks for WasmPatternHooks<'_> {
    fn begin_match(&mut self) -> Result<(), String> {
        self.begin_match_frame()
    }

    fn finish_match(&mut self, accepted: bool) -> Result<(), String> {
        self.finish_match_frame(accepted)
    }

    fn checkpoint_branch(&mut self) -> Result<Option<u64>, String> {
        self.checkpoint_branch_state()
    }

    fn restore_branch(&mut self, checkpoint: u64) -> Result<(), String> {
        self.restore_branch_state(checkpoint)
    }

    fn allows_regex_pattern(
        &mut self,
        kind: MatchSyntaxKind,
        registration_id: &str,
        pattern_index: usize,
    ) -> Result<bool, String> {
        let static_identity = self
            .host
            .config
            .syntax_catalog
            .as_deref()
            .and_then(|catalog| registered_syntax_identity(catalog, kind, registration_id));
        let identity = static_identity.or_else(|| {
            self.dynamic_snapshot.and_then(|snapshot| {
                let expected_kind = catalog_match_syntax_kind(kind);
                snapshot.definitions.values().find_map(|definition| {
                    (definition.kind == expected_kind
                        && definition.id.qualified() == registration_id)
                        .then_some(RegisteredSyntaxIdentity {
                            kind: definition.kind,
                            definition_id: registration_id,
                            registration_id,
                            pattern_index: Some(pattern_index),
                            pattern_source: definition
                                .patterns
                                .get(pattern_index)
                                .map(|pattern| pattern.source.as_str()),
                            tags: None,
                            mark: None,
                            dynamic_handler: Some(definition.handler.as_str()),
                        })
                })
            })
        });
        let exact_handler = self.matching_hooks_registered
            && self
                .host
                .registry
                .has_active_matching_handler_for_registration(
                    &self.host.components,
                    wit_syntax_kind(kind),
                    identity.map(|identity| identity.definition_id),
                    registration_id,
                    pattern_index,
                );
        if exact_handler {
            return Ok(true);
        }
        let declared_handler = identity
            .is_some_and(|identity| has_registered_syntax_handler(&self.host.components, identity));
        if declared_handler || kind != MatchSyntaxKind::Expression {
            return Ok(declared_handler);
        }
        let dynamic = self
            .host
            .config
            .syntax_catalog
            .as_deref()
            .and_then(|catalog| {
                catalog.expressions().find(|expression| {
                    expression.common.registration_id.as_str() == registration_id
                })
            })
            .is_some_and(|expression| {
                expression.return_type_state != ReturnTypeState::Static
                    || expression.return_type_multiplicity_state == ResolutionState::Unresolved
            });
        Ok(dynamic
            && self.host.registry.has_active_matching_capability(
                &self.host.components,
                &DispatchTarget::ParseStage,
                HookPhase::Expression,
                CAPABILITY_EXPRESSION_PARSER,
            ))
    }

    fn may_override_pattern(
        &self,
        kind: MatchSyntaxKind,
        registration_id: &str,
        pattern_index: usize,
    ) -> bool {
        if !self.matching_hooks_registered {
            return false;
        }
        let definition_id = self
            .host
            .config
            .syntax_catalog
            .as_deref()
            .and_then(|catalog| registered_syntax_identity(catalog, kind, registration_id))
            .map(|identity| identity.definition_id);
        self.host
            .registry
            .has_active_matching_handler_for_registration(
                &self.host.components,
                wit_syntax_kind(kind),
                definition_id,
                registration_id,
                pattern_index,
            )
    }

    fn dispatch(&mut self, event: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
        if event.scope == PatternHookScope::Definition && event.timing == PatternHookTiming::Before
        {
            self.prepare_definition_candidate(event.input_range)?;
        } else if event.timing == PatternHookTiming::Before {
            self.prepare_nested_scope(event.scope)?;
        }
        if !self.matching_hooks_registered {
            let control = PatternHookControl::Continue;
            self.finish_hook_state(event.scope, event.timing, &event.outcome, &control)?;
            return Ok(control);
        }
        let target = match event.scope {
            PatternHookScope::Definition => DispatchTarget::Definition {
                definition_id: event.definition_id.to_owned(),
                syntax_kind: wit_syntax_kind(event.kind),
            },
            PatternHookScope::Registration => DispatchTarget::Registration {
                definition_id: event.definition_id.to_owned(),
                registration_id: event.registration_id.to_owned(),
                syntax_kind: wit_syntax_kind(event.kind),
            },
            PatternHookScope::Pattern | PatternHookScope::Element => DispatchTarget::Pattern {
                definition_id: event.definition_id.to_owned(),
                registration_id: event.registration_id.to_owned(),
                pattern_index: event.pattern_index.unwrap_or_default() as u64,
                syntax_kind: wit_syntax_kind(event.kind),
            },
        };
        if !self.host.registry.has_active_matching_capability(
            &self.host.components,
            &target,
            HookPhase::Matching,
            CAPABILITY_HOOKS,
        ) {
            let control = PatternHookControl::Continue;
            self.finish_hook_state(event.scope, event.timing, &event.outcome, &control)?;
            return Ok(control);
        }
        let original_status = match &event.outcome {
            PatternHookOutcome::Pending => MatchingStatus::Pending,
            PatternHookOutcome::Matched { .. } => MatchingStatus::Matched,
            PatternHookOutcome::Failed { .. } => MatchingStatus::Failed,
        };
        let original_range = WitTextRange {
            start: event.input_range.start as u64,
            end: event.input_range.end as u64,
        };
        let original_element_path = event
            .element_path
            .iter()
            .map(|segment| match segment {
                PatternPathSegment::Element(index) => MatchingPathSegment::Element(*index),
                PatternPathSegment::Branch(index) => MatchingPathSegment::Branch(*index),
            })
            .collect::<Vec<_>>();
        let original_pattern_span = event.pattern_span.map(|span| WitTextRange {
            start: span.start as u64,
            end: span.end as u64,
        });
        let original_span = mapped_span_to_wit(event.input_span.mapped);
        let payload = MatchingPayload {
            input: self.input.clone(),
            pattern: event.pattern.map(ToOwned::to_owned),
            definition_id: event.definition_id.to_owned(),
            registration_id: event.registration_id.to_owned(),
            pattern_index: event.pattern_index.map(|index| index as u64),
            element_path: original_element_path.clone(),
            pattern_span: original_pattern_span,
            scope: match event.scope {
                PatternHookScope::Definition => MatchingScope::Definition,
                PatternHookScope::Registration => MatchingScope::Registration,
                PatternHookScope::Pattern => MatchingScope::Pattern,
                PatternHookScope::Element => MatchingScope::Element,
            },
            timing: match event.timing {
                PatternHookTiming::Before => MatchingTiming::Before,
                PatternHookTiming::After => MatchingTiming::After,
            },
            input_range: original_range,
            span: original_span.clone(),
            status: original_status,
            failure_reason: match &event.outcome {
                PatternHookOutcome::Failed { reason } => Some(reason.clone()),
                PatternHookOutcome::Pending | PatternHookOutcome::Matched { .. } => None,
            },
            metadata: Vec::new(),
        };
        let result = self
            .host
            .dispatch_in_parse(
                self.transaction,
                DispatchRequest {
                    context: self.context.clone(),
                    target,
                    phase: HookPhase::Matching,
                    payload: HookPayload::Matching(payload),
                },
            )
            .map_err(|error| error.to_string())?;
        merge_effects(&mut self.effects, result.effects);
        self.calls.extend(result.calls);
        self.failures.extend(result.failures);

        let HookPayload::Matching(output) = result.payload else {
            return Err("matching hook returned a different payload kind".to_owned());
        };
        if output.input != self.input
            || output.pattern.as_deref() != event.pattern
            || output.definition_id != event.definition_id
            || output.registration_id != event.registration_id
            || output.pattern_index != event.pattern_index.map(|index| index as u64)
            || !same_matching_path(&output.element_path, &original_element_path)
            || !same_optional_wit_range(
                output.pattern_span.as_ref(),
                original_pattern_span.as_ref(),
            )
            || !same_mapped_span(&output.span, &original_span)
            || output.scope
                != match event.scope {
                    PatternHookScope::Definition => MatchingScope::Definition,
                    PatternHookScope::Registration => MatchingScope::Registration,
                    PatternHookScope::Pattern => MatchingScope::Pattern,
                    PatternHookScope::Element => MatchingScope::Element,
                }
            || output.timing
                != match event.timing {
                    PatternHookTiming::Before => MatchingTiming::Before,
                    PatternHookTiming::After => MatchingTiming::After,
                }
        {
            return Err("matching hook changed immutable matcher identity fields".to_owned());
        }

        let start = usize::try_from(output.input_range.start)
            .map_err(|_| "matching input range start does not fit usize".to_owned())?;
        let end = usize::try_from(output.input_range.end)
            .map_err(|_| "matching input range end does not fit usize".to_owned())?;
        let range = ParserTextRange::new(start, end);
        let changed = output.status != original_status
            || output.input_range.start != original_range.start
            || output.input_range.end != original_range.end;
        let control = match result.decision {
            HookDecision::Reject(rejection) => PatternHookControl::Fail(rejection.reason),
            HookDecision::Handled if output.status == MatchingStatus::Pending => {
                return Err("handled matching hook must return matched or failed status".to_owned());
            }
            HookDecision::Handled => matching_control(output.status, range, output.failure_reason),
            HookDecision::NotApplicable => PatternHookControl::Continue,
            HookDecision::ContinueProcessing if changed => {
                matching_control(output.status, range, output.failure_reason)
            }
            HookDecision::ContinueProcessing => PatternHookControl::Continue,
        };
        self.finish_hook_state(event.scope, event.timing, &event.outcome, &control)?;
        Ok(control)
    }
}

struct WasmExpressionEnvironment<'a> {
    hooks: WasmPatternHooks<'a>,
    pending_leaf: Option<(StateSavepoint, HookEffectsCheckpoint)>,
    pending_registered: Option<(StateSavepoint, HookEffectsCheckpoint)>,
    expression_candidates: Vec<(StateSavepoint, HookEffectsCheckpoint)>,
    semantic_candidates: Vec<(
        StateSavepoint,
        HookEffectsCheckpoint,
        Option<FunctionRegistrySavepoint>,
    )>,
    function_registry: Option<&'a mut FunctionRegistryTransaction>,
}

impl WasmExpressionEnvironment<'_> {
    fn defer_effects(
        &mut self,
        state: &StateSavepoint,
        checkpoint: &HookEffectsCheckpoint,
    ) -> Result<skript_parser::ExpressionEffects, String> {
        let effects = HookEffects {
            diagnostics: self.hooks.effects.diagnostics[checkpoint.effects.diagnostics.len()..]
                .to_vec(),
            context_updates: self.hooks.effects.context_updates
                [checkpoint.effects.context_updates.len()..]
                .to_vec(),
            parse_requests: self.hooks.effects.parse_requests
                [checkpoint.effects.parse_requests.len()..]
                .to_vec(),
            parse_results: self.hooks.effects.parse_results
                [checkpoint.effects.parse_results.len()..]
                .to_vec(),
        };
        let calls = self.hooks.calls[checkpoint.calls.len()..].to_vec();
        let failures = self.hooks.failures[checkpoint.failures.len()..].to_vec();
        let state = self
            .hooks
            .transaction
            .defer_since(state)
            .map_err(|error| error.to_string())?;
        checkpoint.restore(
            &mut self.hooks.effects,
            &mut self.hooks.calls,
            &mut self.hooks.failures,
        );
        Ok(skript_parser::ExpressionEffects::new(
            DeferredExpressionEffects {
                state,
                effects,
                calls,
                failures,
            },
        ))
    }

    fn into_parts(self) -> (HookEffects, Vec<HookCall>, Vec<ComponentFailure>) {
        self.hooks.into_parts()
    }
}

#[derive(Debug, Clone)]
struct DeferredExpressionEffects {
    state: crate::state::StateDelta,
    effects: HookEffects,
    calls: Vec<HookCall>,
    failures: Vec<ComponentFailure>,
}

impl PatternMatchEnvironment for WasmExpressionEnvironment<'_> {
    fn take_pattern_candidate_state(&mut self) -> Option<skript_parser::ExpressionEffects> {
        self.hooks
            .last_candidate
            .take()
            .map(skript_parser::ExpressionEffects::new)
    }

    fn restore_pattern_candidate_state(
        &mut self,
        state: &skript_parser::ExpressionEffects,
    ) -> Result<(), String> {
        let state = state
            .downcast_ref::<SavedPatternCandidate>()
            .ok_or_else(|| "matcher state belongs to a different environment".to_owned())?;
        self.hooks
            .transaction
            .rollback_to(&state.state)
            .map_err(|error| error.to_string())?;
        state.effects.restore(
            &mut self.hooks.effects,
            &mut self.hooks.calls,
            &mut self.hooks.failures,
        );
        Ok(())
    }

    fn begin_pattern_match(&mut self) -> Result<(), String> {
        self.hooks.begin_match_frame()
    }

    fn finish_pattern_match(&mut self, accepted: bool) -> Result<(), String> {
        self.hooks.finish_match_frame(accepted)
    }

    fn checkpoint_pattern_branch(&mut self) -> Result<Option<u64>, String> {
        self.hooks.checkpoint_branch_state()
    }

    fn restore_pattern_branch(&mut self, checkpoint: u64) -> Result<(), String> {
        self.hooks.restore_branch_state(checkpoint)
    }

    fn allows_regex_pattern(
        &mut self,
        kind: MatchSyntaxKind,
        registration_id: &str,
        pattern_index: usize,
    ) -> Result<bool, String> {
        self.hooks
            .allows_regex_pattern(kind, registration_id, pattern_index)
    }

    fn may_override_pattern(
        &self,
        kind: MatchSyntaxKind,
        registration_id: &str,
        pattern_index: usize,
    ) -> bool {
        self.hooks
            .may_override_pattern(kind, registration_id, pattern_index)
    }

    fn resolve_type(
        &mut self,
        _request: TypeExpressionRequest<'_>,
    ) -> Result<TypeExpressionOutcome, String> {
        Ok(TypeExpressionOutcome::default())
    }

    fn dispatch_hook(&mut self, event: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
        self.hooks.dispatch(event)
    }
}

impl ExpressionParseEnvironment for WasmExpressionEnvironment<'_> {
    fn apply_expression_effects(
        &mut self,
        effects: &skript_parser::ExpressionEffects,
    ) -> Result<(), String> {
        let effects = effects
            .downcast_ref::<DeferredExpressionEffects>()
            .ok_or_else(|| "Expression effects belong to a different environment".to_owned())?;
        if self
            .hooks
            .transaction
            .apply_delta(&effects.state)
            .map_err(|error| error.to_string())?
        {
            merge_effects(&mut self.hooks.effects, effects.effects.clone());
            self.hooks.calls.extend(effects.calls.clone());
            self.hooks.failures.extend(effects.failures.clone());
        }
        Ok(())
    }

    fn begin_expression_candidate(&mut self) -> Result<(), String> {
        self.expression_candidates.push((
            self.hooks
                .transaction
                .savepoint()
                .map_err(|error| error.to_string())?,
            HookEffectsCheckpoint::capture(
                &self.hooks.effects,
                &self.hooks.calls,
                &self.hooks.failures,
            ),
        ));
        Ok(())
    }

    fn defer_expression_candidate(
        &mut self,
        accepted: bool,
    ) -> Result<Option<skript_parser::ExpressionEffects>, String> {
        let (state, checkpoint) = self
            .expression_candidates
            .pop()
            .ok_or_else(|| "Expression candidate was not started".to_owned())?;
        if accepted {
            Ok(Some(self.defer_effects(&state, &checkpoint)?))
        } else {
            self.hooks
                .transaction
                .rollback_to(&state)
                .map_err(|error| error.to_string())?;
            checkpoint.restore(
                &mut self.hooks.effects,
                &mut self.hooks.calls,
                &mut self.hooks.failures,
            );
            Ok(None)
        }
    }

    fn begin_semantic_candidate(&mut self) -> Result<(), String> {
        self.semantic_candidates.push((
            self.hooks
                .transaction
                .savepoint()
                .map_err(|error| error.to_string())?,
            HookEffectsCheckpoint::capture(
                &self.hooks.effects,
                &self.hooks.calls,
                &self.hooks.failures,
            ),
            self.function_registry
                .as_deref()
                .map(FunctionRegistryTransaction::savepoint),
        ));
        Ok(())
    }

    fn discard_structure_candidate(
        &mut self,
        candidate: &skript_parser::StructureCandidate,
    ) -> Result<(), String> {
        let Some(registry) = self.function_registry.as_deref_mut() else {
            return Ok(());
        };
        registry
            .remove_declarations_in_span(candidate.matched.matched.span.mapped.virtual_range)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn finish_semantic_candidate(&mut self, accepted: bool) -> Result<(), String> {
        let Some((savepoint, effects, function_savepoint)) = self.semantic_candidates.pop() else {
            return Err("semantic candidate finished without a matching begin".to_owned());
        };
        if !accepted {
            self.hooks
                .transaction
                .rollback_to(&savepoint)
                .map_err(|error| error.to_string())?;
            effects.restore(
                &mut self.hooks.effects,
                &mut self.hooks.calls,
                &mut self.hooks.failures,
            );
            if let (Some(registry), Some(savepoint)) =
                (self.function_registry.as_deref_mut(), function_savepoint)
            {
                registry
                    .rollback(savepoint)
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }

    fn resolve_effect_candidate(
        &mut self,
        request: EffectSemanticRequest<'_>,
    ) -> Result<EffectSemanticDecision, String> {
        self.hooks.resolve_effect_candidate(request)
    }

    fn resolve_condition_candidate(
        &mut self,
        request: ConditionSemanticRequest<'_>,
    ) -> Result<ConditionSemanticDecision, String> {
        self.hooks.resolve_condition_candidate(request)
    }

    fn lookup_functions(
        &mut self,
        request: FunctionLookupRequest<'_>,
    ) -> Result<Vec<skript_parser::FunctionDefinition>, String> {
        Ok(self
            .function_registry
            .as_deref()
            .map_or_else(Vec::new, |registry| {
                registry.lookup_functions(request.name, FunctionScope::Local)
            }))
    }

    fn parse_expression_leaf(
        &mut self,
        request: ExpressionLeafRequest<'_>,
    ) -> Result<ExpressionLeafParse, String> {
        if self.pending_leaf.is_some() {
            return Err("previous Expression leaf set was not finalized".to_owned());
        }
        let leaf_savepoint = self
            .hooks
            .transaction
            .savepoint()
            .map_err(|error| error.to_string())?;
        let effects_checkpoint = HookEffectsCheckpoint::capture(
            &self.hooks.effects,
            &self.hooks.calls,
            &self.hooks.failures,
        );
        let remaining = WitTextRange {
            start: u64::try_from(request.remaining.start)
                .map_err(|_| "Expression range start does not fit u64".to_owned())?,
            end: u64::try_from(request.remaining.end)
                .map_err(|_| "Expression range end does not fit u64".to_owned())?,
        };
        let span = mapped_span_to_wit(request.span.mapped.clone());
        let expected_types = request
            .expected_types
            .iter()
            .map(|expected| WitExpressionExpectedType {
                class_name: expected.class_name.as_str().to_owned(),
                plural: expected.plural,
            })
            .collect::<Vec<_>>();
        let candidate_ends = request
            .candidate_ends
            .iter()
            .map(|end| {
                u64::try_from(*end)
                    .map_err(|_| "Expression candidate end does not fit u64".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let depth = u32::try_from(request.depth)
            .map_err(|_| "Expression recursion depth does not fit u32".to_owned())?;
        let type_options = expression_type_options(
            self.hooks.host.config.syntax_catalog.as_deref(),
            request.input,
            request.remaining,
            request.candidate_ends,
            request.expected_types,
        );
        let literal_options = expression_literal_options(
            self.hooks.host.config.syntax_catalog.as_deref(),
            request.input,
            request.remaining,
            request.candidate_ends,
            request.expected_types,
        );
        let parser_types = if request.allow_literals {
            expression_parser_types(self.hooks.host, request.expected_types, &literal_options)
        } else {
            Vec::new()
        };
        let mut payload = WitExpressionPayload {
            input: request.input.to_owned(),
            context: parse_context_to_wit(request.context),
            active_type: None,
            remaining,
            span: span.clone(),
            expected_types: expected_types.clone(),
            candidate_ends: candidate_ends.clone(),
            allow_literals: request.allow_literals,
            allow_expressions: request.allow_expressions,
            time: request.time,
            depth,
            type_options: type_options.clone(),
            literal_options: literal_options.clone(),
            type_parser_unresolved: Vec::new(),
            type_parser_outcome: None,
            candidates: Vec::new(),
        };
        let expected_payload = payload.clone();
        let mut result = self
            .hooks
            .host
            .dispatch_in_parse(
                self.hooks.transaction,
                DispatchRequest {
                    context: self.hooks.context.clone(),
                    target: DispatchTarget::ParseStage,
                    phase: HookPhase::Expression,
                    payload: HookPayload::Expression(payload),
                },
            )
            .map_err(|error| error.to_string())?;
        let mut available_parse_results = BTreeMap::new();
        let mut leaf_failure = None;
        available_parse_results.append(&mut result.available_parse_results);
        let rejection_diagnostics =
            semantic_rejection_diagnostics(&result.decision, &result.effects)?;
        merge_effects(&mut self.hooks.effects, result.effects);
        self.hooks.calls.extend(result.calls);
        self.hooks.failures.extend(result.failures);
        if let HookDecision::Reject(rejection) = result.decision {
            self.pending_leaf = Some((leaf_savepoint, effects_checkpoint));
            return Ok(ExpressionLeafParse {
                candidates: Vec::new(),
                failure: Some(
                    FailureTrace::leaf(PatternFailure {
                        span: request.span.clone(),
                        reasons: vec![PatternFailureReason::HookRejected {
                            reason: rejection.reason,
                        }],
                    })
                    .with_semantic_diagnostics(rejection_diagnostics),
                ),
            });
        }
        let HookPayload::Expression(output) = result.payload else {
            return Err("Expression hook returned a different payload kind".to_owned());
        };
        if !same_expression_request(&output, &expected_payload) {
            return Err("Expression hook changed immutable request fields".to_owned());
        }
        payload = output;

        // Each Type has its own candidate set. A provider cannot accidentally
        // edit or reject a successful candidate belonging to another Type.
        let stage_effects = self.defer_effects(&leaf_savepoint, &effects_checkpoint)?;
        let mut candidates = std::mem::take(&mut payload.candidates)
            .into_iter()
            .map(|candidate| {
                let mut candidate = wit_expression_candidate(candidate, &available_parse_results)?;
                candidate.effects = Some(stage_effects.clone());
                Ok(candidate)
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut unresolved_type_reasons = Vec::new();
        for active_type in &parser_types {
            payload.active_type = Some(active_type.clone());
            payload.type_parser_unresolved.clear();
            payload.type_parser_outcome = None;
            payload.type_options = if registered_handler_requires_context(
                &self.hooks.host.components,
                RegisteredSyntaxIdentity {
                    kind: CatalogSyntaxKind::Type,
                    definition_id: &active_type.definition_id,
                    registration_id: &active_type.registration_id,
                    pattern_index: None,
                    pattern_source: None,
                    tags: None,
                    mark: None,
                    dynamic_handler: None,
                },
                REGISTERED_CONTEXT_ALL_TYPE_OPTIONS,
            ) {
                all_expression_type_options(self.hooks.host.config.syntax_catalog.as_deref())
            } else {
                type_options.clone()
            };
            let expected_payload = payload.clone();
            let type_savepoint = self
                .hooks
                .transaction
                .savepoint()
                .map_err(|error| error.to_string())?;
            let type_effects_checkpoint = HookEffectsCheckpoint::capture(
                &self.hooks.effects,
                &self.hooks.calls,
                &self.hooks.failures,
            );
            let result = self
                .hooks
                .host
                .dispatch_in_parse(
                    self.hooks.transaction,
                    DispatchRequest {
                        context: self.hooks.context.clone(),
                        target: DispatchTarget::Registration {
                            syntax_kind: SyntaxKind::Type,
                            definition_id: active_type.definition_id.clone(),
                            registration_id: active_type.registration_id.clone(),
                        },
                        phase: HookPhase::Expression,
                        payload: HookPayload::Expression(payload),
                    },
                )
                .map_err(|error| error.to_string())?;
            let rejection_diagnostics =
                semantic_rejection_diagnostics(&result.decision, &result.effects)?;
            merge_effects(&mut self.hooks.effects, result.effects);
            self.hooks.calls.extend(result.calls);
            self.hooks.failures.extend(result.failures);
            if let HookDecision::Reject(rejection) = result.decision {
                self.hooks
                    .transaction
                    .rollback_to(&type_savepoint)
                    .map_err(|error| error.to_string())?;
                type_effects_checkpoint.restore(
                    &mut self.hooks.effects,
                    &mut self.hooks.calls,
                    &mut self.hooks.failures,
                );
                leaf_failure = skript_parser::choose_failure_trace(
                    leaf_failure,
                    Some(
                        FailureTrace::leaf(PatternFailure {
                            span: request.span.clone(),
                            reasons: vec![PatternFailureReason::HookRejected {
                                reason: rejection.reason,
                            }],
                        })
                        .with_semantic_diagnostics(rejection_diagnostics),
                    ),
                );
                payload = expected_payload;
                payload.active_type = None;
                continue;
            }
            let HookPayload::Expression(output) = result.payload else {
                return Err("Type hook returned a different payload kind".to_owned());
            };
            if !same_expression_request(&output, &expected_payload) {
                return Err("Type hook changed immutable Expression request fields".to_owned());
            }
            payload = output;
            if payload.candidates.is_empty() {
                if payload.type_parser_unresolved.is_empty()
                    && payload.type_parser_outcome.is_none()
                {
                    payload.type_parser_unresolved.push(TypeParserUnresolved {
                        reason: "no WASM Type parser is registered for this SSG Type".to_owned(),
                        required_provider: Some(format!(
                            "type-parser/{}",
                            active_type.registration_id
                        )),
                    });
                }
                let unresolved = std::mem::take(&mut payload.type_parser_unresolved);
                self.hooks
                    .transaction
                    .rollback_to(&type_savepoint)
                    .map_err(|error| error.to_string())?;
                type_effects_checkpoint.restore(
                    &mut self.hooks.effects,
                    &mut self.hooks.calls,
                    &mut self.hooks.failures,
                );
                if !unresolved.is_empty() {
                    unresolved_type_reasons.extend(unresolved.into_iter().map(|unresolved| {
                        PatternFailureReason::TypeParserUnresolved {
                            definition_id: active_type.definition_id.clone(),
                            registration_id: active_type.registration_id.clone(),
                            parser_class: active_type.parser_class.clone(),
                            reason: unresolved.reason,
                            required_provider: unresolved.required_provider,
                        }
                    }));
                }
                payload = expected_payload;
            } else {
                let effects = self.defer_effects(&type_savepoint, &type_effects_checkpoint)?;
                for candidate in std::mem::take(&mut payload.candidates) {
                    let mut candidate =
                        wit_expression_candidate(candidate, &result.available_parse_results)?;
                    candidate.effects = Some(effects.clone());
                    candidates.push(candidate);
                }
            }
            payload.active_type = None;
        }
        if !unresolved_type_reasons.is_empty() {
            leaf_failure = skript_parser::choose_failure_trace(
                leaf_failure,
                Some(FailureTrace::leaf(PatternFailure {
                    span: request.span.clone(),
                    reasons: unresolved_type_reasons,
                })),
            );
        }

        self.pending_leaf = Some((leaf_savepoint, effects_checkpoint));
        Ok(ExpressionLeafParse {
            candidates,
            failure: leaf_failure,
        })
    }

    fn finish_expression_leaf(&mut self, _accepted: bool) -> Result<(), String> {
        let Some((savepoint, effects_checkpoint)) = self.pending_leaf.take() else {
            return Err("Expression leaf set was finalized without a pending dispatch".to_owned());
        };
        self.hooks
            .transaction
            .rollback_to(&savepoint)
            .map_err(|error| error.to_string())?;
        effects_checkpoint.restore(
            &mut self.hooks.effects,
            &mut self.hooks.calls,
            &mut self.hooks.failures,
        );
        Ok(())
    }

    fn can_resolve_registered_expression(&self, syntax: RegisteredSyntaxIdentity<'_>) -> bool {
        has_registered_syntax_handler(&self.hooks.host.components, syntax)
    }

    fn registered_capture_bindings(
        &self,
        syntax: RegisteredSyntaxIdentity<'_>,
    ) -> Result<Vec<RegisteredCaptureBinding>, String> {
        registered_capture_bindings(&self.hooks.host.components, syntax)
    }

    fn resolve_registered_expression(
        &mut self,
        request: RegisteredExpressionRequest<'_>,
    ) -> Result<RegisteredExpressionDecision, String> {
        if self.pending_registered.is_some() {
            return Err("previous registered Expression was not finalized".to_owned());
        }
        let savepoint = self
            .hooks
            .transaction
            .savepoint()
            .map_err(|error| error.to_string())?;
        let effects_checkpoint = HookEffectsCheckpoint::capture(
            &self.hooks.effects,
            &self.hooks.calls,
            &self.hooks.failures,
        );
        let expected_types = request
            .expected_types
            .iter()
            .map(|expected| WitExpressionExpectedType {
                class_name: expected.class_name.as_str().to_owned(),
                plural: expected.plural,
            })
            .collect::<Vec<_>>();
        let regex_captures = request
            .captures
            .iter()
            .filter_map(|capture| match capture {
                PatternCapture::Regex { value, .. } => Some(value.clone()),
                PatternCapture::TypeExpression { .. } => None,
            })
            .collect::<Vec<_>>();
        let type_options = if !regex_captures.is_empty()
            && registered_handler_requires_context(
                &self.hooks.host.components,
                RegisteredSyntaxIdentity {
                    kind: CatalogSyntaxKind::Expression,
                    definition_id: request.definition_id,
                    registration_id: request.registration_id,
                    pattern_index: Some(request.pattern_index),
                    pattern_source: Some(request.pattern),
                    tags: Some(request.tags),
                    mark: Some(request.mark),
                    dynamic_handler: request.dynamic_handler,
                },
                REGISTERED_CONTEXT_ALL_TYPE_OPTIONS,
            ) {
            all_expression_type_options(self.hooks.host.config.syntax_catalog.as_deref())
        } else {
            Vec::new()
        };
        let property_options = registered_property_options(
            self.hooks.host.config.syntax_catalog.as_deref(),
            request.related_property,
            request.children,
        );
        let payload = WitRegisteredExpressionPayload {
            input: request.input.to_owned(),
            context: parse_context_to_wit(request.context),
            time: request.time,
            definition_id: request.definition_id.to_owned(),
            registration_id: request.registration_id.to_owned(),
            element_class: request.element_class.as_str().to_owned(),
            related_property: request.related_property.map(str::to_owned),
            pattern_index: u64::try_from(request.pattern_index)
                .map_err(|_| "Expression pattern index does not fit u64".to_owned())?,
            pattern: request.pattern.to_owned(),
            span: mapped_span_to_wit(request.span.mapped.clone()),
            expected_types,
            declared_return_type: request
                .declared_return_type
                .map(|value| value.as_str().to_owned()),
            declared_multiplicity: request.declared_multiplicity.map(multiplicity_to_wit),
            return_type_state: match request.return_type_state {
                ReturnTypeState::Static => WitReturnTypeState::Static,
                ReturnTypeState::Dynamic => WitReturnTypeState::Dynamic,
                ReturnTypeState::Unresolved => WitReturnTypeState::Unresolved,
            },
            possible_return_types: request
                .possible_return_types
                .iter()
                .map(|value| value.as_str().to_owned())
                .collect(),
            possible_return_types_state: match request.possible_return_types_state {
                PossibleReturnTypesState::Complete => WitPossibleReturnTypesState::Complete,
                PossibleReturnTypesState::Partial => WitPossibleReturnTypesState::Partial,
                PossibleReturnTypesState::Unresolved => WitPossibleReturnTypesState::Unresolved,
            },
            regex_captures,
            tags: request
                .tags
                .iter()
                .map(|tag| WitRegisteredExpressionTag {
                    value: tag.value.clone(),
                    implicit: tag.implicit,
                })
                .collect(),
            mark: request.mark,
            children: request
                .children
                .iter()
                .map(|child| {
                    expression_child_to_wit(
                        child,
                        request.input,
                        self.hooks.host.config.syntax_catalog.as_deref(),
                    )
                })
                .collect(),
            parsed_captures: request
                .parsed_captures
                .iter()
                .map(|capture| parsed_capture_to_wit(capture, request.input))
                .collect(),
            common_child_return_type: common_child_return_type(
                request.children,
                self.hooks.host.config.syntax_catalog.as_deref(),
            ),
            type_options,
            property_options,
            selected_property_option_indices: Vec::new(),
            effective_return_type: request
                .declared_return_type
                .map(|value| value.as_str().to_owned()),
            effective_possible_return_types: request
                .possible_return_types
                .iter()
                .map(|value| value.as_str().to_owned())
                .collect(),
            effective_possible_return_types_state: match request.possible_return_types_state {
                PossibleReturnTypesState::Complete => WitPossibleReturnTypesState::Complete,
                PossibleReturnTypesState::Partial => WitPossibleReturnTypesState::Partial,
                PossibleReturnTypesState::Unresolved => WitPossibleReturnTypesState::Unresolved,
            },
            effective_multiplicity: request.declared_multiplicity.map(multiplicity_to_wit),
            public_data: Vec::new(),
            metadata: Vec::new(),
        };
        let original = payload.clone();
        let result = self
            .hooks
            .host
            .dispatch_in_parse(
                self.hooks.transaction,
                DispatchRequest {
                    context: self.hooks.context.clone(),
                    target: DispatchTarget::Pattern {
                        definition_id: request.definition_id.to_owned(),
                        registration_id: request.registration_id.to_owned(),
                        pattern_index: u64::try_from(request.pattern_index)
                            .map_err(|_| "Expression pattern index does not fit u64".to_owned())?,
                        syntax_kind: SyntaxKind::Expression,
                    },
                    phase: HookPhase::Expression,
                    payload: HookPayload::RegisteredExpression(payload),
                },
            )
            .map_err(|error| error.to_string())?;
        let rejection_diagnostics =
            semantic_rejection_diagnostics(&result.decision, &result.effects)?;
        merge_effects(&mut self.hooks.effects, result.effects);
        self.hooks.calls.extend(result.calls);
        self.hooks.failures.extend(result.failures);
        self.pending_registered = Some((savepoint, effects_checkpoint));

        if let HookDecision::Reject(rejection) = result.decision {
            return Ok(RegisteredExpressionDecision::Reject {
                reason: rejection.reason,
                diagnostics: rejection_diagnostics,
            });
        }
        let HookPayload::RegisteredExpression(output) = result.payload else {
            return Err("registered Expression hook returned a different payload kind".to_owned());
        };
        if !same_registered_expression_identity(&output, &original) {
            return Err("registered Expression hook changed immutable request fields".to_owned());
        }
        validate_selected_property_options(&output)?;
        let changed = output.effective_return_type != original.effective_return_type
            || output.effective_possible_return_types != original.effective_possible_return_types
            || output.effective_possible_return_types_state
                != original.effective_possible_return_types_state
            || output.effective_multiplicity != original.effective_multiplicity
            || output.selected_property_option_indices != original.selected_property_option_indices
            || !public_data::same(&output.public_data, &original.public_data)
            || !same_metadata_entries(&output.metadata, &original.metadata);
        if !changed && matches!(result.decision, HookDecision::ContinueProcessing) {
            if !original.regex_captures.is_empty() {
                return Ok(RegisteredExpressionDecision::Reject {
                    reason: "regex Expression requires a WASM semantic handler".to_owned(),
                    diagnostics: Vec::new(),
                });
            }
            return Ok(RegisteredExpressionDecision::UseDeclared);
        }
        Ok(RegisteredExpressionDecision::Resolved {
            return_type: output.effective_return_type.map(ClassName),
            possible_return_types: output
                .effective_possible_return_types
                .into_iter()
                .map(ClassName)
                .collect(),
            possible_return_types_state: match output.effective_possible_return_types_state {
                WitPossibleReturnTypesState::Complete => PossibleReturnTypesState::Complete,
                WitPossibleReturnTypesState::Partial => PossibleReturnTypesState::Partial,
                WitPossibleReturnTypesState::Unresolved => PossibleReturnTypesState::Unresolved,
            },
            multiplicity: output.effective_multiplicity.map(multiplicity_from_wit),
            public_data: public_data::from_wit(output.public_data)?,
            metadata: metadata_entries(output.metadata)?,
        })
    }

    fn finish_registered_expression(&mut self, accepted: bool) -> Result<(), String> {
        let Some((savepoint, effects_checkpoint)) = self.pending_registered.take() else {
            return Err(
                "registered Expression was finalized without a pending dispatch".to_owned(),
            );
        };
        if !accepted {
            self.hooks
                .transaction
                .rollback_to(&savepoint)
                .map_err(|error| error.to_string())?;
            effects_checkpoint.restore(
                &mut self.hooks.effects,
                &mut self.hooks.calls,
                &mut self.hooks.failures,
            );
        }
        Ok(())
    }

    fn enter_section_children(
        &mut self,
        request: SectionChildrenRequest<'_>,
    ) -> Result<SectionChildrenDecision, String> {
        let savepoint = self
            .hooks
            .transaction
            .savepoint()
            .map_err(|error| error.to_string())?;
        let effects_checkpoint = HookEffectsCheckpoint::capture(
            &self.hooks.effects,
            &self.hooks.calls,
            &self.hooks.failures,
        );
        let payload = section_hook_payload(&request, WitSectionTiming::EnterChildren);
        let original = payload.clone();
        let result = self
            .hooks
            .host
            .dispatch_in_parse(
                self.hooks.transaction,
                DispatchRequest {
                    context: self.hooks.context.clone(),
                    target: DispatchTarget::Pattern {
                        definition_id: request.definition_id.to_owned(),
                        registration_id: request.registration_id.to_owned(),
                        pattern_index: u64::try_from(request.pattern_index)
                            .map_err(|_| "Section pattern index does not fit u64".to_owned())?,
                        syntax_kind: SyntaxKind::Section,
                    },
                    phase: HookPhase::Section,
                    payload: HookPayload::Section(payload),
                },
            )
            .map_err(|error| error.to_string())?;
        let rejection_diagnostics =
            semantic_rejection_diagnostics(&result.decision, &result.effects)?;
        let updates = result.effects.context_updates.clone();
        merge_effects(&mut self.hooks.effects, result.effects);
        self.hooks.calls.extend(result.calls);
        self.hooks.failures.extend(result.failures);
        let HookPayload::Section(output) = result.payload else {
            return Err("Section hook returned a different payload kind".to_owned());
        };
        if !same_section_payload(&output, &original) {
            return Err("Section hook changed immutable candidate fields".to_owned());
        }
        if let HookDecision::Reject(rejection) = result.decision {
            self.hooks
                .transaction
                .rollback_to(&savepoint)
                .map_err(|error| error.to_string())?;
            effects_checkpoint.restore(
                &mut self.hooks.effects,
                &mut self.hooks.calls,
                &mut self.hooks.failures,
            );
            return Ok(SectionChildrenDecision::Reject {
                reason: rejection.reason,
                diagnostics: rejection_diagnostics,
            });
        }
        let context = apply_context_updates(request.context, updates, "Section")?;
        Ok(SectionChildrenDecision::Accept {
            context,
            body_mode: section_body_mode_from_wit(output.candidate.body_mode),
            metadata: metadata_entries(output.candidate.metadata)?,
        })
    }

    fn exit_section_children(
        &mut self,
        request: SectionChildrenRequest<'_>,
    ) -> Result<SectionExitDecision, String> {
        let savepoint = self
            .hooks
            .transaction
            .savepoint()
            .map_err(|error| error.to_string())?;
        let effects_checkpoint = HookEffectsCheckpoint::capture(
            &self.hooks.effects,
            &self.hooks.calls,
            &self.hooks.failures,
        );
        let payload = section_hook_payload(&request, WitSectionTiming::ExitChildren);
        let original = payload.clone();
        let result = self
            .hooks
            .host
            .dispatch_in_parse(
                self.hooks.transaction,
                DispatchRequest {
                    context: self.hooks.context.clone(),
                    target: DispatchTarget::Pattern {
                        definition_id: request.definition_id.to_owned(),
                        registration_id: request.registration_id.to_owned(),
                        pattern_index: u64::try_from(request.pattern_index)
                            .map_err(|_| "Section pattern index does not fit u64".to_owned())?,
                        syntax_kind: SyntaxKind::Section,
                    },
                    phase: HookPhase::Section,
                    payload: HookPayload::Section(payload),
                },
            )
            .map_err(|error| error.to_string())?;
        let rejection_diagnostics =
            semantic_rejection_diagnostics(&result.decision, &result.effects)?;
        let updates = result.effects.context_updates.clone();
        merge_effects(&mut self.hooks.effects, result.effects);
        self.hooks.calls.extend(result.calls);
        self.hooks.failures.extend(result.failures);
        let HookPayload::Section(output) = result.payload else {
            return Err("Section hook returned a different payload kind".to_owned());
        };
        if !same_section_payload(&output, &original) {
            return Err("Section hook changed immutable candidate fields".to_owned());
        }
        if let HookDecision::Reject(rejection) = result.decision {
            self.hooks
                .transaction
                .rollback_to(&savepoint)
                .map_err(|error| error.to_string())?;
            effects_checkpoint.restore(
                &mut self.hooks.effects,
                &mut self.hooks.calls,
                &mut self.hooks.failures,
            );
            return Ok(SectionExitDecision::Reject {
                reason: rejection.reason,
                diagnostics: rejection_diagnostics,
            });
        }
        Ok(SectionExitDecision::Accept {
            context: apply_context_updates(request.context, updates, "Section exit")?,
            metadata: metadata_entries(output.candidate.metadata)?,
        })
    }

    fn enter_structure(
        &mut self,
        request: StructureHookRequest<'_>,
    ) -> Result<StructureHookDecision, String> {
        let payload =
            structure_hook_payload(&request, self.hooks.host.config.syntax_catalog.as_deref())?;
        let mut expected = payload.clone();
        let mut context = self.hooks.context.clone();
        context.syntax_context = request.context.syntax_context;
        let result = self
            .hooks
            .host
            .dispatch_in_parse(
                self.hooks.transaction,
                DispatchRequest {
                    context,
                    target: DispatchTarget::Pattern {
                        definition_id: request.candidate.matched.definition_id.clone(),
                        registration_id: request.candidate.matched.registration_id.clone(),
                        pattern_index: u64::try_from(request.candidate.matched.pattern_index)
                            .map_err(|_| "Structure pattern index does not fit u64".to_owned())?,
                        syntax_kind: SyntaxKind::Structure,
                    },
                    phase: HookPhase::Structure,
                    payload: HookPayload::Structure(payload),
                },
            )
            .map_err(|error| error.to_string())?;
        let rejection_diagnostics =
            semantic_rejection_diagnostics(&result.decision, &result.effects)?;
        let updates = result.effects.context_updates.clone();
        merge_effects(&mut self.hooks.effects, result.effects);
        self.hooks.calls.extend(result.calls);
        self.hooks.failures.extend(result.failures);
        let HookPayload::Structure(output) = result.payload else {
            return Err("Structure hook returned a different payload kind".to_owned());
        };
        apply_wit_structure_context_updates(&mut expected.context, &updates)?;
        if !same_structure_payload_identity(&output, &expected) {
            return Err("Structure hook changed immutable candidate fields".to_owned());
        }
        if let HookDecision::Reject(rejection) = result.decision {
            return Ok(StructureHookDecision::Reject {
                reason: rejection.reason,
                diagnostics: rejection_diagnostics,
            });
        }
        if !output.candidate.declarations.is_empty() {
            let Some(registry) = self.function_registry.as_deref_mut() else {
                return Ok(StructureHookDecision::Reject {
                    reason: "Structure declarations require a document registry".to_owned(),
                    diagnostics: Vec::new(),
                });
            };
            for declaration in output.candidate.declarations {
                let declaration = match declaration {
                    WitParserDeclaration::DocumentFunction(declaration) => {
                        function_declaration_from_wit(declaration)?
                    }
                };
                if let Err(error) = registry.register(declaration) {
                    return Ok(StructureHookDecision::Reject {
                        reason: error.to_string(),
                        diagnostics: Vec::new(),
                    });
                }
            }
        }
        let body_mode = structure_body_mode_from_wit(output.candidate.body_mode);
        if request.candidate.actual_node_type == syntaxes::NodeType::Simple
            && body_mode != StructureBodyMode::None
        {
            return Err("Simple Structure cannot select a body parser".to_owned());
        }
        Ok(StructureHookDecision::Accept {
            context: apply_context_updates(request.context, updates, "Structure")?,
            body_mode,
            metadata: metadata_entries(output.candidate.metadata)?,
        })
    }

    fn exit_structure(
        &mut self,
        request: StructureHookRequest<'_>,
    ) -> Result<StructureExitDecision, String> {
        let savepoint = self
            .hooks
            .transaction
            .savepoint()
            .map_err(|error| error.to_string())?;
        let effects_checkpoint = HookEffectsCheckpoint::capture(
            &self.hooks.effects,
            &self.hooks.calls,
            &self.hooks.failures,
        );
        let payload =
            structure_hook_payload(&request, self.hooks.host.config.syntax_catalog.as_deref())?;
        let original = payload.clone();
        let mut context = self.hooks.context.clone();
        context.syntax_context = request.context.syntax_context;
        let result = self
            .hooks
            .host
            .dispatch_in_parse(
                self.hooks.transaction,
                DispatchRequest {
                    context,
                    target: DispatchTarget::Pattern {
                        definition_id: request.candidate.matched.definition_id.clone(),
                        registration_id: request.candidate.matched.registration_id.clone(),
                        pattern_index: u64::try_from(request.candidate.matched.pattern_index)
                            .map_err(|_| "Structure pattern index does not fit u64".to_owned())?,
                        syntax_kind: SyntaxKind::Structure,
                    },
                    phase: HookPhase::Structure,
                    payload: HookPayload::Structure(payload),
                },
            )
            .map_err(|error| error.to_string())?;
        let rejection_diagnostics =
            semantic_rejection_diagnostics(&result.decision, &result.effects)?;
        merge_effects(&mut self.hooks.effects, result.effects);
        self.hooks.calls.extend(result.calls);
        self.hooks.failures.extend(result.failures);
        let HookPayload::Structure(output) = result.payload else {
            return Err("Structure hook returned a different payload kind".to_owned());
        };
        if !same_structure_payload_identity(&output, &original)
            || output.candidate.body_mode != original.candidate.body_mode
            || !output.candidate.declarations.is_empty()
        {
            return Err("Structure exit hook changed immutable candidate fields".to_owned());
        }
        if let HookDecision::Reject(rejection) = result.decision {
            self.hooks
                .transaction
                .rollback_to(&savepoint)
                .map_err(|error| error.to_string())?;
            effects_checkpoint.restore(
                &mut self.hooks.effects,
                &mut self.hooks.calls,
                &mut self.hooks.failures,
            );
            return Ok(StructureExitDecision::Reject {
                reason: rejection.reason,
                diagnostics: rejection_diagnostics,
            });
        }
        Ok(StructureExitDecision::Accept)
    }

    fn state_revision(&self) -> Result<u64, String> {
        self.hooks
            .transaction
            .state_revision()
            .map_err(|error| error.to_string())
    }
}

fn apply_context_updates(
    original: &ExpressionParseContext,
    updates: Vec<ContextUpdate>,
    owner: &str,
) -> Result<ExpressionParseContext, String> {
    apply_context_update_slice(original, &updates, owner)
}

/// Applies addon-produced parser context updates to a reusable parse context.
///
/// Consumers that select a Structure without parsing its body can use this to
/// carry the accepted Structure's event classes and addon-owned context values
/// into later Effect, Condition, and Expression parses.
pub fn apply_parser_context_updates(
    original: &ExpressionParseContext,
    updates: &[ContextUpdate],
) -> Result<ExpressionParseContext, String> {
    apply_context_update_slice(original, updates, "Parser")
}

fn apply_context_update_slice(
    original: &ExpressionParseContext,
    updates: &[ContextUpdate],
    owner: &str,
) -> Result<ExpressionParseContext, String> {
    let mut context = original.clone();
    for update in updates {
        if update.syntax_context != context.syntax_context {
            continue;
        }
        if update.key == "parser.event-classes" {
            context.event_classes = update
                .value
                .as_deref()
                .map(|value| {
                    std::str::from_utf8(value)
                        .map_err(|_| format!("{owner} Event classes are not UTF-8"))
                        .map(|value| {
                            value
                                .split(';')
                                .filter(|value| !value.is_empty())
                                .map(|value| ClassName(value.to_owned()))
                                .collect()
                        })
                })
                .transpose()?
                .unwrap_or_default();
            continue;
        }
        if let Some(value) = &update.value {
            context.values.insert(
                update.key.clone(),
                String::from_utf8(value.clone())
                    .map_err(|_| format!("{owner} context update is not UTF-8"))?,
            );
        } else {
            context.values.remove(&update.key);
        }
    }
    Ok(context)
}

fn structure_hook_payload(
    request: &StructureHookRequest<'_>,
    catalog: Option<&Catalog>,
) -> Result<WitStructurePayload, String> {
    let candidate = request.candidate;
    let mut entries = Vec::new();
    if let StructureBody::Entries(values) = &candidate.body {
        flatten_structure_entries(values, None, &mut entries);
    }
    Ok(WitStructurePayload {
        input: request.input.to_owned(),
        body_tree: parser_raw_subtree_to_wit(request.tree, candidate.raw_node_id),
        context: parse_context_to_wit(request.context),
        timing: match request.timing {
            StructureHookTiming::EnterBody => WitStructureTiming::EnterBody,
            StructureHookTiming::ExitBody => WitStructureTiming::ExitBody,
        },
        type_options: all_expression_type_options(catalog),
        candidate: WitStructureCandidate {
            raw_node_id: candidate.raw_node_id.get(),
            definition_id: candidate.matched.definition_id.clone(),
            registration_id: candidate.matched.registration_id.clone(),
            element_class: candidate
                .element_class
                .as_ref()
                .map(|value| value.as_str().to_owned()),
            priority: candidate.matched.priority,
            registration_order: u64::try_from(candidate.matched.registration_order)
                .map_err(|_| "Structure registration order does not fit u64".to_owned())?,
            pattern_index: u64::try_from(candidate.matched.pattern_index)
                .map_err(|_| "Structure pattern index does not fit u64".to_owned())?,
            pattern: candidate.matched.pattern.clone(),
            span: mapped_span_to_wit(candidate.matched.matched.span.mapped.clone()),
            declared_node_type: structure_node_type_to_wit(candidate.declared_node_type),
            actual_node_type: structure_node_type_to_wit(candidate.actual_node_type),
            regex_captures: candidate
                .matched
                .matched
                .captures
                .iter()
                .filter_map(|capture| match capture {
                    PatternCapture::Regex { value, .. } => Some(value.clone()),
                    PatternCapture::TypeExpression { .. } => None,
                })
                .collect(),
            tags: candidate
                .matched
                .matched
                .tags
                .iter()
                .map(syntax_tag_to_wit)
                .collect(),
            mark: candidate.matched.matched.mark,
            marks: candidate
                .matched
                .matched
                .marks
                .iter()
                .map(syntax_mark_to_wit)
                .collect(),
            parsed_captures: candidate
                .parsed_captures
                .iter()
                .map(|capture| parsed_capture_to_wit(capture, request.input))
                .collect(),
            body_mode: structure_body_mode_to_wit(request.default_body_mode),
            child_node_ids: structure_child_node_ids(&candidate.body),
            entries,
            handler: candidate.handler.clone(),
            metadata: metadata_to_wit(&candidate.metadata),
            declarations: Vec::new(),
        },
    })
}

fn flatten_structure_entries(
    values: &[StructureEntry],
    parent: Option<u64>,
    output: &mut Vec<WitStructureEntry>,
) {
    for value in values {
        let index = output.len() as u64;
        output.push(structure_entry_to_wit(value, parent));
        if let StructureEntryValue::Container(children) = &value.value {
            flatten_structure_entries(children, Some(index), output);
        }
    }
}

fn structure_entry_to_wit(value: &StructureEntry, parent: Option<u64>) -> WitStructureEntry {
    let (value_kind, value_summary) = match &value.value {
        StructureEntryValue::Raw(_) => (WitStructureEntryValueKind::Raw, None),
        StructureEntryValue::Expression(node) => {
            let node = unwrap_grouped_expression(node);
            (
                WitStructureEntryValueKind::Expression,
                Some(WitParseSummary {
                    kind: "expression".to_owned(),
                    definition_id: match &node.kind {
                        ExpressionNodeKind::Registered { definition_id, .. } => {
                            Some(definition_id.clone())
                        }
                        _ => None,
                    },
                    registration_id: match &node.kind {
                        ExpressionNodeKind::Registered {
                            registration_id, ..
                        } => Some(registration_id.clone()),
                        _ => None,
                    },
                    element_class: None,
                    pattern_index: match &node.kind {
                        ExpressionNodeKind::Registered { pattern_index, .. } => {
                            Some(*pattern_index as u64)
                        }
                        _ => None,
                    },
                    return_type: node
                        .return_type
                        .as_ref()
                        .map(|value| value.as_str().to_owned()),
                    possible_return_types: node
                        .possible_return_types
                        .iter()
                        .map(|value| value.as_str().to_owned())
                        .collect(),
                    possible_return_types_state: match node.possible_return_types_state {
                        PossibleReturnTypesState::Complete => WitPossibleReturnTypesState::Complete,
                        PossibleReturnTypesState::Partial => WitPossibleReturnTypesState::Partial,
                        PossibleReturnTypesState::Unresolved => {
                            WitPossibleReturnTypesState::Unresolved
                        }
                    },
                    multiplicity: node.multiplicity.map(multiplicity_to_wit),
                    public_data: public_data::to_wit(&node.public_data),
                    metadata: metadata_to_wit(&node.metadata),
                }),
            )
        }
        StructureEntryValue::Trigger(_) => (WitStructureEntryValueKind::Trigger, None),
        StructureEntryValue::Container(_) => (WitStructureEntryValueKind::Container, None),
        StructureEntryValue::Section(_) => (WitStructureEntryValueKind::Section, None),
        StructureEntryValue::Unknown(_) => (WitStructureEntryValueKind::Unknown, None),
    };
    WitStructureEntry {
        raw_node_id: value.raw_node_id.map(|id| id.get()),
        parent_entry: parent,
        key: value.key.clone(),
        entry_data_class: value.entry_data_class.as_str().to_owned(),
        kind: match value.kind {
            syntaxes::EntryKind::Literal => WitStructureEntryKind::Literal,
            syntaxes::EntryKind::VariableString => WitStructureEntryKind::VariableString,
            syntaxes::EntryKind::Expression => WitStructureEntryKind::Expression,
            syntaxes::EntryKind::Trigger => WitStructureEntryKind::Trigger,
            syntaxes::EntryKind::Container => WitStructureEntryKind::Container,
            syntaxes::EntryKind::Section => WitStructureEntryKind::Section,
            syntaxes::EntryKind::KeyValue => WitStructureEntryKind::KeyValue,
            syntaxes::EntryKind::Unknown => WitStructureEntryKind::Unknown,
        },
        source: value.source.clone(),
        span: mapped_span_to_wit(value.span.mapped.clone()),
        defaulted: value.defaulted,
        value_kind,
        value_summary,
    }
}

fn unwrap_grouped_expression(mut node: &ExpressionNode) -> &ExpressionNode {
    while matches!(&node.kind, ExpressionNodeKind::Grouped) {
        let Some(child) = node.children.first() else {
            break;
        };
        node = child;
    }
    node
}

fn structure_child_node_ids(body: &StructureBody) -> Vec<u64> {
    match body {
        StructureBody::None => Vec::new(),
        StructureBody::Raw(ids) => ids.iter().map(|id| id.get()).collect(),
        StructureBody::Entries(entries) => entries
            .iter()
            .filter_map(|entry| entry.raw_node_id.map(|id| id.get()))
            .collect(),
        StructureBody::Trigger(nodes) => nodes
            .iter()
            .filter_map(|node| match node {
                skript_parser::SectionBodyNode::Section(value) => {
                    value.selected.as_ref().map_or_else(
                        || value.unknown.as_ref().map(|value| value.raw_node_id),
                        |value| Some(value.raw_node_id),
                    )
                }
                skript_parser::SectionBodyNode::Effect(value) => {
                    value.selected.as_ref().map_or_else(
                        || value.unknown.as_ref().map(|value| value.raw_node_id),
                        |value| Some(value.raw_node_id),
                    )
                }
                skript_parser::SectionBodyNode::Condition { raw_node_id, .. } => Some(*raw_node_id),
                skript_parser::SectionBodyNode::Trivia(id)
                | skript_parser::SectionBodyNode::Unclaimed(id) => Some(*id),
            })
            .map(|id| id.get())
            .collect(),
    }
}

fn structure_node_type_to_wit(value: syntaxes::NodeType) -> WitStructureNodeType {
    match value {
        syntaxes::NodeType::Simple => WitStructureNodeType::Simple,
        syntaxes::NodeType::Section => WitStructureNodeType::Section,
        syntaxes::NodeType::Both => WitStructureNodeType::Both,
    }
}

fn structure_body_mode_to_wit(value: StructureBodyMode) -> WitStructureBodyMode {
    match value {
        StructureBodyMode::None => WitStructureBodyMode::None,
        StructureBodyMode::Raw => WitStructureBodyMode::Raw,
        StructureBodyMode::Entries => WitStructureBodyMode::Entries,
        StructureBodyMode::Trigger => WitStructureBodyMode::Trigger,
    }
}

fn structure_body_mode_from_wit(value: WitStructureBodyMode) -> StructureBodyMode {
    match value {
        WitStructureBodyMode::None => StructureBodyMode::None,
        WitStructureBodyMode::Raw => StructureBodyMode::Raw,
        WitStructureBodyMode::Entries => StructureBodyMode::Entries,
        WitStructureBodyMode::Trigger => StructureBodyMode::Trigger,
    }
}

fn function_declaration_from_wit(
    declaration: WitFunctionDeclaration,
) -> Result<FunctionDeclaration, String> {
    let start = usize::try_from(declaration.span.start)
        .map_err(|_| "Function declaration span start does not fit usize".to_owned())?;
    let end = usize::try_from(declaration.span.end)
        .map_err(|_| "Function declaration span end does not fit usize".to_owned())?;
    let parameters = declaration
        .parameters
        .into_iter()
        .map(|parameter| FunctionParameterDeclaration {
            name: parameter.name,
            parameter_type: ClassName(parameter.class_name),
            single: parameter.single,
            default_source: parameter.default_source,
        })
        .collect();
    let return_contract = FunctionReturnContract {
        return_type: declaration.returns.class_name.map(ClassName),
        single: declaration.returns.single,
    };
    Ok(FunctionDeclaration {
        source: declaration.source,
        span: ParserTextRange::new(start, end),
        scope: match declaration.scope {
            WitFunctionDeclarationScope::Global => FunctionScope::Global,
            WitFunctionDeclarationScope::Local => FunctionScope::Local,
        },
        name: declaration.name,
        parameters,
        return_contract,
        metadata: metadata_entries(declaration.metadata)?,
    })
}

fn same_structure_payload_identity(
    left: &WitStructurePayload,
    right: &WitStructurePayload,
) -> bool {
    left.input == right.input
        && same_raw_tree(&left.body_tree, &right.body_tree)
        && same_parse_context(&left.context, &right.context)
        && left.timing == right.timing
        && same_expression_type_options(&left.type_options, &right.type_options)
        && left.candidate.raw_node_id == right.candidate.raw_node_id
        && left.candidate.definition_id == right.candidate.definition_id
        && left.candidate.registration_id == right.candidate.registration_id
        && left.candidate.element_class == right.candidate.element_class
        && left.candidate.priority == right.candidate.priority
        && left.candidate.registration_order == right.candidate.registration_order
        && left.candidate.pattern_index == right.candidate.pattern_index
        && left.candidate.pattern == right.candidate.pattern
        && same_mapped_span(&left.candidate.span, &right.candidate.span)
        && left.candidate.declared_node_type == right.candidate.declared_node_type
        && left.candidate.actual_node_type == right.candidate.actual_node_type
        && left.candidate.regex_captures == right.candidate.regex_captures
        && same_syntax_tags(&left.candidate.tags, &right.candidate.tags)
        && left.candidate.mark == right.candidate.mark
        && same_syntax_marks(&left.candidate.marks, &right.candidate.marks)
        && same_parsed_captures(
            &left.candidate.parsed_captures,
            &right.candidate.parsed_captures,
        )
        && left.candidate.child_node_ids == right.candidate.child_node_ids
        && same_structure_entries(&left.candidate.entries, &right.candidate.entries)
        && left.candidate.handler == right.candidate.handler
}

fn parse_context_to_wit(context: &ExpressionParseContext) -> WitParseContext {
    WitParseContext {
        syntax_context: context.syntax_context,
        event_classes: context
            .event_classes
            .iter()
            .map(|class| class.as_str().to_owned())
            .collect(),
        values: context
            .values
            .iter()
            .map(|(key, value)| WitParseContextValue {
                key: key.clone(),
                value: value.clone(),
            })
            .collect(),
    }
}

fn same_parse_context(left: &WitParseContext, right: &WitParseContext) -> bool {
    left.syntax_context == right.syntax_context
        && left.event_classes == right.event_classes
        && left.values.len() == right.values.len()
        && left
            .values
            .iter()
            .zip(&right.values)
            .all(|(left, right)| left.key == right.key && left.value == right.value)
}

fn same_raw_tree(left: &RawTree, right: &RawTree) -> bool {
    left.roots == right.roots
        && same_raw_tree_nodes(&left.nodes, &right.nodes)
        && same_raw_diagnostics(&left.diagnostics, &right.diagnostics)
        && same_indentation(left.indentation.as_ref(), right.indentation.as_ref())
}

fn same_raw_tree_nodes(left: &[WitRawTreeNode], right: &[WitRawTreeNode]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.id == right.id
                && left.kind == right.kind
                && left.text == right.text
                && same_mapped_span(&left.span, &right.span)
                && same_raw_line(&left.line, &right.line)
                && same_optional_mapped_span(left.code_span.as_ref(), right.code_span.as_ref())
                && same_optional_mapped_span(left.header_span.as_ref(), right.header_span.as_ref())
                && same_optional_mapped_span(left.body_span.as_ref(), right.body_span.as_ref())
                && left.indent_level == right.indent_level
                && same_raw_invalid_reason(
                    left.invalid_reason.as_ref(),
                    right.invalid_reason.as_ref(),
                )
                && left.syntax_context == right.syntax_context
                && left.parent == right.parent
                && left.children == right.children
        })
}

fn same_raw_line(left: &WitRawLine, right: &WitRawLine) -> bool {
    left.number == right.number
        && left.raw_text == right.raw_text
        && left.line_ending == right.line_ending
        && same_mapped_span(&left.span, &right.span)
        && same_mapped_span(&left.content_span, &right.content_span)
        && same_mapped_span(&left.line_ending_span, &right.line_ending_span)
        && same_raw_trivia(&left.indentation, &right.indentation)
        && same_raw_trivia_list(&left.trailing_trivia, &right.trailing_trivia)
}

fn same_raw_trivia_list(left: &[WitRawTrivia], right: &[WitRawTrivia]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| same_raw_trivia(left, right))
}

fn same_raw_trivia(left: &WitRawTrivia, right: &WitRawTrivia) -> bool {
    left.kind == right.kind && left.text == right.text && same_mapped_span(&left.span, &right.span)
}

fn same_optional_mapped_span(left: Option<&MappedSpan>, right: Option<&MappedSpan>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => same_mapped_span(left, right),
        (None, None) => true,
        _ => false,
    }
}

fn same_raw_invalid_reason(
    left: Option<&WitRawInvalidReason>,
    right: Option<&WitRawInvalidReason>,
) -> bool {
    match (left, right) {
        (
            Some(WitRawInvalidReason::MixedIndentation),
            Some(WitRawInvalidReason::MixedIndentation),
        )
        | (
            Some(WitRawInvalidReason::InvalidIndentation),
            Some(WitRawInvalidReason::InvalidIndentation),
        )
        | (None, None) => true,
        (
            Some(WitRawInvalidReason::UnexpectedIndentation(left)),
            Some(WitRawInvalidReason::UnexpectedIndentation(right)),
        ) => left.expected_level == right.expected_level && left.actual_level == right.actual_level,
        _ => false,
    }
}

fn same_raw_diagnostics(left: &[WitRawDiagnostic], right: &[WitRawDiagnostic]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.code == right.code
                && left.severity == right.severity
                && left.message == right.message
                && same_mapped_span(&left.span, &right.span)
                && same_raw_related(&left.related, &right.related)
        })
}

fn same_raw_related(left: &[WitRawRelatedSpan], right: &[WitRawRelatedSpan]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.message == right.message && same_mapped_span(&left.span, &right.span)
        })
}

fn same_indentation(left: Option<&WitIndentation>, right: Option<&WitIndentation>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left.kind == right.kind && left.unit == right.unit,
        (None, None) => true,
        _ => false,
    }
}

fn same_structure_entries(left: &[WitStructureEntry], right: &[WitStructureEntry]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.raw_node_id == right.raw_node_id
                && left.parent_entry == right.parent_entry
                && left.key == right.key
                && left.entry_data_class == right.entry_data_class
                && left.kind == right.kind
                && left.source == right.source
                && same_mapped_span(&left.span, &right.span)
                && left.defaulted == right.defaulted
                && left.value_kind == right.value_kind
                && same_parse_summary(left.value_summary.as_ref(), right.value_summary.as_ref())
        })
}

fn section_hook_payload(
    request: &SectionChildrenRequest<'_>,
    timing: WitSectionTiming,
) -> WitSectionPayload {
    WitSectionPayload {
        input: request.input.to_owned(),
        context: parse_context_to_wit(request.context),
        raw_node_id: request.raw_node_id.get(),
        span: mapped_span_to_wit(request.span.mapped.clone()),
        timing,
        preceding_siblings: request
            .preceding_siblings
            .iter()
            .map(section_sibling_to_wit)
            .collect(),
        next_sibling: request.next_sibling.map(section_raw_node_to_wit),
        raw_children: request
            .raw_children
            .iter()
            .map(section_raw_node_to_wit)
            .collect(),
        candidate: WitSectionCandidate {
            raw_node_id: request.raw_node_id.get(),
            definition_id: request.definition_id.to_owned(),
            registration_id: request.registration_id.to_owned(),
            element_class: request.element_class.map(|class| class.as_str().to_owned()),
            pattern_index: u64::try_from(request.pattern_index).unwrap_or(u64::MAX),
            span: mapped_span_to_wit(request.span.mapped.clone()),
            loop_section: request.loop_section,
            effect_section: request.effect_section,
            section_expression: request.section_expression,
            regex_captures: request
                .captures
                .iter()
                .filter_map(|capture| match capture {
                    PatternCapture::Regex { value, .. } => Some(value.clone()),
                    PatternCapture::TypeExpression { .. } => None,
                })
                .collect(),
            tags: request.tags.iter().map(syntax_tag_to_wit).collect(),
            mark: request.mark,
            marks: request.marks.iter().map(syntax_mark_to_wit).collect(),
            parsed_captures: request
                .parsed_captures
                .iter()
                .map(|capture| parsed_capture_to_wit(capture, request.input))
                .collect(),
            body_mode: section_body_mode_to_wit(request.body_mode),
            metadata: request
                .metadata
                .iter()
                .map(|(key, value)| WitMetadataEntry {
                    key: key.clone(),
                    value: value.clone(),
                    owner_component_id: None,
                })
                .collect(),
        },
    }
}

fn same_section_payload(left: &WitSectionPayload, right: &WitSectionPayload) -> bool {
    left.input == right.input
        && same_parse_context(&left.context, &right.context)
        && left.raw_node_id == right.raw_node_id
        && same_mapped_span(&left.span, &right.span)
        && left.timing == right.timing
        && same_section_siblings(&left.preceding_siblings, &right.preceding_siblings)
        && same_optional_section_raw_node(left.next_sibling.as_ref(), right.next_sibling.as_ref())
        && same_section_raw_nodes(&left.raw_children, &right.raw_children)
        && left.candidate.raw_node_id == right.candidate.raw_node_id
        && left.candidate.definition_id == right.candidate.definition_id
        && left.candidate.registration_id == right.candidate.registration_id
        && left.candidate.element_class == right.candidate.element_class
        && left.candidate.pattern_index == right.candidate.pattern_index
        && same_mapped_span(&left.candidate.span, &right.candidate.span)
        && left.candidate.loop_section == right.candidate.loop_section
        && left.candidate.effect_section == right.candidate.effect_section
        && left.candidate.section_expression == right.candidate.section_expression
        && left.candidate.regex_captures == right.candidate.regex_captures
        && same_syntax_tags(&left.candidate.tags, &right.candidate.tags)
        && left.candidate.mark == right.candidate.mark
        && same_syntax_marks(&left.candidate.marks, &right.candidate.marks)
        && same_parsed_captures(
            &left.candidate.parsed_captures,
            &right.candidate.parsed_captures,
        )
}

fn section_body_mode_to_wit(value: ParserSectionBodyMode) -> WitSectionBodyMode {
    match value {
        ParserSectionBodyMode::Trigger => WitSectionBodyMode::Trigger,
        ParserSectionBodyMode::Conditions => WitSectionBodyMode::Conditions,
    }
}

fn section_body_mode_from_wit(value: WitSectionBodyMode) -> ParserSectionBodyMode {
    match value {
        WitSectionBodyMode::Trigger => ParserSectionBodyMode::Trigger,
        WitSectionBodyMode::Conditions => ParserSectionBodyMode::Conditions,
    }
}

fn section_raw_node_to_wit(value: &ParserSectionRawNodeSummary) -> WitSectionRawNode {
    WitSectionRawNode {
        raw_node_id: value.raw_node_id.get(),
        kind: match value.kind {
            ParserRawNodeKind::Blank => WitSectionRawNodeKind::Blank,
            ParserRawNodeKind::Comment => WitSectionRawNodeKind::Comment,
            ParserRawNodeKind::Simple => WitSectionRawNodeKind::Simple,
            ParserRawNodeKind::Section => WitSectionRawNodeKind::Section,
            ParserRawNodeKind::Invalid => WitSectionRawNodeKind::Invalid,
        },
        source: value.source.clone(),
        span: mapped_span_to_wit(value.span.mapped.clone()),
    }
}

fn section_sibling_to_wit(value: &ParserSectionSiblingSummary) -> WitSectionSibling {
    WitSectionSibling {
        raw_node_id: value.raw_node_id.get(),
        definition_id: value.definition_id.clone(),
        registration_id: value.registration_id.clone(),
        element_class: value
            .element_class
            .as_ref()
            .map(|class| class.as_str().to_owned()),
        pattern_index: u64::try_from(value.pattern_index).unwrap_or(u64::MAX),
        source: value.source.clone(),
        span: mapped_span_to_wit(value.span.mapped.clone()),
        handler: value.handler.clone(),
        metadata: value
            .metadata
            .iter()
            .map(|(key, value)| WitMetadataEntry {
                key: key.clone(),
                value: value.clone(),
                owner_component_id: None,
            })
            .collect(),
    }
}

fn same_section_raw_node(left: &WitSectionRawNode, right: &WitSectionRawNode) -> bool {
    left.raw_node_id == right.raw_node_id
        && left.kind == right.kind
        && left.source == right.source
        && same_mapped_span(&left.span, &right.span)
}

fn same_section_raw_nodes(left: &[WitSectionRawNode], right: &[WitSectionRawNode]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| same_section_raw_node(left, right))
}

fn same_optional_section_raw_node(
    left: Option<&WitSectionRawNode>,
    right: Option<&WitSectionRawNode>,
) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => same_section_raw_node(left, right),
        (None, None) => true,
        _ => false,
    }
}

fn same_section_siblings(left: &[WitSectionSibling], right: &[WitSectionSibling]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.raw_node_id == right.raw_node_id
                && left.definition_id == right.definition_id
                && left.registration_id == right.registration_id
                && left.element_class == right.element_class
                && left.pattern_index == right.pattern_index
                && left.source == right.source
                && same_mapped_span(&left.span, &right.span)
                && left.handler == right.handler
                && same_metadata_entries(&left.metadata, &right.metadata)
        })
}

fn syntax_tag_to_wit(tag: &skript_parser::ParseTagCapture) -> WitSyntaxTag {
    WitSyntaxTag {
        value: tag.value.clone(),
        pattern_span: WitTextRange {
            start: tag.pattern_span.start as u64,
            end: tag.pattern_span.end as u64,
        },
        input_span: mapped_span_to_wit(tag.input_span.mapped.clone()),
        implicit: tag.implicit,
    }
}

fn syntax_mark_to_wit(mark: &skript_parser::ParseMarkCapture) -> WitSyntaxMark {
    WitSyntaxMark {
        value: mark.value,
        pattern_span: WitTextRange {
            start: mark.pattern_span.start as u64,
            end: mark.pattern_span.end as u64,
        },
        input_span: mapped_span_to_wit(mark.input_span.mapped.clone()),
        accumulated: mark.accumulated,
    }
}

fn same_syntax_tags(left: &[WitSyntaxTag], right: &[WitSyntaxTag]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.value == right.value
                && left.pattern_span.start == right.pattern_span.start
                && left.pattern_span.end == right.pattern_span.end
                && same_mapped_span(&left.input_span, &right.input_span)
                && left.implicit == right.implicit
        })
}

fn same_syntax_marks(left: &[WitSyntaxMark], right: &[WitSyntaxMark]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.value == right.value
                && left.pattern_span.start == right.pattern_span.start
                && left.pattern_span.end == right.pattern_span.end
                && same_mapped_span(&left.input_span, &right.input_span)
                && left.accumulated == right.accumulated
        })
}

fn condition_hook_payload(
    input: &str,
    context: &ExpressionParseContext,
    candidate: &skript_parser::ConditionCandidate,
    catalog: &Catalog,
) -> Result<WitConditionPayload, String> {
    let ConditionNodeKind::Registered {
        definition_id,
        registration_id,
        pattern_index,
        pattern,
        priority,
        registration_order,
    } = &candidate.node.kind
    else {
        return Err("grouped Conditions do not have registered semantics".to_owned());
    };
    let element_class = catalog
        .conditions()
        .find(|condition| condition.common.registration_id.as_str() == registration_id)
        .map(|condition| condition.common.element_class.as_str().to_owned());
    Ok(WitConditionPayload {
        input: input.to_owned(),
        context: parse_context_to_wit(context),
        candidate: WitConditionCandidate {
            definition_id: definition_id.clone(),
            registration_id: registration_id.clone(),
            element_class,
            priority: *priority,
            registration_order: u64::try_from(*registration_order).unwrap_or(u64::MAX),
            pattern_index: u64::try_from(*pattern_index).unwrap_or(u64::MAX),
            pattern: pattern.clone(),
            span: mapped_span_to_wit(candidate.node.span.mapped.clone()),
            captures: candidate
                .node
                .captures
                .iter()
                .map(|capture| match capture {
                    PatternCapture::Regex {
                        pattern_span,
                        value,
                        span,
                        ..
                    } => WitConditionCapture::Regex(WitConditionRegexCapture {
                        pattern_span: WitTextRange {
                            start: pattern_span.start as u64,
                            end: pattern_span.end as u64,
                        },
                        value: value.clone(),
                        span: mapped_span_to_wit(span.mapped.clone()),
                    }),
                    PatternCapture::TypeExpression {
                        pattern_span,
                        expression,
                        value,
                        span,
                        alternative_index,
                        resolution_id,
                        ..
                    } => WitConditionCapture::Expression(WitConditionExpressionCapture {
                        pattern_span: WitTextRange {
                            start: pattern_span.start as u64,
                            end: pattern_span.end as u64,
                        },
                        expression: expression.display_with(catalog.plural_rules()).to_string(),
                        value: value.clone(),
                        span: mapped_span_to_wit(span.mapped.clone()),
                        alternative_index: alternative_index.map(|index| index as u64),
                        resolution_id: resolution_id.clone(),
                    }),
                })
                .collect(),
            tags: candidate
                .node
                .tags
                .iter()
                .map(|tag| WitConditionTag {
                    value: tag.value.clone(),
                    pattern_span: WitTextRange {
                        start: tag.pattern_span.start as u64,
                        end: tag.pattern_span.end as u64,
                    },
                    input_span: mapped_span_to_wit(tag.input_span.mapped.clone()),
                    implicit: tag.implicit,
                })
                .collect(),
            mark: candidate.node.mark,
            marks: candidate
                .node
                .marks
                .iter()
                .map(|mark| WitConditionMark {
                    value: mark.value,
                    pattern_span: WitTextRange {
                        start: mark.pattern_span.start as u64,
                        end: mark.pattern_span.end as u64,
                    },
                    input_span: mapped_span_to_wit(mark.input_span.mapped.clone()),
                    accumulated: mark.accumulated,
                })
                .collect(),
            handler: candidate.node.handler.clone(),
            metadata: metadata_to_wit(&candidate.node.metadata),
            children: candidate
                .node
                .expressions
                .iter()
                .map(|child| expression_child_to_wit(child, input, Some(catalog)))
                .collect(),
        },
    })
}

struct EffectHookPayloadView<'a> {
    input: &'a str,
    context: &'a ExpressionParseContext,
    raw_node_id: ParserRawNodeId,
    span: &'a skript_parser::MappedSpan,
    timing: WitEffectTiming,
    candidate: Option<&'a EffectCandidate>,
    alternatives: &'a [EffectCandidate],
    failure: Option<&'a PatternFailure>,
    near_match: Option<&'a EffectCandidateFailure>,
    catalog: &'a Catalog,
}

fn parsed_capture_to_wit(capture: &ParserParsedCapture, input: &str) -> WitParsedCapture {
    WitParsedCapture {
        capture_index: u64::try_from(capture.capture_index).unwrap_or(u64::MAX),
        parser_id: capture.result.parser_id.clone(),
        status: match capture.result.status {
            ParserParsedCaptureStatus::Success => WitParseResultStatus::Success,
            ParserParsedCaptureStatus::Partial => WitParseResultStatus::Partial,
            ParserParsedCaptureStatus::Failed => WitParseResultStatus::Failed,
        },
        text: capture
            .result
            .span
            .local_range
            .slice(input)
            .unwrap_or_default()
            .to_owned(),
        span: mapped_span_to_wit(capture.result.span.mapped.clone()),
        expected_types: capture
            .result
            .summary
            .as_ref()
            .and_then(|summary| summary.return_type.as_ref())
            .map(|class_name| {
                vec![WitExpressionExpectedType {
                    class_name: class_name.as_str().to_owned(),
                    plural: capture
                        .result
                        .summary
                        .as_ref()
                        .and_then(|summary| summary.multiplicity)
                        == Some(Multiplicity::Multiple),
                }]
            })
            .unwrap_or_default(),
        summary: capture
            .result
            .summary
            .as_ref()
            .map(|summary| WitParseSummary {
                kind: summary.kind.clone(),
                definition_id: summary.definition_id.clone(),
                registration_id: summary.registration_id.clone(),
                element_class: summary
                    .element_class
                    .as_ref()
                    .map(|class_name| class_name.as_str().to_owned()),
                pattern_index: summary
                    .pattern_index
                    .and_then(|index| u64::try_from(index).ok()),
                return_type: summary
                    .return_type
                    .as_ref()
                    .map(|class_name| class_name.as_str().to_owned()),
                possible_return_types: summary
                    .possible_return_types
                    .iter()
                    .map(|class_name| class_name.as_str().to_owned())
                    .collect(),
                possible_return_types_state: match summary.possible_return_types_state {
                    PossibleReturnTypesState::Complete => WitPossibleReturnTypesState::Complete,
                    PossibleReturnTypesState::Partial => WitPossibleReturnTypesState::Partial,
                    PossibleReturnTypesState::Unresolved => WitPossibleReturnTypesState::Unresolved,
                },
                multiplicity: summary.multiplicity.map(multiplicity_to_wit),
                public_data: public_data::to_wit(&summary.public_data),
                metadata: metadata_to_wit(&summary.metadata),
            }),
        attachments: capture
            .result
            .attachments
            .iter()
            .map(|attachment| WitAddonAttachment {
                owner_component_id: attachment.owner_component_id.clone(),
                schema_id: attachment.schema_id.clone(),
                schema_version: attachment.schema_version,
                encoding: attachment.encoding.clone(),
                bytes: attachment.bytes.clone(),
            })
            .collect(),
        diagnostics: capture
            .result
            .diagnostics
            .iter()
            .map(|diagnostic| Diagnostic {
                code: diagnostic
                    .metadata
                    .get("code")
                    .cloned()
                    .unwrap_or_else(|| "parser.capture".to_owned()),
                severity: DiagnosticSeverity::Error,
                message: diagnostic.message.clone(),
                span: mapped_span_to_wit(
                    diagnostic
                        .span
                        .as_ref()
                        .unwrap_or(&capture.result.span)
                        .mapped
                        .clone(),
                ),
                related: Vec::new(),
            })
            .collect(),
    }
}

fn effect_hook_payload(view: EffectHookPayloadView<'_>) -> WitEffectPayload {
    WitEffectPayload {
        input: view.input.to_owned(),
        context: parse_context_to_wit(view.context),
        raw_node_id: view.raw_node_id.get(),
        span: mapped_span_to_wit(view.span.clone()),
        timing: view.timing,
        candidate: view
            .candidate
            .map(|candidate| effect_candidate_to_wit(candidate, view.catalog, view.input)),
        alternatives: view
            .alternatives
            .iter()
            .map(|candidate| effect_candidate_to_wit(candidate, view.catalog, view.input))
            .collect(),
        failure: view.failure.map(effect_failure_to_wit),
        near_match: view.near_match.map(effect_near_match_to_wit),
    }
}

fn effect_near_match_to_wit(candidate: &EffectCandidateFailure) -> WitEffectNearMatch {
    WitEffectNearMatch {
        definition_id: candidate.matched.definition_id.clone(),
        registration_id: candidate.matched.registration_id.clone(),
        element_class: candidate
            .element_class
            .as_ref()
            .map(|class| class.as_str().to_owned()),
        priority: candidate.matched.priority,
        registration_order: u64::try_from(candidate.matched.registration_order).unwrap_or(u64::MAX),
        resolved_order: candidate
            .matched
            .resolved_order
            .and_then(|order| u64::try_from(order).ok()),
        handler: candidate.handler.clone(),
        metadata: metadata_to_wit(&candidate.metadata),
        failure: effect_failure_to_wit(&candidate.matched.trace.root_cause().failure),
    }
}

fn effect_candidate_to_wit(
    candidate: &EffectCandidate,
    catalog: &Catalog,
    input: &str,
) -> WitEffectCandidate {
    WitEffectCandidate {
        raw_node_id: candidate.raw_node_id.get(),
        definition_id: candidate.matched.definition_id.clone(),
        registration_id: candidate.matched.registration_id.clone(),
        element_class: effect_candidate_element_class(candidate, catalog),
        priority: candidate.matched.priority,
        registration_order: u64::try_from(candidate.matched.registration_order).unwrap_or(u64::MAX),
        pattern_index: u64::try_from(candidate.matched.pattern_index).unwrap_or(u64::MAX),
        pattern: candidate.matched.pattern.clone(),
        span: mapped_span_to_wit(candidate.matched.matched.span.mapped.clone()),
        captures: candidate
            .matched
            .matched
            .captures
            .iter()
            .map(|capture| match capture {
                PatternCapture::Regex {
                    pattern_span,
                    value,
                    span,
                    ..
                } => WitEffectCapture::Regex(WitEffectRegexCapture {
                    pattern_span: WitTextRange {
                        start: pattern_span.start as u64,
                        end: pattern_span.end as u64,
                    },
                    value: value.clone(),
                    span: mapped_span_to_wit(span.mapped.clone()),
                }),
                PatternCapture::TypeExpression {
                    pattern_span,
                    expression,
                    value,
                    span,
                    alternative_index,
                    resolution_id,
                    ..
                } => WitEffectCapture::Expression(WitEffectExpressionCapture {
                    pattern_span: WitTextRange {
                        start: pattern_span.start as u64,
                        end: pattern_span.end as u64,
                    },
                    expression: expression.display_with(catalog.plural_rules()).to_string(),
                    value: value.clone(),
                    span: mapped_span_to_wit(span.mapped.clone()),
                    alternative_index: alternative_index.map(|index| index as u64),
                    resolution_id: resolution_id.clone(),
                }),
            })
            .collect(),
        tags: candidate
            .matched
            .matched
            .tags
            .iter()
            .map(|tag| WitEffectTag {
                value: tag.value.clone(),
                pattern_span: WitTextRange {
                    start: tag.pattern_span.start as u64,
                    end: tag.pattern_span.end as u64,
                },
                input_span: mapped_span_to_wit(tag.input_span.mapped.clone()),
                implicit: tag.implicit,
            })
            .collect(),
        mark: candidate.matched.matched.mark,
        marks: candidate
            .matched
            .matched
            .marks
            .iter()
            .map(|mark| WitEffectMark {
                value: mark.value,
                pattern_span: WitTextRange {
                    start: mark.pattern_span.start as u64,
                    end: mark.pattern_span.end as u64,
                },
                input_span: mapped_span_to_wit(mark.input_span.mapped.clone()),
                accumulated: mark.accumulated,
            })
            .collect(),
        handler: candidate.handler.clone(),
        metadata: metadata_to_wit(&candidate.metadata),
        parsed_captures: candidate
            .parsed_captures
            .iter()
            .map(|capture| parsed_capture_to_wit(capture, input))
            .collect(),
    }
}

fn effect_candidate_element_class(
    candidate: &EffectCandidate,
    catalog: &Catalog,
) -> Option<String> {
    catalog
        .syntax_by_registration_id(&candidate.matched.registration_id)
        .into_iter()
        .find_map(|syntax| match (candidate.matched.kind, syntax) {
            (MatchSyntaxKind::Effect, Syntax::Effect(effect))
                if effect.common.definition_id.as_str() == candidate.matched.definition_id =>
            {
                Some(effect.common.element_class.as_str().to_owned())
            }
            (MatchSyntaxKind::Section, Syntax::Section(section))
                if section.effect_section
                    && section.common.definition_id.as_str() == candidate.matched.definition_id =>
            {
                Some(section.common.element_class.as_str().to_owned())
            }
            _ => None,
        })
}

fn effect_failure_to_wit(failure: &PatternFailure) -> WitEffectFailure {
    WitEffectFailure {
        offset: failure.span.mapped.virtual_range.start as u64,
        span: mapped_span_to_wit(failure.span.mapped.clone()),
        reasons: failure.reasons.iter().map(effect_failure_reason).collect(),
    }
}

fn unknown_effect_failure(unknown: &UnknownEffectNode) -> Option<&PatternFailure> {
    unknown
        .failures
        .primary()
        .map(|candidate| &candidate.matched.trace.root_cause().failure)
        .or_else(|| {
            unknown
                .failures
                .fallback
                .as_ref()
                .map(|trace| &trace.root_cause().failure)
        })
}

fn effect_failure_reason(reason: &PatternFailureReason) -> String {
    match reason {
        PatternFailureReason::Literal { expected } => format!("literal:{expected}"),
        PatternFailureReason::Regex { pattern } => format!("regex:{pattern}"),
        PatternFailureReason::Expression => "expression:unrecognized".to_owned(),
        PatternFailureReason::TypeExpression { expected } => {
            format!("expression:{}", expected.join("/"))
        }
        PatternFailureReason::EventRestricted { supported, current } => format!(
            "event-restricted:supported={};current={}",
            supported.join("/"),
            current.join("/")
        ),
        PatternFailureReason::TypeParserUnresolved {
            definition_id,
            registration_id,
            parser_class,
            reason,
            required_provider,
        } => format!(
            "type-parser-unresolved:definition={definition_id};registration={registration_id};parser={};provider={};reason={reason}",
            parser_class.as_deref().unwrap_or("<unknown>"),
            required_provider.as_deref().unwrap_or("<unspecified>")
        ),
        PatternFailureReason::TrailingInput => "trailing-input".to_owned(),
        PatternFailureReason::HookRejected { reason } => format!("hook-rejected:{reason}"),
    }
}

fn validate_condition_payload_identity(
    original: &WitConditionPayload,
    output: &WitConditionPayload,
) -> Result<(), HostError> {
    let left = &original.candidate;
    let right = &output.candidate;
    if output.input != original.input
        || !same_parse_context(&output.context, &original.context)
        || left.definition_id != right.definition_id
        || left.registration_id != right.registration_id
        || left.element_class != right.element_class
        || left.priority != right.priority
        || left.registration_order != right.registration_order
        || left.pattern_index != right.pattern_index
        || left.pattern != right.pattern
        || !same_mapped_span(&left.span, &right.span)
        || !same_condition_captures(&left.captures, &right.captures)
        || !same_condition_tags(&left.tags, &right.tags)
        || left.mark != right.mark
        || !same_condition_marks(&left.marks, &right.marks)
        || !same_registered_expression_children(&left.children, &right.children)
    {
        return Err(HostError::InvalidConditionHookOutput {
            message: "hook changed immutable Condition identity or captures".to_owned(),
        });
    }
    Ok(())
}

fn same_condition_captures(left: &[WitConditionCapture], right: &[WitConditionCapture]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| match (left, right) {
                (WitConditionCapture::Regex(left), WitConditionCapture::Regex(right)) => {
                    left.pattern_span.start == right.pattern_span.start
                        && left.pattern_span.end == right.pattern_span.end
                        && left.value == right.value
                        && same_mapped_span(&left.span, &right.span)
                }
                (WitConditionCapture::Expression(left), WitConditionCapture::Expression(right)) => {
                    left.pattern_span.start == right.pattern_span.start
                        && left.pattern_span.end == right.pattern_span.end
                        && left.expression == right.expression
                        && left.value == right.value
                        && same_mapped_span(&left.span, &right.span)
                        && left.alternative_index == right.alternative_index
                        && left.resolution_id == right.resolution_id
                }
                _ => false,
            })
}

fn same_condition_tags(left: &[WitConditionTag], right: &[WitConditionTag]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.value == right.value
                && left.pattern_span.start == right.pattern_span.start
                && left.pattern_span.end == right.pattern_span.end
                && same_mapped_span(&left.input_span, &right.input_span)
                && left.implicit == right.implicit
        })
}

fn same_condition_marks(left: &[WitConditionMark], right: &[WitConditionMark]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.value == right.value
                && left.pattern_span.start == right.pattern_span.start
                && left.pattern_span.end == right.pattern_span.end
                && same_mapped_span(&left.input_span, &right.input_span)
                && left.accumulated == right.accumulated
        })
}

fn same_registered_expression_children(
    left: &[WitRegisteredExpressionChild],
    right: &[WitRegisteredExpressionChild],
) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.text == right.text
                && left.kind == right.kind
                && left.parser_id == right.parser_id
                && left.definition_id == right.definition_id
                && left.registration_id == right.registration_id
                && left.pattern_index == right.pattern_index
                && left.element_class == right.element_class
                && left.return_type == right.return_type
                && left.possible_return_types == right.possible_return_types
                && left.possible_return_types_state == right.possible_return_types_state
                && left.multiplicity == right.multiplicity
                && public_data::same(&left.public_data, &right.public_data)
                && same_metadata_entries(&left.metadata, &right.metadata)
        })
}

fn validate_effect_payload_identity(
    original: &WitEffectPayload,
    output: &WitEffectPayload,
    allow_candidate_replacement: bool,
) -> Result<(), HostError> {
    if output.input != original.input
        || !same_parse_context(&output.context, &original.context)
        || output.raw_node_id != original.raw_node_id
        || !same_mapped_span(&output.span, &original.span)
        || output.timing != original.timing
        || !same_effect_candidates(&output.alternatives, &original.alternatives)
        || !same_effect_failure(output.failure.as_ref(), original.failure.as_ref())
        || !same_effect_near_match(output.near_match.as_ref(), original.near_match.as_ref())
    {
        return Err(HostError::InvalidEffectHookOutput {
            message: "hook changed immutable Effect input, alternatives, or failure fields"
                .to_owned(),
        });
    }
    match (&original.candidate, &output.candidate) {
        (None, None) => Ok(()),
        (Some(original), Some(output)) if allow_candidate_replacement => {
            if same_effect_candidate_identity(original, output) {
                Ok(())
            } else {
                Err(HostError::InvalidEffectHookOutput {
                    message: "hook changed immutable Effect candidate identity or captures"
                        .to_owned(),
                })
            }
        }
        (Some(original), Some(output)) if same_effect_candidate_full(original, output) => Ok(()),
        (Some(_), Some(_)) | (Some(_), None) | (None, Some(_)) => {
            Err(HostError::InvalidEffectHookOutput {
                message: "hook added, removed, or illegally replaced the Effect candidate"
                    .to_owned(),
            })
        }
    }
}

fn same_effect_near_match(
    left: Option<&WitEffectNearMatch>,
    right: Option<&WitEffectNearMatch>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.definition_id == right.definition_id
                && left.registration_id == right.registration_id
                && left.element_class == right.element_class
                && left.priority == right.priority
                && left.registration_order == right.registration_order
                && left.resolved_order == right.resolved_order
                && left.handler == right.handler
                && same_metadata_entries(&left.metadata, &right.metadata)
                && same_effect_failure(Some(&left.failure), Some(&right.failure))
        }
        (None, Some(_)) | (Some(_), None) => false,
    }
}

fn same_effect_candidate_identity(
    original: &WitEffectCandidate,
    output: &WitEffectCandidate,
) -> bool {
    output.raw_node_id == original.raw_node_id
        && output.definition_id == original.definition_id
        && output.registration_id == original.registration_id
        && output.element_class == original.element_class
        && output.priority == original.priority
        && output.registration_order == original.registration_order
        && output.pattern_index == original.pattern_index
        && output.pattern == original.pattern
        && same_mapped_span(&output.span, &original.span)
        && same_effect_captures(&output.captures, &original.captures)
        && same_effect_tags(&output.tags, &original.tags)
        && output.mark == original.mark
        && same_effect_marks(&output.marks, &original.marks)
        && same_parsed_captures(&output.parsed_captures, &original.parsed_captures)
}

fn same_effect_candidate_full(left: &WitEffectCandidate, right: &WitEffectCandidate) -> bool {
    same_effect_candidate_identity(left, right)
        && left.handler == right.handler
        && same_metadata_entries(&left.metadata, &right.metadata)
}

fn same_effect_candidates(left: &[WitEffectCandidate], right: &[WitEffectCandidate]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| same_effect_candidate_full(left, right))
}

fn same_effect_failure(left: Option<&WitEffectFailure>, right: Option<&WitEffectFailure>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.offset == right.offset
                && same_mapped_span(&left.span, &right.span)
                && left.reasons == right.reasons
        }
        (None, Some(_)) | (Some(_), None) => false,
    }
}

fn same_effect_captures(left: &[WitEffectCapture], right: &[WitEffectCapture]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| match (left, right) {
                (WitEffectCapture::Regex(left), WitEffectCapture::Regex(right)) => {
                    same_wit_range(&left.pattern_span, &right.pattern_span)
                        && left.value == right.value
                        && same_mapped_span(&left.span, &right.span)
                }
                (WitEffectCapture::Expression(left), WitEffectCapture::Expression(right)) => {
                    same_wit_range(&left.pattern_span, &right.pattern_span)
                        && left.expression == right.expression
                        && left.value == right.value
                        && same_mapped_span(&left.span, &right.span)
                        && left.alternative_index == right.alternative_index
                        && left.resolution_id == right.resolution_id
                }
                (WitEffectCapture::Regex(_), WitEffectCapture::Expression(_))
                | (WitEffectCapture::Expression(_), WitEffectCapture::Regex(_)) => false,
            })
}

fn same_effect_tags(left: &[WitEffectTag], right: &[WitEffectTag]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.value == right.value
                && same_wit_range(&left.pattern_span, &right.pattern_span)
                && same_mapped_span(&left.input_span, &right.input_span)
                && left.implicit == right.implicit
        })
}

fn same_effect_marks(left: &[WitEffectMark], right: &[WitEffectMark]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.value == right.value
                && same_wit_range(&left.pattern_span, &right.pattern_span)
                && same_mapped_span(&left.input_span, &right.input_span)
                && left.accumulated == right.accumulated
        })
}

fn same_metadata_entries(left: &[WitMetadataEntry], right: &[WitMetadataEntry]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.owner_component_id == right.owner_component_id
                && left.key == right.key
                && left.value == right.value
        })
}

fn same_parsed_captures(left: &[WitParsedCapture], right: &[WitParsedCapture]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.capture_index == right.capture_index
                && left.parser_id == right.parser_id
                && left.status == right.status
                && left.text == right.text
                && same_mapped_span(&left.span, &right.span)
                && left.expected_types.len() == right.expected_types.len()
                && left
                    .expected_types
                    .iter()
                    .zip(&right.expected_types)
                    .all(|(left, right)| {
                        left.class_name == right.class_name && left.plural == right.plural
                    })
                && same_parse_summary(left.summary.as_ref(), right.summary.as_ref())
                && left.attachments.len() == right.attachments.len()
                && left
                    .attachments
                    .iter()
                    .zip(&right.attachments)
                    .all(|(left, right)| {
                        left.owner_component_id == right.owner_component_id
                            && left.schema_id == right.schema_id
                            && left.schema_version == right.schema_version
                            && left.encoding == right.encoding
                            && left.bytes == right.bytes
                    })
                && same_diagnostics(&left.diagnostics, &right.diagnostics)
        })
}

fn same_parse_summary(left: Option<&WitParseSummary>, right: Option<&WitParseSummary>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.kind == right.kind
                && left.definition_id == right.definition_id
                && left.registration_id == right.registration_id
                && left.element_class == right.element_class
                && left.pattern_index == right.pattern_index
                && left.return_type == right.return_type
                && left.multiplicity == right.multiplicity
                && left.possible_return_types == right.possible_return_types
                && left.possible_return_types_state == right.possible_return_types_state
                && public_data::same(&left.public_data, &right.public_data)
                && same_metadata_entries(&left.metadata, &right.metadata)
        }
        _ => false,
    }
}

fn same_diagnostics(left: &[Diagnostic], right: &[Diagnostic]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.code == right.code
                && left.message == right.message
                && left.severity == right.severity
                && same_mapped_span(&left.span, &right.span)
                && left.related.len() == right.related.len()
                && left
                    .related
                    .iter()
                    .zip(&right.related)
                    .all(|(left, right)| {
                        left.message == right.message && same_mapped_span(&left.span, &right.span)
                    })
        })
}
fn apply_effect_hook_replacement(
    matches: &mut EffectMatches,
    output: WitEffectPayload,
) -> Result<(), HostError> {
    let selected = matches
        .selected
        .as_mut()
        .ok_or_else(|| HostError::InvalidEffectHookOutput {
            message: "Effect replacement requires a selected candidate".to_owned(),
        })?;
    let output = output
        .candidate
        .ok_or_else(|| HostError::InvalidEffectHookOutput {
            message: "Effect hook removed the selected candidate".to_owned(),
        })?;
    let metadata = metadata_entries(output.metadata)
        .map_err(|message| HostError::InvalidEffectHookOutput { message })?;
    selected.handler = output.handler;
    selected.metadata = metadata;
    Ok(())
}

fn merge_decision_diagnostics(effects: &mut HookEffects, decision: &HookDecision) {
    if let HookDecision::Reject(rejection) = decision {
        effects.diagnostics.extend(rejection.diagnostics.clone());
    }
}

fn semantic_rejection_diagnostics(
    decision: &HookDecision,
    effects: &HookEffects,
) -> Result<Vec<ParserSemanticDiagnostic>, String> {
    let HookDecision::Reject(rejection) = decision else {
        return Ok(Vec::new());
    };
    rejection
        .diagnostics
        .iter()
        .chain(&effects.diagnostics)
        .map(semantic_diagnostic_from_wit)
        .collect()
}

fn semantic_diagnostic_from_wit(
    diagnostic: &Diagnostic,
) -> Result<ParserSemanticDiagnostic, String> {
    Ok(ParserSemanticDiagnostic {
        code: diagnostic.code.clone(),
        message: diagnostic.message.clone(),
        severity: match diagnostic.severity {
            DiagnosticSeverity::Error => ParserSemanticDiagnosticSeverity::Error,
            DiagnosticSeverity::Warning => ParserSemanticDiagnosticSeverity::Warning,
            DiagnosticSeverity::Information => ParserSemanticDiagnosticSeverity::Information,
            DiagnosticSeverity::Hint => ParserSemanticDiagnosticSeverity::Hint,
        },
        span: parser_mapped_span_from_wit(&diagnostic.span)?,
        related: diagnostic
            .related
            .iter()
            .map(|related| {
                Ok(ParserSemanticRelatedSpan {
                    message: related.message.clone(),
                    span: parser_mapped_span_from_wit(&related.span)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
    })
}

fn parser_mapped_span_from_wit(span: &MappedSpan) -> Result<skript_parser::MappedSpan, String> {
    let start = usize::try_from(span.virtual_range.start)
        .map_err(|_| "diagnostic span start does not fit usize".to_owned())?;
    let end = usize::try_from(span.virtual_range.end)
        .map_err(|_| "diagnostic span end does not fit usize".to_owned())?;
    let origins = span
        .origins
        .iter()
        .map(|origin| {
            let original_start = usize::try_from(origin.original_range.start)
                .map_err(|_| "diagnostic origin start does not fit usize".to_owned())?;
            let original_end = usize::try_from(origin.original_range.end)
                .map_err(|_| "diagnostic origin end does not fit usize".to_owned())?;
            let expansion = origin
                .expansion
                .map(|value| {
                    u32::try_from(value)
                        .map(ExpansionId::new)
                        .map_err(|_| "diagnostic expansion ID does not fit u32".to_owned())
                })
                .transpose()?;
            Ok(skript_parser::SourceOrigin {
                original_range: ParserTextRange::new(original_start, original_end),
                kind: match origin.kind {
                    WitOriginKind::Exact => ParserOriginKind::Exact,
                    WitOriginKind::Replaced => ParserOriginKind::Replaced,
                    WitOriginKind::Anchored => ParserOriginKind::Anchored,
                },
                expansion,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(skript_parser::MappedSpan {
        virtual_range: ParserTextRange::new(start, end),
        origins,
    })
}

fn promote_semantic_diagnostics(effects: &mut HookEffects, trace: Option<&FailureTrace>) {
    let Some(trace) = trace else {
        return;
    };
    for diagnostic in trace.semantic_diagnostics() {
        let diagnostic = semantic_diagnostic_to_wit(diagnostic);
        if !effects
            .diagnostics
            .iter()
            .any(|existing| same_diagnostic(existing, &diagnostic))
        {
            effects.diagnostics.push(diagnostic);
        }
    }
}

fn promote_candidate_semantic_diagnostics(effects: &mut HookEffects, candidate: &CandidateFailure) {
    promote_semantic_diagnostics(effects, Some(&candidate.trace));
    for related in &candidate.related {
        promote_semantic_diagnostics(effects, Some(related));
    }
}

fn semantic_diagnostic_to_wit(diagnostic: &ParserSemanticDiagnostic) -> Diagnostic {
    Diagnostic {
        code: diagnostic.code.clone(),
        message: diagnostic.message.clone(),
        severity: match diagnostic.severity {
            ParserSemanticDiagnosticSeverity::Error => DiagnosticSeverity::Error,
            ParserSemanticDiagnosticSeverity::Warning => DiagnosticSeverity::Warning,
            ParserSemanticDiagnosticSeverity::Information => DiagnosticSeverity::Information,
            ParserSemanticDiagnosticSeverity::Hint => DiagnosticSeverity::Hint,
        },
        span: mapped_span_to_wit(diagnostic.span.clone()),
        related: diagnostic
            .related
            .iter()
            .map(|related| RelatedSpan {
                message: related.message.clone(),
                span: mapped_span_to_wit(related.span.clone()),
            })
            .collect(),
    }
}

fn same_diagnostic(left: &Diagnostic, right: &Diagnostic) -> bool {
    left.code == right.code
        && left.message == right.message
        && left.severity == right.severity
        && same_mapped_span(&left.span, &right.span)
        && left.related.len() == right.related.len()
        && left
            .related
            .iter()
            .zip(&right.related)
            .all(|(left, right)| {
                left.message == right.message && same_mapped_span(&left.span, &right.span)
            })
}

fn unknown_effect_matches(
    source: &MappedSource,
    node: &skript_parser::RawNode,
    failure: Option<PatternFailure>,
    candidate: Option<EffectCandidateFailure>,
) -> Result<EffectMatches, HostError> {
    let code_span = node
        .code_span
        .clone()
        .ok_or(EffectParseError::MissingCodeSpan { node_id: node.id })?;
    let range = code_span.virtual_range;
    let input = range
        .slice(source.virtual_source())
        .ok_or(EffectParseError::InvalidCodeRange { range })?
        .to_owned();
    Ok(EffectMatches {
        selected: None,
        alternatives: Vec::new(),
        unknown: Some(UnknownEffectNode {
            raw_node_id: node.id,
            source: input,
            span: MatchSpan {
                local_range: range,
                mapped: code_span,
            },
            failures: RankedFailures {
                fallback: failure.map(FailureTrace::leaf),
                candidates: candidate.into_iter().collect(),
            },
        }),
    })
}
fn wit_expression_candidate(
    candidate: WitExpressionLeafCandidate,
    available_parse_results: &BTreeMap<u64, ExecutedParseResult>,
) -> Result<ExpressionLeafCandidate, String> {
    if candidate.parser_id.trim().is_empty() {
        return Err("Expression candidate parser ID is blank".to_owned());
    }
    let start = usize::try_from(candidate.range.start)
        .map_err(|_| "Expression candidate start does not fit usize".to_owned())?;
    let end = usize::try_from(candidate.range.end)
        .map_err(|_| "Expression candidate end does not fit usize".to_owned())?;
    let metadata = metadata_entries(candidate.metadata)?;
    let mut children = Vec::with_capacity(candidate.children.len());
    for reference in candidate.children {
        let result = available_parse_results
            .get(&reference.host_token)
            .ok_or_else(|| {
                format!(
                    "Expression candidate {} references unavailable parse-result token {}",
                    candidate.parser_id, reference.host_token
                )
            })?;
        if result.wire.parser_id != skript_parser::HOST_EXPRESSION_PARSER_ID {
            return Err(format!(
                "Expression candidate {} references non-Expression parser {}",
                candidate.parser_id, result.wire.parser_id
            ));
        }
        let child = result
            .expression_roots
            .get(&reference.root_id)
            .ok_or_else(|| {
                format!(
                    "Expression candidate {} references unavailable root {} from token {}",
                    candidate.parser_id, reference.root_id, reference.host_token
                )
            })?;
        children.push(child.clone());
    }
    Ok(ExpressionLeafCandidate {
        effects: None,
        parser_id: candidate.parser_id,
        kind: match candidate.kind {
            WitExpressionLeafKind::Variable => ExpressionLeafKind::Variable,
            WitExpressionLeafKind::Literal => ExpressionLeafKind::Literal,
            WitExpressionLeafKind::Function => ExpressionLeafKind::Function,
            WitExpressionLeafKind::Custom => ExpressionLeafKind::Custom,
        },
        timing: match candidate.timing {
            crate::bindings::nlaocs::skript_parser_addon::types::ExpressionLeafTiming::BeforeRegistered =>
                skript_parser::ExpressionLeafTiming::BeforeRegistered,
            crate::bindings::nlaocs::skript_parser_addon::types::ExpressionLeafTiming::AfterRegistered =>
                skript_parser::ExpressionLeafTiming::AfterRegistered,
        },
        range: ParserTextRange::new(start, end),
        return_type: candidate.return_type.map(ClassName),
        multiplicity: candidate.multiplicity.map(|value| match value {
            WitDynamicMultiplicity::Single => Multiplicity::Single,
            WitDynamicMultiplicity::Multiple => Multiplicity::Multiple,
            WitDynamicMultiplicity::Both => Multiplicity::Both,
        }),
        children,
        public_data: public_data::from_wit(candidate.public_data)?,
        metadata,
    })
}

fn expression_type_options(
    catalog: Option<&Catalog>,
    input: &str,
    remaining: ParserTextRange,
    candidate_ends: &[usize],
    expected_types: &[skript_parser::ExpressionExpectedType],
) -> Vec<WitExpressionTypeOption> {
    let Some(catalog) = catalog else {
        return Vec::new();
    };
    let expects_class_info = catalog
        .type_by_code_name("classinfo")
        .is_some_and(|class_info| {
            expected_types.is_empty()
                || expected_types.iter().any(|expected| {
                    catalog.can_convert(
                        class_info.original_class.as_str(),
                        expected.class_name.as_str(),
                    )
                })
        });
    let candidate_inputs = candidate_ends
        .iter()
        .filter_map(|end| input.get(remaining.start..*end))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    all_expression_type_options(Some(catalog))
        .into_iter()
        .filter(|option| {
            option.has_parser
                && (expected_types
                    .iter()
                    .any(|expected| option.class_name == expected.class_name.as_str())
                    || expects_class_info
                        && candidate_inputs
                            .iter()
                            .any(|input| type_option_matches_input(option, input)))
        })
        .collect()
}

thread_local! {
    static TYPE_USER_INPUT_PATTERNS: RefCell<HashMap<String, Option<fancy_regex::Regex>>> =
        RefCell::new(HashMap::new());
}

fn type_option_matches_input(option: &WitExpressionTypeOption, input: &str) -> bool {
    if input.eq_ignore_ascii_case(&option.code_name)
        || input.eq_ignore_ascii_case(&option.singular)
        || input.eq_ignore_ascii_case(&option.plural)
    {
        return true;
    }
    option.user_input_patterns.iter().any(|source| {
        TYPE_USER_INPUT_PATTERNS.with(|patterns| {
            let mut patterns = patterns.borrow_mut();
            patterns
                .entry(source.clone())
                .or_insert_with(|| fancy_regex::Regex::new(&format!("(?i)^(?:{source})$")).ok())
                .as_ref()
                .is_some_and(|pattern| pattern.is_match(input).unwrap_or(false))
        })
    })
}

fn all_expression_type_options(catalog: Option<&Catalog>) -> Vec<WitExpressionTypeOption> {
    let mut options = catalog
        .into_iter()
        .flat_map(Catalog::types)
        .map(|value| expression_type_option(catalog, value))
        .collect::<Vec<_>>();
    options.sort_by_key(|option| option.type_parse_order);
    options
}

fn expression_type_option(
    catalog: Option<&Catalog>,
    value: &syntaxes::Type,
) -> WitExpressionTypeOption {
    WitExpressionTypeOption {
        source_record: catalog.and_then(|catalog| {
            catalog_record_ref(
                catalog,
                "Types.json",
                value.source_index,
                value.registration_id.as_str(),
            )
        }),
        definition_id: value.definition_id.as_str().to_owned(),
        registration_id: value.registration_id.as_str().to_owned(),
        addon_name: value.addon.name.clone(),
        addon_version: value.addon.version.clone(),
        code_name: value.code_name.as_str().to_owned(),
        class_name: value.original_class.as_str().to_owned(),
        parser_class: value
            .parser_class
            .as_ref()
            .map(|class| class.as_str().to_owned()),
        type_parse_order: u64::try_from(value.type_parse_order).unwrap_or(u64::MAX),
        before: value
            .before
            .iter()
            .map(|code_name| code_name.as_str().to_owned())
            .collect(),
        after: value
            .after
            .iter()
            .map(|code_name| code_name.as_str().to_owned())
            .collect(),
        singular: value.noun.singular.clone(),
        plural: value.noun.plural.clone(),
        user_input_patterns: value.user_input_patterns.clone(),
        has_parser: value.has_parser,
        parse_contexts: value.parse_contexts.clone(),
        has_supplier: value.has_supplier,
    }
}

fn expression_parser_types(
    host: &ParserHost,
    expected_types: &[skript_parser::ExpressionExpectedType],
    literal_options: &[WitExpressionLiteralOption],
) -> Vec<WitExpressionTypeOption> {
    let Some(catalog) = host.config.syntax_catalog.as_deref() else {
        return Vec::new();
    };
    // Standard and third-party parsers use the same per-registration route.
    // Broad Type subscriptions also see finite values from any Addon snapshot.
    let type_handlers = host
        .components
        .iter()
        .filter(|component| !component.disabled && !component.unloaded)
        .flat_map(|component| {
            component
                .manifest
                .registered_syntax_handlers
                .iter()
                .filter(|handler| handler.kind == SyntaxKind::Type)
                .map(move |handler| (component, handler))
        })
        .collect::<Vec<_>>();
    let mut options = catalog
        .types()
        .filter(|value| value.has_parser)
        .filter(|value| {
            expected_types.is_empty()
                || expected_types.iter().any(|expected| {
                    catalog.can_convert(value.original_class.as_str(), expected.class_name.as_str())
                })
        })
        .filter(|value| {
            host.registry.has_active_more_specific_target(
                &host.components,
                &DispatchTarget::Registration {
                    syntax_kind: SyntaxKind::Type,
                    definition_id: value.definition_id.as_str().to_owned(),
                    registration_id: value.registration_id.as_str().to_owned(),
                },
                HookPhase::Expression,
            ) || literal_options.iter().any(|literal| {
                literal.code_name == value.code_name.as_str()
                    && literal.class_name == value.original_class.as_str()
                    && literal.type_parse_order == value.type_parse_order as u64
            }) || type_handlers.iter().any(|(component, handler)| {
                registered_handler_matches(
                    component,
                    handler,
                    RegisteredSyntaxIdentity {
                        kind: CatalogSyntaxKind::Type,
                        definition_id: value.definition_id.as_str(),
                        registration_id: value.registration_id.as_str(),
                        pattern_index: None,
                        pattern_source: None,
                        tags: None,
                        mark: None,
                        dynamic_handler: None,
                    },
                )
            }) || expected_types
                .iter()
                .any(|expected| expected.class_name.as_str() == value.original_class.as_str())
        })
        .map(|value| expression_type_option(Some(catalog), value))
        .collect::<Vec<_>>();
    // Classes.parse tries parseSimple before conversion. Never allow a Type
    // needing conversion to displace an otherwise directly assignable parser.
    // https://github.com/SkriptLang/Skript/blob/2.16.0/src/main/java/ch/njol/skript/registrations/Classes.java
    options.sort_by_key(|option| {
        let needs_conversion = !expected_types.is_empty()
            && !expected_types.iter().any(|expected| {
                catalog.is_class_assignable(&option.class_name, expected.class_name.as_str())
            });
        (needs_conversion, option.type_parse_order)
    });
    options
}

fn expression_literal_options(
    catalog: Option<&Catalog>,
    input: &str,
    remaining: ParserTextRange,
    candidate_ends: &[usize],
    expected_types: &[skript_parser::ExpressionExpectedType],
) -> Vec<WitExpressionLiteralOption> {
    let Some(catalog) = catalog else {
        return Vec::new();
    };
    let mut options = Vec::new();
    for end in candidate_ends.iter().copied().rev() {
        let Some(text) = input.get(remaining.start..end) else {
            continue;
        };
        let mut inputs = vec![(remaining.start, text)];
        if let Some(offset) = prefixed_literal_suffix_offset(text) {
            inputs.push((remaining.start + offset, &text[offset..]));
        }
        for (start, literal_text) in inputs {
            for matched in catalog.type_literal_matches(literal_text) {
                let value = matched.type_info;
                if !expected_types.is_empty()
                    && !expected_types.iter().any(|expected| {
                        catalog.can_convert(
                            value.original_class.as_str(),
                            expected.class_name.as_str(),
                        )
                    })
                {
                    continue;
                }
                let literal = matched.literal;
                let alias = matches!(matched.source, syntaxes::TypeLiteralSource::Alias)
                    .then(|| catalog.alias(matched.canonical_value))
                    .flatten();
                options.push(WitExpressionLiteralOption {
                    source_record: catalog_record_ref(
                        catalog,
                        "Types.json",
                        value.source_index,
                        value.registration_id.as_str(),
                    ),
                    literal_index: matched.literal_index.map(|index| index as u64),
                    code_name: value.code_name.as_str().to_owned(),
                    class_name: value.original_class.as_str().to_owned(),
                    type_parse_order: u64::try_from(value.type_parse_order).unwrap_or(u64::MAX),
                    range: WitTextRange {
                        start: u64::try_from(start).unwrap_or(u64::MAX),
                        end: u64::try_from(end).unwrap_or(u64::MAX),
                    },
                    canonical_value: matched.canonical_value.to_owned(),
                    source: match matched.source {
                        syntaxes::TypeLiteralSource::ParserPattern => {
                            WitExpressionLiteralSource::ParserPattern
                        }
                        syntaxes::TypeLiteralSource::Supplier => {
                            WitExpressionLiteralSource::Supplier
                        }
                        syntaxes::TypeLiteralSource::EnumConstant => {
                            WitExpressionLiteralSource::EnumConstant
                        }
                        syntaxes::TypeLiteralSource::Alias => WitExpressionLiteralSource::Alias,
                    },
                    plural: matched.plural,
                    addon_name: value.addon.name.clone(),
                    addon_version: value.addon.version.clone(),
                    parser_class: value
                        .parser_class
                        .as_ref()
                        .map(|class| class.as_str().to_owned()),
                    parse_contexts: value.parse_contexts.clone(),
                    value_class: literal.map(|literal| literal.value_class.as_str().to_owned()),
                    represented_class: literal
                        .and_then(|literal| literal.represented_class.as_ref())
                        .map(|class| class.as_str().to_owned()),
                    variable_name: literal.and_then(|literal| literal.variable_name.clone()),
                    debug_text: literal.and_then(|literal| literal.debug_text.clone()),
                    enum_constant: literal.and_then(|literal| literal.enum_constant.clone()),
                    alias_all: alias.map(|target| target.all),
                    alias_type_count: alias
                        .map(|target| u64::try_from(target.types.len()).unwrap_or(u64::MAX)),
                });
            }
        }
    }
    options
}

fn prefixed_literal_suffix_offset(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let digits = bytes
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    let mut offset = if digits > 0 {
        skip_ascii_spaces(text, digits)?
    } else {
        0
    };
    let mut prefixed = digits > 0;

    if digits > 0
        && let Some(next) = skip_ascii_word(text, offset, "of")
    {
        offset = next;
    }
    for word in ["all", "every", "a", "an"] {
        if let Some(next) = skip_ascii_word(text, offset, word) {
            offset = next;
            prefixed = true;
            break;
        }
    }
    (prefixed && offset < text.len()).then_some(offset)
}

fn skip_ascii_word(text: &str, offset: usize, word: &str) -> Option<usize> {
    let end = offset.checked_add(word.len())?;
    if !text.get(offset..end)?.eq_ignore_ascii_case(word) {
        return None;
    }
    skip_ascii_spaces(text, end)
}

fn skip_ascii_spaces(text: &str, offset: usize) -> Option<usize> {
    let spaces = text
        .as_bytes()
        .get(offset..)?
        .iter()
        .take_while(|byte| byte.is_ascii_whitespace())
        .count();
    (spaces > 0).then_some(offset + spaces)
}

fn registered_property_options(
    catalog: Option<&Catalog>,
    related_property: Option<&str>,
    children: &[skript_parser::ExpressionNode],
) -> Vec<WitRegisteredExpressionPropertyOption> {
    let (Some(catalog), Some(property_name)) = (catalog, related_property) else {
        return Vec::new();
    };
    let source_types = children
        .iter()
        .enumerate()
        // Target-type helpers describe a requested result, not the Property holder.
        .filter(|(_, child)| expression_child_semantic_role(child) != Some("target-type"))
        .flat_map(|(index, child)| {
            // PropertyBaseSyntax tests every possibleReturnTypes() entry. Keep the
            // effective type as a fallback for incomplete older SSG records.
            let mut types = child.possible_return_types.iter().collect::<Vec<_>>();
            if (types.is_empty()
                || child.possible_return_types_state != PossibleReturnTypesState::Complete)
                && let Some(return_type) = child.return_type.as_ref()
                && !types.contains(&return_type)
            {
                types.push(return_type);
            }
            types.into_iter().map(move |ty| (index, ty))
        })
        .collect::<Vec<_>>();
    let mut selected: Vec<(
        usize,
        &syntaxes::Property,
        usize,
        &syntaxes::TypeProperty,
        usize,
        &'static str,
    )> = Vec::new();
    for (property_source_index, property) in catalog
        .properties()
        .iter()
        .enumerate()
        .filter(|(_, property)| property.name == property_name)
    {
        for (source_child_index, source_type) in &source_types {
            let mut closest: Option<(usize, &syntaxes::TypeProperty, &'static str)> = None;
            for (related_type_index, option) in property.related_types.iter().enumerate() {
                if option.type_class.as_str() == source_type.as_str() {
                    closest = Some((related_type_index, option, "exact"));
                    break;
                }
                if catalog.is_class_assignable(source_type.as_str(), option.type_class.as_str())
                    && closest.is_none_or(|(_, current, _)| {
                        catalog.is_class_assignable(
                            option.type_class.as_str(),
                            current.type_class.as_str(),
                        )
                    })
                {
                    closest = Some((related_type_index, option, "assignable"));
                }
            }
            if let Some((related_type_index, option, match_kind)) = closest {
                selected.push((
                    property_source_index,
                    property,
                    related_type_index,
                    option,
                    *source_child_index,
                    match_kind,
                ));
            } else {
                selected.extend(
                    property
                        .related_types
                        .iter()
                        .enumerate()
                        .filter(|(_, option)| {
                            catalog.can_convert(source_type.as_str(), option.type_class.as_str())
                        })
                        .map(|(related_type_index, option)| {
                            (
                                property_source_index,
                                property,
                                related_type_index,
                                option,
                                *source_child_index,
                                "convertible",
                            )
                        }),
                );
            }
        }
    }
    selected.sort_by_key(
        |(property_index, _, related_index, _, child_index, match_kind)| {
            (
                *property_index,
                *related_index,
                *child_index,
                usize::from(*match_kind != "exact"),
            )
        },
    );
    selected.dedup_by_key(|(property_index, _, related_index, _, child_index, _)| {
        (*property_index, *related_index, *child_index)
    });
    selected
        .into_iter()
        .map(
            |(
                property_source_index,
                property,
                related_type_index,
                option,
                source_child_index,
                match_kind,
            )| {
                WitRegisteredExpressionPropertyOption {
                    source_record: catalog_record_ref(
                        catalog,
                        "Properties.json",
                        property_source_index,
                        property.registration_id.as_str(),
                    ),
                    property_source_index: property_source_index as u64,
                    related_type_index: related_type_index as u64,
                    source_child_index: source_child_index as u64,
                    match_kind: match_kind.to_owned(),
                    property_registration_id: property.registration_id.as_str().to_owned(),
                    property_name: property.name.clone(),
                    property_handler_class: property.handler_class.as_str().to_owned(),
                    property_addon_name: property.addon.name.clone(),
                    property_addon_version: property.addon.version.clone(),
                    input_class: option.type_class.as_str().to_owned(),
                    handler_class: option.handler_class.as_str().to_owned(),
                    handler_kind: property_handler_kind_name(&option.handler_kind).to_owned(),
                    provider_addon_name: option.provider.as_ref().map(|addon| addon.name.clone()),
                    provider_addon_version: option
                        .provider
                        .as_ref()
                        .map(|addon| addon.version.clone()),
                    type_code_name: option.type_code_name.as_str().to_owned(),
                    element_types: option
                        .element_types
                        .clone()
                        .unwrap_or_default()
                        .into_iter()
                        .map(|value| value.as_str().to_owned())
                        .collect(),
                    return_types: option
                        .possible_return_types
                        .as_ref()
                        .filter(|types| !types.is_empty())
                        .cloned()
                        .or_else(|| option.return_type.clone().map(|value| vec![value]))
                        .unwrap_or_default()
                        .into_iter()
                        .map(|value| value.as_str().to_owned())
                        .collect(),
                    supported_axes: option.supported_axes.clone().unwrap_or_default(),
                    accepted_changers: option
                        .accepted_changers
                        .as_ref()
                        .map(|modes| {
                            modes
                                .iter()
                                .map(|(mode, types)| WitAcceptedChangeMode {
                                    mode: catalog_change_mode_name(*mode).to_owned(),
                                    accepted_types: types
                                        .iter()
                                        .map(|value| value.as_str().to_owned())
                                        .collect(),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                    accepted_changers_state: option.expression_metadata_state.map(|state| {
                        match state {
                            ResolutionState::Resolved => WitMetadataResolutionState::Resolved,
                            ResolutionState::Unresolved => WitMetadataResolutionState::Unresolved,
                        }
                    }),
                    requires_source_expression_change: option.requires_source_expression_change,
                }
            },
        )
        .collect()
}

fn expression_child_semantic_role(node: &skript_parser::ExpressionNode) -> Option<&str> {
    node.metadata
        .get("semantic-role")
        .map(String::as_str)
        // Older CoreLibrary artifacts used target-class before roles were explicit.
        .or_else(|| {
            node.metadata
                .contains_key("target-class")
                .then_some("target-type")
        })
}

fn property_handler_kind_name(kind: &syntaxes::PropertyHandlerKind) -> &'static str {
    match kind {
        syntaxes::PropertyHandlerKind::Expression => "expression",
        syntaxes::PropertyHandlerKind::Condition => "condition",
        syntaxes::PropertyHandlerKind::Contains => "contains",
        syntaxes::PropertyHandlerKind::TypedValue => "typed-value",
        syntaxes::PropertyHandlerKind::Wxyz => "wxyz",
        syntaxes::PropertyHandlerKind::Custom => "custom",
    }
}

fn catalog_change_mode_name(mode: CatalogChangeMode) -> &'static str {
    match mode {
        CatalogChangeMode::Add => "ADD",
        CatalogChangeMode::Set => "SET",
        CatalogChangeMode::Remove => "REMOVE",
        CatalogChangeMode::RemoveAll => "REMOVE_ALL",
        CatalogChangeMode::Delete => "DELETE",
        CatalogChangeMode::Reset => "RESET",
    }
}

fn wit_event_value_option(value: &syntaxes::EventValue) -> wit_catalog_data::EventValueOption {
    wit_catalog_data::EventValueOption {
        event_class: value.event_class.as_str().to_owned(),
        value_class: value.value_class.as_str().to_owned(),
        time: value.time,
        registration_id: value.registration_id.as_str().to_owned(),
        patterns: value.patterns.clone().unwrap_or_default(),
        excludes: value
            .excludes
            .as_ref()
            .into_iter()
            .flatten()
            .map(|class| class.as_str().to_owned())
            .collect(),
        exclude_error_message: value.exclude_error_message.clone(),
        resolution_order: u64::try_from(value.resolution_order).unwrap_or(u64::MAX),
        registration_order: value
            .registration_order
            .and_then(|order| u64::try_from(order).ok()),
        accepted_changers: value
            .accepted_changers
            .as_ref()
            .into_iter()
            .flat_map(|modes| modes.iter())
            .map(|(mode, accepted_types)| WitAcceptedChangeMode {
                mode: catalog_change_mode_name(*mode).to_owned(),
                accepted_types: accepted_types
                    .iter()
                    .map(|class_name| class_name.as_str().to_owned())
                    .collect(),
            })
            .collect(),
        context_dependent: value.context_dependent,
        has_custom_input_validator: value.has_custom_input_validator,
        has_custom_event_validator: value.has_custom_event_validator,
    }
}

fn event_value_matches_input(catalog: &Catalog, value: &syntaxes::EventValue, input: &str) -> bool {
    if let Some(patterns) = value
        .patterns
        .as_ref()
        .filter(|patterns| !patterns.is_empty())
    {
        return patterns.iter().any(|pattern_source| {
            let Ok(parsed) =
                syntax_pattern_parser::syntax::parse(pattern_source, catalog.plural_rules())
            else {
                return false;
            };
            let source = MappedSource::identity(input);
            let Ok(input) = MatchInput::from_source(
                &source,
                ParserTextRange::new(0, source.virtual_source().len()),
            ) else {
                return false;
            };
            let candidate = PatternCandidate {
                kind: MatchSyntaxKind::Expression,
                definition_id: value.registration_id.as_str().to_owned(),
                registration_id: value.registration_id.as_str().to_owned(),
                priority: 0,
                registration_order: value.registration_order.unwrap_or(value.resolution_order),
                resolved_order: Some(value.resolution_order),
                patterns: vec![MatchPattern {
                    pattern_index: 0,
                    source: pattern_source,
                    parsed: &parsed,
                }],
            };
            run_pattern_matcher(
                input,
                &[candidate],
                &mut RejectTypeExpressions,
                &mut NoopPatternMatchHooks,
                PatternMatcherConfig::default(),
            )
            .is_ok_and(|matches| matches.selected.is_some())
        });
    }

    let plural = value.value_class.as_str().ends_with("[]");
    let component = value
        .value_class
        .as_str()
        .strip_suffix("[]")
        .unwrap_or(value.value_class.as_str());
    if let Some(type_info) = catalog
        .types()
        .find(|type_info| type_info.original_class.as_str() == component)
        && !type_info.user_input_patterns.is_empty()
    {
        return type_info.user_input_patterns.iter().any(|pattern| {
            Regex::new(&format!("(?i)^(?:{pattern})$"))
                .ok()
                .is_some_and(|pattern| pattern.is_match(input).unwrap_or(false))
        });
    }

    let simple_name = component
        .rsplit(['.', '$'])
        .next()
        .unwrap_or(component)
        .to_ascii_lowercase();
    let expected = if plural {
        catalog.plural_rules().to_plural(&simple_name)
    } else {
        simple_name
    };
    input.eq_ignore_ascii_case(&expected)
}

fn expression_node_registration(
    node: &ExpressionNode,
) -> (Option<String>, Option<String>, Option<u64>) {
    match &node.kind {
        ExpressionNodeKind::Registered {
            definition_id,
            registration_id,
            pattern_index,
        } => (
            Some(definition_id.clone()),
            Some(registration_id.clone()),
            u64::try_from(*pattern_index).ok(),
        ),
        _ => (None, None, None),
    }
}

fn expression_node_element_class(
    node: &skript_parser::ExpressionNode,
    catalog: Option<&Catalog>,
) -> Option<String> {
    let skript_parser::ExpressionNodeKind::Registered {
        registration_id, ..
    } = &node.kind
    else {
        return None;
    };
    catalog?
        .expressions()
        .find(|expression| expression.common.registration_id.as_str() == registration_id)
        .map(|expression| expression.common.element_class.as_str().to_owned())
}

fn expression_child_to_wit(
    child: &ExpressionNode,
    input: &str,
    catalog: Option<&Catalog>,
) -> WitRegisteredExpressionChild {
    let (kind, parser_id) = expression_node_identity(child);
    let (definition_id, registration_id, pattern_index) = expression_node_registration(child);
    WitRegisteredExpressionChild {
        text: child
            .span
            .local_range
            .slice(input)
            .unwrap_or_default()
            .to_owned(),
        kind: kind.to_owned(),
        parser_id: parser_id.map(str::to_owned),
        definition_id,
        registration_id,
        pattern_index,
        element_class: expression_node_element_class(child, catalog),
        return_type: child
            .return_type
            .as_ref()
            .map(|value| value.as_str().to_owned()),
        possible_return_types: child
            .possible_return_types
            .iter()
            .map(|value| value.as_str().to_owned())
            .collect(),
        possible_return_types_state: match child.possible_return_types_state {
            PossibleReturnTypesState::Complete => WitPossibleReturnTypesState::Complete,
            PossibleReturnTypesState::Partial => WitPossibleReturnTypesState::Partial,
            PossibleReturnTypesState::Unresolved => WitPossibleReturnTypesState::Unresolved,
        },
        multiplicity: child.multiplicity.map(multiplicity_to_wit),
        public_data: public_data::to_wit(&child.public_data),
        metadata: metadata_to_wit(&child.metadata),
    }
}

fn expression_node_identity(node: &ExpressionNode) -> (&'static str, Option<&str>) {
    match &node.kind {
        ExpressionNodeKind::Grouped => ("grouped-expression", None),
        ExpressionNodeKind::List { .. } => ("expression-list", None),
        ExpressionNodeKind::Registered { .. } => ("registered-expression", None),
        ExpressionNodeKind::Variable { parser_id } => ("variable", Some(parser_id)),
        ExpressionNodeKind::Literal { parser_id } => ("literal", Some(parser_id)),
        ExpressionNodeKind::Function { parser_id } => ("function", Some(parser_id)),
        ExpressionNodeKind::Arithmetic { .. } => ("arithmetic", None),
        ExpressionNodeKind::Custom { parser_id } => ("custom", Some(parser_id)),
    }
}

fn common_child_return_type(
    children: &[skript_parser::ExpressionNode],
    catalog: Option<&Catalog>,
) -> Option<String> {
    let catalog = catalog?;
    let types = children
        .iter()
        .map(|child| child.return_type.clone())
        .collect::<Option<Vec<_>>>()?;
    catalog
        .common_skript_class(&types)
        .map(|value| value.as_str().to_owned())
}

fn same_expression_request(left: &WitExpressionPayload, right: &WitExpressionPayload) -> bool {
    left.input == right.input
        && same_parse_context(&left.context, &right.context)
        && match (&left.active_type, &right.active_type) {
            (Some(left), Some(right)) => same_expression_type_option(left, right),
            (None, None) => true,
            _ => false,
        }
        && same_wit_range(&left.remaining, &right.remaining)
        && same_mapped_span(&left.span, &right.span)
        && left.expected_types.len() == right.expected_types.len()
        && left
            .expected_types
            .iter()
            .zip(&right.expected_types)
            .all(|(left, right)| left.class_name == right.class_name && left.plural == right.plural)
        && left.candidate_ends == right.candidate_ends
        && left.allow_literals == right.allow_literals
        && left.allow_expressions == right.allow_expressions
        && left.time == right.time
        && left.depth == right.depth
        && same_expression_type_options(&left.type_options, &right.type_options)
        && same_expression_literal_options(&left.literal_options, &right.literal_options)
}

fn same_expression_type_option(
    left: &WitExpressionTypeOption,
    right: &WitExpressionTypeOption,
) -> bool {
    same_catalog_record_ref(&left.source_record, &right.source_record)
        && left.definition_id == right.definition_id
        && left.registration_id == right.registration_id
        && left.addon_name == right.addon_name
        && left.addon_version == right.addon_version
        && left.code_name == right.code_name
        && left.class_name == right.class_name
        && left.parser_class == right.parser_class
        && left.type_parse_order == right.type_parse_order
        && left.before == right.before
        && left.after == right.after
        && left.singular == right.singular
        && left.plural == right.plural
        && left.user_input_patterns == right.user_input_patterns
        && left.has_parser == right.has_parser
        && left.parse_contexts == right.parse_contexts
        && left.has_supplier == right.has_supplier
}

fn same_expression_type_options(
    left: &[WitExpressionTypeOption],
    right: &[WitExpressionTypeOption],
) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| same_expression_type_option(left, right))
}

fn same_expression_literal_options(
    left: &[WitExpressionLiteralOption],
    right: &[WitExpressionLiteralOption],
) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            same_catalog_record_ref(&left.source_record, &right.source_record)
                && left.literal_index == right.literal_index
                && left.code_name == right.code_name
                && left.class_name == right.class_name
                && left.type_parse_order == right.type_parse_order
                && same_wit_range(&left.range, &right.range)
                && left.canonical_value == right.canonical_value
                && left.source == right.source
                && left.plural == right.plural
                && left.addon_name == right.addon_name
                && left.addon_version == right.addon_version
                && left.parser_class == right.parser_class
                && left.parse_contexts == right.parse_contexts
                && left.value_class == right.value_class
                && left.represented_class == right.represented_class
                && left.variable_name == right.variable_name
                && left.debug_text == right.debug_text
                && left.enum_constant == right.enum_constant
        })
}

fn same_catalog_record_ref(
    left: &Option<WitCatalogRecordRef>,
    right: &Option<WitCatalogRecordRef>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.source_digest == right.source_digest
                && left.snapshot_id == right.snapshot_id
                && left.document == right.document
                && left.index == right.index
                && left.byte_length == right.byte_length
        }
        _ => false,
    }
}

fn multiplicity_to_wit(value: Multiplicity) -> WitDynamicMultiplicity {
    match value {
        Multiplicity::Single => WitDynamicMultiplicity::Single,
        Multiplicity::Multiple => WitDynamicMultiplicity::Multiple,
        Multiplicity::Both => WitDynamicMultiplicity::Both,
    }
}

fn multiplicity_from_wit(value: WitDynamicMultiplicity) -> Multiplicity {
    match value {
        WitDynamicMultiplicity::Single => Multiplicity::Single,
        WitDynamicMultiplicity::Multiple => Multiplicity::Multiple,
        WitDynamicMultiplicity::Both => Multiplicity::Both,
    }
}

fn metadata_entries(entries: Vec<WitMetadataEntry>) -> Result<BTreeMap<String, String>, String> {
    let mut metadata = BTreeMap::new();
    for entry in entries {
        let WitMetadataEntry {
            owner_component_id,
            key,
            value,
        } = entry;
        let key = owner_component_id.map_or(key.clone(), |owner| format!("{owner}/{key}"));
        if metadata.insert(key.clone(), value).is_some() {
            return Err(format!("metadata key {key} is repeated"));
        }
    }
    Ok(metadata)
}

fn same_registered_expression_identity(
    left: &WitRegisteredExpressionPayload,
    right: &WitRegisteredExpressionPayload,
) -> bool {
    left.input == right.input
        && same_parse_context(&left.context, &right.context)
        && left.definition_id == right.definition_id
        && left.registration_id == right.registration_id
        && left.element_class == right.element_class
        && left.related_property == right.related_property
        && left.pattern_index == right.pattern_index
        && left.pattern == right.pattern
        && same_mapped_span(&left.span, &right.span)
        && left.expected_types.len() == right.expected_types.len()
        && left
            .expected_types
            .iter()
            .zip(&right.expected_types)
            .all(|(left, right)| left.class_name == right.class_name && left.plural == right.plural)
        && left.declared_return_type == right.declared_return_type
        && left.declared_multiplicity == right.declared_multiplicity
        && left.return_type_state == right.return_type_state
        && left.possible_return_types == right.possible_return_types
        && left.possible_return_types_state == right.possible_return_types_state
        && left.time == right.time
        && left.regex_captures == right.regex_captures
        && left.tags.len() == right.tags.len()
        && left
            .tags
            .iter()
            .zip(&right.tags)
            .all(|(left, right)| left.value == right.value && left.implicit == right.implicit)
        && left.mark == right.mark
        && same_registered_expression_children(&left.children, &right.children)
        && same_parsed_captures(&left.parsed_captures, &right.parsed_captures)
        && left.common_child_return_type == right.common_child_return_type
        && same_expression_type_options(&left.type_options, &right.type_options)
        && left.property_options.len() == right.property_options.len()
        && left
            .property_options
            .iter()
            .zip(&right.property_options)
            .all(|(left, right)| {
                same_catalog_record_ref(&left.source_record, &right.source_record)
                    && left.property_source_index == right.property_source_index
                    && left.related_type_index == right.related_type_index
                    && left.source_child_index == right.source_child_index
                    && left.match_kind == right.match_kind
                    && left.property_registration_id == right.property_registration_id
                    && left.property_name == right.property_name
                    && left.property_handler_class == right.property_handler_class
                    && left.property_addon_name == right.property_addon_name
                    && left.property_addon_version == right.property_addon_version
                    && left.input_class == right.input_class
                    && left.handler_class == right.handler_class
                    && left.handler_kind == right.handler_kind
                    && left.provider_addon_name == right.provider_addon_name
                    && left.provider_addon_version == right.provider_addon_version
                    && left.type_code_name == right.type_code_name
                    && left.element_types == right.element_types
                    && left.return_types == right.return_types
                    && left.supported_axes == right.supported_axes
                    && same_accepted_changers(&left.accepted_changers, &right.accepted_changers)
                    && left.accepted_changers_state == right.accepted_changers_state
                    && left.requires_source_expression_change
                        == right.requires_source_expression_change
            })
}

fn validate_selected_property_options(
    payload: &WitRegisteredExpressionPayload,
) -> Result<(), String> {
    let mut seen = HashSet::new();
    let mut source_child_index = None;
    for index in &payload.selected_property_option_indices {
        let index = usize::try_from(*index)
            .map_err(|_| "selected Property option index does not fit host memory".to_owned())?;
        if index >= payload.property_options.len() {
            return Err(format!(
                "selected Property option index {index} is outside the {}-option payload",
                payload.property_options.len()
            ));
        }
        if !seen.insert(index) {
            return Err(format!(
                "selected Property option index {index} is repeated"
            ));
        }
        let option = &payload.property_options[index];
        if source_child_index
            .replace(option.source_child_index)
            .is_some_and(|previous| previous != option.source_child_index)
        {
            return Err(
                "selected Property options refer to different source Expressions".to_owned(),
            );
        }
    }
    Ok(())
}

fn same_accepted_changers(left: &[WitAcceptedChangeMode], right: &[WitAcceptedChangeMode]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.mode == right.mode && left.accepted_types == right.accepted_types
        })
}

fn same_matching_path(left: &[MatchingPathSegment], right: &[MatchingPathSegment]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| match (left, right) {
                (MatchingPathSegment::Element(left), MatchingPathSegment::Element(right))
                | (MatchingPathSegment::Branch(left), MatchingPathSegment::Branch(right)) => {
                    left == right
                }
                _ => false,
            })
}

fn same_optional_wit_range(left: Option<&WitTextRange>, right: Option<&WitTextRange>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => same_wit_range(left, right),
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

fn same_wit_range(left: &WitTextRange, right: &WitTextRange) -> bool {
    left.start == right.start && left.end == right.end
}

fn same_mapped_span(left: &MappedSpan, right: &MappedSpan) -> bool {
    same_wit_range(&left.virtual_range, &right.virtual_range)
        && left.origins.len() == right.origins.len()
        && left
            .origins
            .iter()
            .zip(&right.origins)
            .all(|(left, right)| {
                same_wit_range(&left.original_range, &right.original_range)
                    && mem::discriminant(&left.kind) == mem::discriminant(&right.kind)
                    && left.expansion == right.expansion
            })
}

fn matching_control(
    status: MatchingStatus,
    range: ParserTextRange,
    failure_reason: Option<String>,
) -> PatternHookControl {
    match status {
        MatchingStatus::Pending => PatternHookControl::Continue,
        MatchingStatus::Matched => PatternHookControl::Match(range),
        MatchingStatus::Failed => PatternHookControl::Fail(
            failure_reason.unwrap_or_else(|| "matching hook rejected the scope".to_owned()),
        ),
    }
}

fn wit_syntax_kind(kind: MatchSyntaxKind) -> SyntaxKind {
    match kind {
        MatchSyntaxKind::Event => SyntaxKind::Event,
        MatchSyntaxKind::Condition => SyntaxKind::Condition,
        MatchSyntaxKind::Effect => SyntaxKind::Effect,
        MatchSyntaxKind::Expression => SyntaxKind::Expression,
        MatchSyntaxKind::Type => SyntaxKind::Type,
        MatchSyntaxKind::Function => SyntaxKind::Function,
        MatchSyntaxKind::Section => SyntaxKind::Section,
        MatchSyntaxKind::Structure => SyntaxKind::Structure,
    }
}

fn registered_capture_bindings(
    components: &[ComponentEntry],
    syntax: RegisteredSyntaxIdentity<'_>,
) -> Result<Vec<RegisteredCaptureBinding>, String> {
    let mut bindings = BTreeMap::<usize, RegisteredCaptureBinding>::new();
    for component in components
        .iter()
        .filter(|component| !component.disabled && !component.unloaded)
    {
        for handler in component
            .manifest
            .registered_syntax_handlers
            .iter()
            .filter(|handler| registered_handler_matches(component, handler, syntax))
        {
            for binding in &handler.capture_parsers {
                let capture_index = usize::try_from(binding.capture_index).map_err(|_| {
                    format!(
                        "capture index {} does not fit this platform",
                        binding.capture_index
                    )
                })?;
                let binding = RegisteredCaptureBinding {
                    capture_index,
                    parser_id: binding.parser_id.clone(),
                    required: binding.required,
                    options: binding
                        .options
                        .iter()
                        .map(|entry| (entry.key.clone(), entry.value.clone()))
                        .collect(),
                };
                if let Some(existing) = bindings.get(&capture_index) {
                    if existing != &binding {
                        return Err(format!(
                            "registered syntax {} pattern {} has conflicting capture parsers at index {}",
                            syntax.registration_id,
                            syntax
                                .pattern_index
                                .map_or_else(|| "*".to_owned(), |index| index.to_string()),
                            capture_index
                        ));
                    }
                } else {
                    bindings.insert(capture_index, binding);
                }
            }
        }
    }
    Ok(bindings.into_values().collect())
}

fn has_registered_syntax_handler(
    components: &[ComponentEntry],
    syntax: RegisteredSyntaxIdentity<'_>,
) -> bool {
    components
        .iter()
        .filter(|component| !component.disabled && !component.unloaded)
        .any(|component| {
            component
                .manifest
                .registered_syntax_handlers
                .iter()
                .any(|handler| registered_handler_matches(component, handler, syntax))
        })
}

fn registered_handler_requires_context(
    components: &[ComponentEntry],
    syntax: RegisteredSyntaxIdentity<'_>,
    requirement: &str,
) -> bool {
    components
        .iter()
        .filter(|component| !component.disabled && !component.unloaded)
        .any(|component| {
            component
                .manifest
                .registered_syntax_handlers
                .iter()
                .filter(|handler| registered_handler_matches(component, handler, syntax))
                .any(|handler| {
                    handler
                        .context_requirements
                        .iter()
                        .any(|declared| declared == requirement)
                })
        })
}

fn registered_handler_matches(
    component: &ComponentEntry,
    handler: &crate::bindings::nlaocs::skript_parser_addon::types::RegisteredSyntaxHandler,
    syntax: RegisteredSyntaxIdentity<'_>,
) -> bool {
    if catalog_syntax_kind(handler.kind) != syntax.kind {
        return false;
    }
    if !handler.pattern_indices.is_empty()
        && !syntax.pattern_index.is_some_and(|index| {
            handler
                .pattern_indices
                .contains(&u64::try_from(index).unwrap_or(u64::MAX))
        })
    {
        return false;
    }
    if !handler.pattern_sources.is_empty()
        && !syntax.pattern_source.is_some_and(|source| {
            handler
                .pattern_sources
                .iter()
                .any(|expected| expected == source)
        })
    {
        return false;
    }
    if !handler.required_tags.is_empty()
        && !syntax.tags.is_some_and(|tags| {
            handler
                .required_tags
                .iter()
                .all(|required| tags.iter().any(|tag| tag.value == *required))
        })
    {
        return false;
    }
    if syntax.tags.is_some_and(|tags| {
        handler
            .forbidden_tags
            .iter()
            .any(|forbidden| tags.iter().any(|tag| tag.value == *forbidden))
    }) {
        return false;
    }
    if !handler.marks.is_empty()
        && !syntax
            .mark
            .is_some_and(|mark| handler.marks.contains(&mark))
    {
        return false;
    }
    let binding = component
        .registered_handler_bindings
        .iter()
        .find(|binding| binding.handler_id == handler.handler_id);
    handler.targets.iter().any(|target| match target {
        RegisteredSyntaxHandlerTarget::DynamicHandler(handler_id) => {
            syntax.dynamic_handler == Some(handler_id.as_str())
        }
        RegisteredSyntaxHandlerTarget::Definition(_) => binding.is_some_and(|binding| {
            binding
                .definition_ids
                .iter()
                .any(|id| id == syntax.definition_id)
        }),
        RegisteredSyntaxHandlerTarget::Registration(_)
        | RegisteredSyntaxHandlerTarget::ParserClass(_)
        | RegisteredSyntaxHandlerTarget::ClassSuffix(_)
        | RegisteredSyntaxHandlerTarget::SuperClass(_) => binding.is_some_and(|binding| {
            binding
                .registration_ids
                .iter()
                .any(|id| id == syntax.registration_id)
        }),
    })
}

fn registered_syntax_identity<'a>(
    catalog: &'a Catalog,
    kind: MatchSyntaxKind,
    registration_id: &str,
) -> Option<RegisteredSyntaxIdentity<'a>> {
    let kind = catalog_match_syntax_kind(kind);
    let syntaxes = catalog.syntax_by_registration_id(registration_id);
    let mut matches = syntaxes.iter().filter(|syntax| syntax.kind() == kind);
    let syntax = *matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(RegisteredSyntaxIdentity {
        kind,
        definition_id: syntax.definition_id().as_str(),
        registration_id: syntax.registration_id().as_str(),
        pattern_index: None,
        pattern_source: None,
        tags: None,
        mark: None,
        dynamic_handler: None,
    })
}

fn catalog_match_syntax_kind(kind: MatchSyntaxKind) -> CatalogSyntaxKind {
    match kind {
        MatchSyntaxKind::Event => CatalogSyntaxKind::Event,
        MatchSyntaxKind::Condition => CatalogSyntaxKind::Condition,
        MatchSyntaxKind::Effect => CatalogSyntaxKind::Effect,
        MatchSyntaxKind::Expression => CatalogSyntaxKind::Expression,
        MatchSyntaxKind::Type => CatalogSyntaxKind::Type,
        MatchSyntaxKind::Function => CatalogSyntaxKind::Function,
        MatchSyntaxKind::Section => CatalogSyntaxKind::Section,
        MatchSyntaxKind::Structure => CatalogSyntaxKind::Structure,
    }
}
struct EpochTicker {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl EpochTicker {
    fn start(engine: Engine, tick: Duration) -> Result<Self, HostError> {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("parser-wasm-epoch".to_owned())
            .spawn(move || {
                while !thread_stop.load(Ordering::Acquire) {
                    thread::sleep(tick);
                    engine.increment_epoch();
                }
            })
            .map_err(|error| HostError::Engine {
                message: error.to_string(),
            })?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

// Shares immutable Wasmtime infrastructure while each ParserHost keeps its own
// Store, component instances, registries, and transactional state.
struct SharedHostRuntime {
    engine: Engine,
    linker: Linker<StoreData>,
    components: Mutex<HashMap<[u8; 32], Component>>,
    _epoch_ticker: EpochTicker,
}

impl SharedHostRuntime {
    fn new(epoch_tick: Duration) -> Result<Self, HostError> {
        let mut wasmtime_config = Config::new();
        wasmtime_config.wasm_component_model(true);
        wasmtime_config.consume_fuel(true);
        wasmtime_config.epoch_interruption(true);
        let engine = Engine::new(&wasmtime_config).map_err(|error| HostError::Engine {
            message: error.to_string(),
        })?;
        let ticker = EpochTicker::start(engine.clone(), epoch_tick)?;
        let mut linker = Linker::new(&engine);
        ParserAddon::add_to_linker::<_, HasSelf<_>>(&mut linker, |data: &mut StoreData| data)
            .map_err(|error| HostError::Engine {
                message: format!("failed to register parser addon host imports: {error}"),
            })?;
        Ok(Self {
            engine,
            linker,
            components: Mutex::new(HashMap::new()),
            _epoch_ticker: ticker,
        })
    }

    fn component(&self, bytes: &[u8], component_id: &str) -> Result<Component, HostError> {
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        let mut components = self
            .components
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(component) = components.get(&digest) {
            return Ok(component.clone());
        }
        let component = Component::new(&self.engine, bytes)
            .map_err(|error| classify_component_error(component_id.to_owned(), "compile", error))?;
        components.insert(digest, component.clone());
        Ok(component)
    }
}

static SHARED_HOST_RUNTIMES: LazyLock<Mutex<HashMap<Duration, Arc<SharedHostRuntime>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn shared_host_runtime(epoch_tick: Duration) -> Result<Arc<SharedHostRuntime>, HostError> {
    let mut runtimes = SHARED_HOST_RUNTIMES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(runtime) = runtimes.get(&epoch_tick) {
        return Ok(Arc::clone(runtime));
    }
    let runtime = Arc::new(SharedHostRuntime::new(epoch_tick)?);
    runtimes.insert(epoch_tick, Arc::clone(&runtime));
    Ok(runtime)
}

/// Wasmtime component registry and orchestrator for all parser extension stages.
///
/// Construction loads and negotiates the mandatory CoreLibrary first. Optional
/// addon components are then registered in deterministic load order. Parsing
/// work may use convenience methods that commit automatically or a caller-owned
/// [ParseTransaction] that spans Text macros, Tree macros, dynamic syntax, and
/// matching as one atomic document revision.
///
/// # Examples
///
/// Library code can accept the bundled CoreLibrary bytes from the executable
/// crate and inspect the mandatory first component:
///
/// ~~~no_run
/// use parser_wasm::{HostConfig, HostError, ParserHost};
///
/// fn create_host(core_library: &[u8]) -> Result<ParserHost, HostError> {
///     let host = ParserHost::new(core_library, HostConfig::default())?;
///     let components = host.components();
///
///     assert_eq!(components[0].component_id, "nlaocs.core-library");
///     assert!(!components[0].disabled);
///     Ok(host)
/// }
/// # let _ = create_host;
/// ~~~
pub struct ParserHost {
    runtime: Arc<SharedHostRuntime>,
    config: HostConfig,
    state_store: StateStore,
    dynamic_syntax_registry: Option<DynamicSyntaxRegistry>,
    capabilities: Vec<Capability>,
    components: Vec<ComponentEntry>,
    registry: SubscriptionRegistry,
    type_user_input_matchers: Arc<[TypeUserInputMatcher]>,
    active_parse_requests: Vec<ParseRequestKey>,
    next_parse_result_token: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParseRequestKey {
    parser_id: String,
    input: String,
    expected_types: Vec<(String, bool)>,
    options: Vec<(String, String)>,
    syntax_context: u64,
}

impl ParseRequestKey {
    fn new(request: &ParseRequest, context: &InvocationContext) -> Self {
        Self {
            parser_id: request.parser_id.clone(),
            input: request.input.clone(),
            expected_types: request
                .expected_types
                .iter()
                .map(|expected| (expected.class_name.clone(), expected.plural))
                .collect(),
            options: request
                .options
                .iter()
                .map(|entry| (entry.key.clone(), entry.value.clone()))
                .collect(),
            syntax_context: context.syntax_context,
        }
    }
}

impl ParserHost {
    /// Creates a host and synchronously loads the mandatory CoreLibrary component.
    pub fn new(core_library: &[u8], mut config: HostConfig) -> Result<Self, HostError> {
        if core_library.is_empty() {
            return Err(HostError::CoreLibraryMissing);
        }
        config.inherit_catalog_runtime();
        config.validate()?;
        let state_store = StateStore::new(config.state_store.clone())?;
        let catalog_source_available = config
            .syntax_catalog
            .as_ref()
            .is_some_and(|catalog| catalog.source().is_some());
        let dynamic_syntax_registry = config
            .syntax_catalog
            .clone()
            .map(DynamicSyntaxRegistry::new);
        let type_user_input_matchers =
            build_type_user_input_matchers(config.syntax_catalog.as_deref());

        let runtime = shared_host_runtime(config.epoch_tick)?;
        let capabilities = configured_host_capabilities(
            dynamic_syntax_registry.is_some(),
            catalog_source_available,
        );
        let mut host = Self {
            runtime,
            config,
            state_store,
            dynamic_syntax_registry,
            capabilities,
            components: Vec::new(),
            registry: SubscriptionRegistry::default(),
            type_user_input_matchers,
            active_parse_requests: Vec::new(),
            next_parse_result_token: 1,
        };
        host.load_component(core_library, true)?;
        Ok(host)
    }

    /// Starts matching StateStore and dynamic-syntax overlays for a document revision.
    pub fn begin_parse(
        &self,
        project_uri: &str,
        document_id: &str,
        document_revision: u64,
    ) -> Result<ParseTransaction, HostError> {
        let transaction =
            self.state_store
                .begin_parse(project_uri, document_id, document_revision)?;
        if let Some(registry) = &self.dynamic_syntax_registry
            && let Err(error) = registry.begin_document(document_id, document_revision)
        {
            let _ = transaction.cancel();
            return Err(error.into());
        }
        Ok(transaction)
    }

    /// Compiles, negotiates, initializes, and registers an optional addon Component.
    pub fn load_addon(&mut self, component: &[u8]) -> Result<ComponentInfo, HostError> {
        self.load_component(component, false)
    }

    /// Returns loaded components in deterministic load order, including disabled entries.
    pub fn components(&self) -> Vec<ComponentInfo> {
        self.components
            .iter()
            .map(|entry| ComponentInfo {
                component_id: entry.manifest.component_id.clone(),
                component_version: entry.manifest.component_version.clone(),
                load_order: entry.load_order,
                disabled: entry.disabled || entry.unloaded,
            })
            .collect()
    }

    /// Freezes the current revision into a deterministic immutable candidate snapshot.
    pub fn dynamic_syntax_snapshot(
        &self,
        transaction: &ParseTransaction,
    ) -> Result<DynamicSyntaxSnapshot, HostError> {
        let registry = self
            .dynamic_syntax_registry
            .as_ref()
            .ok_or(HostError::DynamicSyntaxUnavailable)?;
        Ok(registry.freeze(
            &transaction.document_id()?,
            transaction.document_revision()?,
        )?)
    }

    /// Runs native registered-pattern matching with transactional WASM hooks.
    pub fn match_patterns_in_parse<R: TypeExpressionResolver>(
        &mut self,
        transaction: &ParseTransaction,
        context: InvocationContext,
        input: MatchInput<'_>,
        candidates: &[PatternCandidate<'_>],
        resolver: &mut R,
        config: PatternMatcherConfig,
    ) -> Result<WasmPatternMatchResult, HostError> {
        let document_id = transaction.document_id()?;
        let document_revision = transaction.document_revision()?;
        if document_id != context.document_id || document_revision != context.document_revision {
            return Err(StateError::InvalidInput {
                message: format!(
                    "matcher context {}@{} does not match parse transaction {}@{}",
                    context.document_id, context.document_revision, document_id, document_revision,
                ),
            }
            .into());
        }

        let base = transaction.savepoint()?;
        let input_text = input.text().to_owned();
        let matching_hooks_registered = self.registry.has_matching_hooks();
        let dynamic_snapshot = self
            .dynamic_syntax_registry
            .as_ref()
            .map(|_| self.dynamic_syntax_snapshot(transaction))
            .transpose()?;
        let mut hooks = WasmPatternHooks {
            host: self,
            transaction,
            dynamic_snapshot: dynamic_snapshot.as_ref(),
            matching_hooks_registered,
            context,
            input: input_text,
            frames: Vec::new(),
            scope_frames: Vec::new(),
            branch_states: Vec::new(),
            last_candidate: None,
            effects: empty_effects(),
            calls: Vec::new(),
            failures: Vec::new(),
        };
        let result = run_pattern_matcher(input, candidates, resolver, &mut hooks, config);
        let (effects, calls, failures) = hooks.into_parts();
        match result {
            Ok(matches) => Ok(WasmPatternMatchResult {
                matches,
                effects,
                calls,
                failures,
            }),
            Err(error) => {
                transaction.rollback_to(&base)?;
                Err(error.into())
            }
        }
    }
    /// Parses one Expression with SSG registrations and WASM leaf parsers.
    ///
    /// The caller owns the surrounding parse transaction. A no-match or parser
    /// failure restores the transaction to its entry savepoint; accepted
    /// candidates retain only state selected by matching/Expression hooks.
    pub fn parse_expression_in_parse(
        &mut self,
        transaction: &ParseTransaction,
        context: InvocationContext,
        request: ExpressionParseRequest<'_>,
        mut config: ExpressionParserConfig,
    ) -> Result<WasmExpressionParseResult, HostError> {
        let document_id = transaction.document_id()?;
        let document_revision = transaction.document_revision()?;
        if document_id != context.document_id || document_revision != context.document_revision {
            return Err(StateError::InvalidInput {
                message: format!(
                    "Expression context {}@{} does not match parse transaction {}@{}",
                    context.document_id, context.document_revision, document_id, document_revision,
                ),
            }
            .into());
        }
        if request.context.syntax_context != context.syntax_context {
            return Err(StateError::InvalidInput {
                message: format!(
                    "Expression syntax context {} does not match invocation context {}",
                    request.context.syntax_context, context.syntax_context,
                ),
            }
            .into());
        }

        config.function =
            function_policy_for_runtime(self.config.runtime_profile.skript_version.as_deref());
        let catalog = self
            .config
            .syntax_catalog
            .clone()
            .ok_or(HostError::SyntaxCatalogUnavailable)?;
        let dynamic_snapshot = self.dynamic_syntax_snapshot(transaction)?;
        let base = transaction.savepoint()?;
        let input_text = request.source.virtual_source().to_owned();
        let matching_hooks_registered = self.registry.has_matching_hooks();
        let hooks = WasmPatternHooks {
            host: self,
            transaction,
            dynamic_snapshot: Some(&dynamic_snapshot),
            matching_hooks_registered,
            context,
            input: input_text,
            frames: Vec::new(),
            scope_frames: Vec::new(),
            branch_states: Vec::new(),
            last_candidate: None,
            effects: empty_effects(),
            calls: Vec::new(),
            failures: Vec::new(),
        };
        let mut environment = WasmExpressionEnvironment {
            hooks,
            pending_leaf: None,
            pending_registered: None,
            expression_candidates: Vec::new(),
            semantic_candidates: Vec::new(),
            function_registry: None,
        };
        let result = run_expression_parser(
            catalog.as_ref(),
            Some(&dynamic_snapshot),
            request,
            &mut environment,
            config,
        );
        let (mut effects, mut calls, failures) = environment.into_parts();
        match result {
            Ok(matches) => {
                if matches.selected.is_none() {
                    promote_semantic_diagnostics(
                        &mut effects,
                        matches
                            .failure
                            .as_ref()
                            .and_then(|failure| failure.trace.as_ref()),
                    );
                    transaction.rollback_to(&base)?;
                    retain_diagnostics_only(&mut effects, &mut calls);
                }
                Ok(WasmExpressionParseResult {
                    matches,
                    effects,
                    calls,
                    failures,
                })
            }
            Err(error) => {
                transaction.rollback_to(&base)?;
                Err(error.into())
            }
        }
    }

    /// Parses one Condition with SSG registrations and recursive Expression hooks.
    ///
    /// The caller owns the parse transaction. A no-match or parser failure
    /// restores the transaction to its entry savepoint, while a selected
    /// Condition keeps only state belonging to accepted nested candidates.
    pub fn parse_condition_in_parse(
        &mut self,
        transaction: &ParseTransaction,
        context: InvocationContext,
        request: ConditionParseRequest<'_>,
        mut config: ConditionParserConfig,
    ) -> Result<WasmConditionParseResult, HostError> {
        let document_id = transaction.document_id()?;
        let document_revision = transaction.document_revision()?;
        if document_id != context.document_id || document_revision != context.document_revision {
            return Err(StateError::InvalidInput {
                message: format!(
                    "Condition context {}@{} does not match parse transaction {}@{}",
                    context.document_id, context.document_revision, document_id, document_revision,
                ),
            }
            .into());
        }
        if request.context.syntax_context != context.syntax_context {
            return Err(StateError::InvalidInput {
                message: format!(
                    "Condition syntax context {} does not match invocation context {}",
                    request.context.syntax_context, context.syntax_context,
                ),
            }
            .into());
        }

        config.expression.function =
            function_policy_for_runtime(self.config.runtime_profile.skript_version.as_deref());
        let catalog = self
            .config
            .syntax_catalog
            .clone()
            .ok_or(HostError::SyntaxCatalogUnavailable)?;
        let dynamic_snapshot = self.dynamic_syntax_snapshot(transaction)?;
        let base = transaction.savepoint()?;
        let input_text = request.source.virtual_source().to_owned();
        let matching_hooks_registered = self.registry.has_matching_hooks();
        let hooks = WasmPatternHooks {
            host: self,
            transaction,
            dynamic_snapshot: Some(&dynamic_snapshot),
            matching_hooks_registered,
            context,
            input: input_text,
            frames: Vec::new(),
            scope_frames: Vec::new(),
            branch_states: Vec::new(),
            last_candidate: None,
            effects: empty_effects(),
            calls: Vec::new(),
            failures: Vec::new(),
        };
        let mut environment = WasmExpressionEnvironment {
            hooks,
            pending_leaf: None,
            pending_registered: None,
            expression_candidates: Vec::new(),
            semantic_candidates: Vec::new(),
            function_registry: None,
        };
        let result = run_condition_parser(
            catalog.as_ref(),
            Some(&dynamic_snapshot),
            request,
            &mut environment,
            config,
        );
        let (mut effects, mut calls, failures) = environment.into_parts();
        match result {
            Ok(matches) => {
                if matches.selected.is_none() {
                    promote_semantic_diagnostics(
                        &mut effects,
                        matches
                            .unknown
                            .as_ref()
                            .and_then(|unknown| unknown.failure.as_ref()),
                    );
                    transaction.rollback_to(&base)?;
                    retain_diagnostics_only(&mut effects, &mut calls);
                }
                Ok(WasmConditionParseResult {
                    matches,
                    effects,
                    calls,
                    failures,
                })
            }
            Err(error) => {
                transaction.rollback_to(&base)?;
                Err(error.into())
            }
        }
    }

    /// Parses one Section header and recursively claims its RawTree children.
    pub fn parse_section_in_parse(
        &mut self,
        transaction: &ParseTransaction,
        context: InvocationContext,
        request: SectionParseRequest<'_>,
        mut config: SectionParserConfig,
    ) -> Result<WasmSectionParseResult, HostError> {
        let document_id = transaction.document_id()?;
        let document_revision = transaction.document_revision()?;
        if document_id != context.document_id || document_revision != context.document_revision {
            return Err(StateError::InvalidInput {
                message: format!(
                    "Section context {}@{} does not match parse transaction {}@{}",
                    context.document_id, context.document_revision, document_id, document_revision,
                ),
            }
            .into());
        }
        let node_context = u64::from(request.node.syntax_context.get());
        if node_context != context.syntax_context
            || request.context.syntax_context != context.syntax_context
        {
            return Err(StateError::InvalidInput {
                message: format!(
                    "Section syntax contexts node={node_context}, parser={}, invocation={} do not match",
                    request.context.syntax_context, context.syntax_context,
                ),
            }
            .into());
        }

        config.expression.function =
            function_policy_for_runtime(self.config.runtime_profile.skript_version.as_deref());
        let catalog = self
            .config
            .syntax_catalog
            .clone()
            .ok_or(HostError::SyntaxCatalogUnavailable)?;
        let dynamic_snapshot = self.dynamic_syntax_snapshot(transaction)?;
        let base = transaction.savepoint()?;
        let matching_hooks_registered = self.registry.has_matching_hooks();
        let hooks = WasmPatternHooks {
            host: self,
            transaction,
            dynamic_snapshot: Some(&dynamic_snapshot),
            matching_hooks_registered,
            context,
            input: request.source.virtual_source().to_owned(),
            frames: Vec::new(),
            scope_frames: Vec::new(),
            branch_states: Vec::new(),
            last_candidate: None,
            effects: empty_effects(),
            calls: Vec::new(),
            failures: Vec::new(),
        };
        let mut environment = WasmExpressionEnvironment {
            hooks,
            pending_leaf: None,
            pending_registered: None,
            expression_candidates: Vec::new(),
            semantic_candidates: Vec::new(),
            function_registry: None,
        };
        let parsed = run_section_parser(
            catalog.as_ref(),
            Some(&dynamic_snapshot),
            request,
            &mut environment,
            config,
        );
        let (mut effects, mut calls, failures) = environment.into_parts();
        match parsed {
            Ok(matches) => {
                if matches.selected.is_none() {
                    promote_semantic_diagnostics(
                        &mut effects,
                        matches
                            .unknown
                            .as_ref()
                            .and_then(|unknown| unknown.failure.as_ref()),
                    );
                    transaction.rollback_to(&base)?;
                    retain_diagnostics_only(&mut effects, &mut calls);
                }
                Ok(WasmSectionParseResult {
                    matches,
                    effects,
                    calls,
                    failures,
                })
            }
            Err(error) => {
                transaction.rollback_to(&base)?;
                Err(error.into())
            }
        }
    }

    /// Parses every top-level RawTree root as a Structure in two lifecycle passes.
    pub fn parse_structures_in_parse(
        &mut self,
        transaction: &ParseTransaction,
        context: InvocationContext,
        request: StructureParseRequest<'_>,
        mut config: StructureParserConfig,
    ) -> Result<WasmStructureParseResult, HostError> {
        let document_id = transaction.document_id()?;
        let document_revision = transaction.document_revision()?;
        if document_id != context.document_id || document_revision != context.document_revision {
            return Err(StateError::InvalidInput {
                message: format!(
                    "Structure context {}@{} does not match parse transaction {}@{}",
                    context.document_id, context.document_revision, document_id, document_revision,
                ),
            }
            .into());
        }
        if request.context.syntax_context != context.syntax_context {
            return Err(StateError::InvalidInput {
                message: format!(
                    "Structure parser context {} does not match invocation context {}",
                    request.context.syntax_context, context.syntax_context,
                ),
            }
            .into());
        }

        config.expression.function =
            function_policy_for_runtime(self.config.runtime_profile.skript_version.as_deref());
        let catalog = self
            .config
            .syntax_catalog
            .clone()
            .ok_or(HostError::SyntaxCatalogUnavailable)?;
        let dynamic_snapshot = self.dynamic_syntax_snapshot(transaction)?;
        let base = transaction.savepoint()?;
        let mut function_registry = FunctionRegistryTransaction::new(
            context.document_id.clone(),
            document_revision,
            function_policy_for_runtime(self.config.runtime_profile.skript_version.as_deref()),
        );
        let matching_hooks_registered = self.registry.has_matching_hooks();
        let hooks = WasmPatternHooks {
            host: self,
            transaction,
            dynamic_snapshot: Some(&dynamic_snapshot),
            matching_hooks_registered,
            context,
            input: request.source.virtual_source().to_owned(),
            frames: Vec::new(),
            scope_frames: Vec::new(),
            branch_states: Vec::new(),
            last_candidate: None,
            effects: empty_effects(),
            calls: Vec::new(),
            failures: Vec::new(),
        };
        let mut environment = WasmExpressionEnvironment {
            hooks,
            pending_leaf: None,
            pending_registered: None,
            expression_candidates: Vec::new(),
            semantic_candidates: Vec::new(),
            function_registry: Some(&mut function_registry),
        };
        let parsed = run_structure_parser(
            catalog.as_ref(),
            Some(&dynamic_snapshot),
            request,
            &mut environment,
            config,
        );
        let (mut effects, calls, failures) = environment.into_parts();
        match parsed {
            Ok(document) => {
                for root in &document.roots {
                    if let StructureDocumentNode::Structure(matches) = root
                        && matches.selected.is_none()
                    {
                        promote_semantic_diagnostics(
                            &mut effects,
                            matches
                                .unknown
                                .as_ref()
                                .and_then(|unknown| unknown.failure.as_ref()),
                        );
                    }
                }
                let functions = function_registry.freeze()?;
                Ok(WasmStructureParseResult {
                    document,
                    functions,
                    effects,
                    calls,
                    failures,
                })
            }
            Err(error) => {
                transaction.rollback_to(&base)?;
                Err(error.into())
            }
        }
    }

    /// Parses one lossless Simple node as an Effect with nested Expression and WASM hooks.
    ///
    /// The caller owns the parse transaction. State written while exploring
    /// rejected candidates, a rejected Effect hook, or an unknown node is
    /// restored to the entry savepoint. Only the selected candidate and its
    /// accepted Expression/Effect hooks retain transactional changes.
    pub fn parse_effect_in_parse(
        &mut self,
        transaction: &ParseTransaction,
        context: InvocationContext,
        request: EffectParseRequest<'_>,
        mut config: EffectParserConfig,
    ) -> Result<WasmEffectParseResult, HostError> {
        let document_id = transaction.document_id()?;
        let document_revision = transaction.document_revision()?;
        if document_id != context.document_id || document_revision != context.document_revision {
            return Err(StateError::InvalidInput {
                message: format!(
                    "Effect context {}@{} does not match parse transaction {}@{}",
                    context.document_id, context.document_revision, document_id, document_revision,
                ),
            }
            .into());
        }
        let node_context = u64::from(request.node.syntax_context.get());
        if node_context != context.syntax_context
            || request.context.syntax_context != context.syntax_context
        {
            return Err(StateError::InvalidInput {
                message: format!(
                    "Effect syntax contexts node={node_context}, parser={}, invocation={} do not match",
                    request.context.syntax_context, context.syntax_context,
                ),
            }
            .into());
        }
        if request.node.kind != ParserRawNodeKind::Simple {
            return Err(EffectParseError::UnsupportedNodeKind {
                actual: request.node.kind,
            }
            .into());
        }
        let code_span =
            request
                .node
                .code_span
                .clone()
                .ok_or(EffectParseError::MissingCodeSpan {
                    node_id: request.node.id,
                })?;
        let code_range = code_span.virtual_range;
        let input = code_range
            .slice(request.source.virtual_source())
            .ok_or(EffectParseError::InvalidCodeRange { range: code_range })?
            .to_owned();

        config.expression.function =
            function_policy_for_runtime(self.config.runtime_profile.skript_version.as_deref());
        let catalog = self
            .config
            .syntax_catalog
            .clone()
            .ok_or(HostError::SyntaxCatalogUnavailable)?;
        let dynamic_snapshot = self.dynamic_syntax_snapshot(transaction)?;
        let base = transaction.savepoint()?;
        let source = request.source;
        let node = request.node;
        let mut parser_context = request.context;
        let mut effects = empty_effects();
        let mut calls = Vec::new();
        let mut failures = Vec::new();

        let before_payload = effect_hook_payload(EffectHookPayloadView {
            input: &input,
            context: &parser_context,
            raw_node_id: node.id,
            span: &code_span,
            timing: WitEffectTiming::Before,
            candidate: None,
            alternatives: &[],
            failure: None,
            near_match: None,
            catalog: catalog.as_ref(),
        });
        let before = match self.dispatch_in_parse(
            transaction,
            DispatchRequest {
                context: context.clone(),
                target: DispatchTarget::SyntaxKind(SyntaxKind::Effect),
                phase: HookPhase::Effect,
                payload: HookPayload::Effect(before_payload.clone()),
            },
        ) {
            Ok(result) => result,
            Err(error) => {
                transaction.rollback_to(&base)?;
                return Err(error);
            }
        };
        let before_updates = before.effects.context_updates.clone();
        merge_decision_diagnostics(&mut effects, &before.decision);
        merge_effects(&mut effects, before.effects);
        calls.extend(before.calls);
        failures.extend(before.failures);
        let HookPayload::Effect(before_output) = before.payload else {
            transaction.rollback_to(&base)?;
            return Err(HostError::InvalidEffectHookOutput {
                message: "before hook returned a different payload kind".to_owned(),
            });
        };
        if let Err(error) = validate_effect_payload_identity(&before_payload, &before_output, false)
        {
            transaction.rollback_to(&base)?;
            return Err(error);
        }
        if matches!(before.decision, HookDecision::Reject(_)) {
            transaction.rollback_to(&base)?;
            return Ok(WasmEffectParseResult {
                matches: unknown_effect_matches(source, node, None, None)?,
                effects,
                calls,
                failures,
            });
        }
        parser_context = match apply_context_updates(&parser_context, before_updates, "Effect") {
            Ok(context) => context,
            Err(message) => {
                transaction.rollback_to(&base)?;
                return Err(HostError::InvalidEffectHookOutput { message });
            }
        };

        let matching_hooks_registered = self.registry.has_matching_hooks();
        let hooks = WasmPatternHooks {
            host: self,
            transaction,
            dynamic_snapshot: Some(&dynamic_snapshot),
            matching_hooks_registered,
            context: context.clone(),
            input: source.virtual_source().to_owned(),
            frames: Vec::new(),
            scope_frames: Vec::new(),
            branch_states: Vec::new(),
            last_candidate: None,
            effects: empty_effects(),
            calls: Vec::new(),
            failures: Vec::new(),
        };
        let mut environment = WasmExpressionEnvironment {
            hooks,
            pending_leaf: None,
            pending_registered: None,
            expression_candidates: Vec::new(),
            semantic_candidates: Vec::new(),
            function_registry: None,
        };
        let parsed = run_effect_parser(
            catalog.as_ref(),
            Some(&dynamic_snapshot),
            EffectParseRequest {
                source,
                node,
                context: parser_context.clone(),
            },
            &mut environment,
            config,
        );
        let (nested_effects, nested_calls, nested_failures) = environment.into_parts();
        merge_effects(&mut effects, nested_effects);
        calls.extend(nested_calls);
        failures.extend(nested_failures);
        let mut matches = match parsed {
            Ok(matches) => matches,
            Err(error) => {
                transaction.rollback_to(&base)?;
                return Err(error.into());
            }
        };
        if matches.selected.is_none()
            && let Some(unknown) = matches.unknown.as_ref()
        {
            if let Some(candidate) = unknown.failures.primary() {
                promote_candidate_semantic_diagnostics(&mut effects, &candidate.matched);
            } else {
                promote_semantic_diagnostics(&mut effects, unknown.failures.fallback.as_ref());
            }
        }
        // Selected candidates already ran their exact Effect hook inside the
        // parser. Keep this outer pass for unknown/near-match diagnostics only.
        if matches.selected.is_some() {
            return Ok(WasmEffectParseResult {
                matches,
                effects,
                calls,
                failures,
            });
        }

        let target = matches
            .selected
            .as_ref()
            .map(|selected| DispatchTarget::Pattern {
                definition_id: selected.matched.definition_id.clone(),
                registration_id: selected.matched.registration_id.clone(),
                pattern_index: u64::try_from(selected.matched.pattern_index).unwrap_or(u64::MAX),
                syntax_kind: wit_syntax_kind(selected.matched.kind),
            })
            .or_else(|| {
                matches
                    .unknown
                    .as_ref()
                    .and_then(|unknown| unknown.failures.primary())
                    .map(|candidate| {
                        candidate.matched.pattern_index.map_or_else(
                            || DispatchTarget::Registration {
                                definition_id: candidate.matched.definition_id.clone(),
                                registration_id: candidate.matched.registration_id.clone(),
                                syntax_kind: wit_syntax_kind(candidate.matched.kind),
                            },
                            |pattern_index| DispatchTarget::Pattern {
                                definition_id: candidate.matched.definition_id.clone(),
                                registration_id: candidate.matched.registration_id.clone(),
                                pattern_index: u64::try_from(pattern_index).unwrap_or(u64::MAX),
                                syntax_kind: wit_syntax_kind(candidate.matched.kind),
                            },
                        )
                    })
            })
            .unwrap_or(DispatchTarget::SyntaxKind(SyntaxKind::Effect));
        let after_payload = effect_hook_payload(EffectHookPayloadView {
            input: &input,
            context: &parser_context,
            raw_node_id: node.id,
            span: &code_span,
            timing: WitEffectTiming::After,
            candidate: matches.selected.as_ref(),
            alternatives: &matches.alternatives,
            failure: matches.unknown.as_ref().and_then(unknown_effect_failure),
            near_match: matches
                .unknown
                .as_ref()
                .and_then(|unknown| unknown.failures.primary()),
            catalog: catalog.as_ref(),
        });
        let after = match self.dispatch_in_parse(
            transaction,
            DispatchRequest {
                context,
                target,
                phase: HookPhase::Effect,
                payload: HookPayload::Effect(after_payload.clone()),
            },
        ) {
            Ok(result) => result,
            Err(error) => {
                transaction.rollback_to(&base)?;
                return Err(error);
            }
        };
        merge_decision_diagnostics(&mut effects, &after.decision);
        merge_effects(&mut effects, after.effects);
        calls.extend(after.calls);
        failures.extend(after.failures);
        let HookPayload::Effect(after_output) = after.payload else {
            transaction.rollback_to(&base)?;
            return Err(HostError::InvalidEffectHookOutput {
                message: "after hook returned a different payload kind".to_owned(),
            });
        };
        if let Err(error) = validate_effect_payload_identity(&after_payload, &after_output, true) {
            transaction.rollback_to(&base)?;
            return Err(error);
        }

        if let HookDecision::Reject(rejection) = after.decision {
            transaction.rollback_to(&base)?;
            if let Some(selected) = matches.selected.take() {
                let span = rejection
                    .diagnostics
                    .first()
                    .and_then(|diagnostic| {
                        let start = usize::try_from(diagnostic.span.virtual_range.start).ok()?;
                        let end = usize::try_from(diagnostic.span.virtual_range.end).ok()?;
                        selected
                            .parsed_captures
                            .iter()
                            .find(|capture| {
                                capture.result.span.mapped.virtual_range
                                    == ParserTextRange::new(start, end)
                            })
                            .map(|capture| capture.result.span.clone())
                    })
                    .unwrap_or_else(|| selected.matched.matched.span.clone());
                let failure = PatternFailure {
                    span,
                    reasons: vec![PatternFailureReason::HookRejected {
                        reason: rejection.reason,
                    }],
                };
                let registration_id = selected.matched.registration_id.clone();
                let element_class = catalog
                    .effects()
                    .find(|effect| effect.common.registration_id.as_str() == registration_id)
                    .map(|effect| effect.common.element_class.clone());
                let rejected = EffectCandidateFailure {
                    matched: CandidateFailure {
                        kind: selected.matched.kind,
                        definition_id: selected.matched.definition_id,
                        registration_id: selected.matched.registration_id,
                        priority: selected.matched.priority,
                        registration_order: selected.matched.registration_order,
                        resolved_order: None,
                        literal_anchor: selected.matched.literal_anchor,
                        pattern_index: Some(selected.matched.pattern_index),
                        pattern: Some(selected.matched.pattern),
                        trace: FailureTrace::leaf(failure.clone()),
                        related: Vec::new(),
                    },
                    element_class,
                    handler: selected.handler,
                    metadata: selected.metadata,
                };
                matches = unknown_effect_matches(source, node, Some(failure), Some(rejected))?;
            }
        } else if matches.selected.is_some() {
            if let Err(error) = apply_effect_hook_replacement(&mut matches, after_output) {
                transaction.rollback_to(&base)?;
                return Err(error);
            }
        } else {
            transaction.rollback_to(&base)?;
            retain_diagnostics_only(&mut effects, &mut calls);
        }

        Ok(WasmEffectParseResult {
            matches,
            effects,
            calls,
            failures,
        })
    }
    /// Disables an addon and removes its baseline dynamic syntax.
    pub fn unload_addon(&mut self, component_id: &str) -> Result<bool, HostError> {
        if component_id == CORE_LIBRARY_COMPONENT_ID {
            return Err(HostError::CannotUnloadCoreLibrary);
        }
        let Some(entry) = self
            .components
            .iter_mut()
            .find(|entry| entry.manifest.component_id == component_id && !entry.unloaded)
        else {
            return Ok(false);
        };
        entry.unloaded = true;
        if let Some(registry) = &self.dynamic_syntax_registry {
            registry.remove_component(component_id)?;
        }
        Ok(true)
    }

    /// Runs one generic dispatch in an automatically committed parse transaction.
    pub fn dispatch(
        &mut self,
        project_uri: &str,
        request: DispatchRequest,
    ) -> Result<DispatchResult, HostError> {
        let transaction = self.begin_parse(
            project_uri,
            &request.context.document_id,
            request.context.document_revision,
        )?;
        match self.dispatch_in_parse(&transaction, request) {
            Ok(result) if matches!(result.decision, HookDecision::Reject(_)) => {
                transaction.cancel()?;
                Ok(result)
            }
            Ok(result) => {
                if self.dynamic_syntax_registry.is_some()
                    && let Err(error) = self.dynamic_syntax_snapshot(&transaction)
                {
                    let _ = transaction.cancel();
                    return Err(error);
                }
                transaction.commit()?;
                Ok(result)
            }
            Err(error) => {
                let _ = transaction.cancel();
                Err(error)
            }
        }
    }

    /// Runs one generic dispatch inside a caller-owned parse transaction.
    pub fn dispatch_in_parse(
        &mut self,
        transaction: &ParseTransaction,
        request: DispatchRequest,
    ) -> Result<DispatchResult, HostError> {
        let document_id = transaction.document_id()?;
        let document_revision = transaction.document_revision()?;
        if document_id != request.context.document_id
            || document_revision != request.context.document_revision
        {
            return Err(StateError::InvalidInput {
                message: format!(
                    "dispatch context {}@{} does not match parse transaction {}@{}",
                    request.context.document_id,
                    request.context.document_revision,
                    document_id,
                    document_revision
                ),
            }
            .into());
        }

        let state_savepoint = transaction.savepoint()?;
        let dynamic_savepoint = if is_dynamic_prepass_phase(request.phase) {
            self.dynamic_syntax_registry
                .as_ref()
                .map(|registry| registry.savepoint(&document_id, document_revision))
                .transpose()?
        } else {
            if let Some(registry) = &self.dynamic_syntax_registry {
                registry.freeze(&document_id, document_revision)?;
            }
            None
        };
        let result = self.dispatch_with_transaction(transaction, request);
        match result {
            Ok(result) if matches!(result.decision, HookDecision::Reject(_)) => {
                transaction.rollback_to(&state_savepoint)?;
                if let (Some(registry), Some(savepoint)) =
                    (&self.dynamic_syntax_registry, &dynamic_savepoint)
                {
                    registry.rollback_to(savepoint)?;
                }
                Ok(result)
            }
            Ok(result) => Ok(result),
            Err(error) => {
                transaction.rollback_to(&state_savepoint)?;
                if let (Some(registry), Some(savepoint)) =
                    (&self.dynamic_syntax_registry, &dynamic_savepoint)
                {
                    registry.rollback_to(savepoint)?;
                }
                Err(error)
            }
        }
    }

    /// Runs Text preprocessing in an automatically committed parse transaction.
    pub fn expand_text(
        &mut self,
        project_uri: &str,
        request: TextMacroRequest,
    ) -> Result<TextMacroResult, HostError> {
        let transaction = self.begin_parse(
            project_uri,
            &request.context.document_id,
            request.context.document_revision,
        )?;
        match self.expand_text_in_parse(&transaction, request) {
            Ok(result) if matches!(result.decision, HookDecision::Reject(_)) => {
                transaction.cancel()?;
                Ok(result)
            }
            Ok(result) => {
                if self.dynamic_syntax_registry.is_some()
                    && let Err(error) = self.dynamic_syntax_snapshot(&transaction)
                {
                    let _ = transaction.cancel();
                    return Err(error);
                }
                transaction.commit()?;
                Ok(result)
            }
            Err(error) => {
                let _ = transaction.cancel();
                Err(error)
            }
        }
    }

    /// Runs ordered Text macros inside a caller-owned parse transaction.
    pub fn expand_text_in_parse(
        &mut self,
        transaction: &ParseTransaction,
        request: TextMacroRequest,
    ) -> Result<TextMacroResult, HostError> {
        let document_id = transaction.document_id()?;
        let document_revision = transaction.document_revision()?;
        if document_id != request.context.document_id
            || document_revision != request.context.document_revision
        {
            return Err(StateError::InvalidInput {
                message: format!(
                    "text macro context {}@{} does not match parse transaction {}@{}",
                    request.context.document_id,
                    request.context.document_revision,
                    document_id,
                    document_revision
                ),
            }
            .into());
        }
        if request.source.virtual_source().len() > self.config.max_virtual_source_bytes {
            return Err(HostError::VirtualSourceQuotaExceeded {
                limit: self.config.max_virtual_source_bytes,
            });
        }

        let original_source = request.source.clone();
        let state_savepoint = transaction.savepoint()?;
        let dynamic_savepoint = self
            .dynamic_syntax_registry
            .as_ref()
            .map(|registry| registry.savepoint(&document_id, document_revision))
            .transpose()?;
        let result = self.expand_text_with_transaction(transaction, request);
        match result {
            Ok(mut result) if matches!(result.decision, HookDecision::Reject(_)) => {
                transaction.rollback_to(&state_savepoint)?;
                if let (Some(registry), Some(savepoint)) =
                    (&self.dynamic_syntax_registry, &dynamic_savepoint)
                {
                    registry.rollback_to(savepoint)?;
                }
                mark_text_macro_result_rolled_back(&original_source, &mut result);
                result.source = original_source;
                Ok(result)
            }
            Ok(result) => Ok(result),
            Err(error) => {
                transaction.rollback_to(&state_savepoint)?;
                if let (Some(registry), Some(savepoint)) =
                    (&self.dynamic_syntax_registry, &dynamic_savepoint)
                {
                    registry.rollback_to(savepoint)?;
                }
                Err(error)
            }
        }
    }

    fn expand_text_with_transaction(
        &mut self,
        transaction: &ParseTransaction,
        request: TextMacroRequest,
    ) -> Result<TextMacroResult, HostError> {
        let candidates = self.registry.matching_capability(
            &DispatchTarget::ParseStage,
            HookPhase::Preprocess,
            CAPABILITY_TEXT_MACRO,
        );
        let document_id = transaction.document_id()?;
        let document_revision = transaction.document_revision()?;
        let mut source = request.source;
        let mut effects = empty_effects();
        let mut calls = Vec::new();
        let mut failures = Vec::new();
        let mut output_bytes = 0usize;
        let mut generated_bytes = 0usize;
        let mut expansions = 0usize;
        let mut decision = HookDecision::ContinueProcessing;

        for candidate in candidates {
            if self.components[candidate.component_index].disabled
                || self.components[candidate.component_index].unloaded
            {
                continue;
            }
            if calls.len() >= self.config.max_calls_per_dispatch {
                return Err(HostError::CallQuotaExceeded {
                    limit: self.config.max_calls_per_dispatch,
                });
            }

            let component_id = self.components[candidate.component_index]
                .manifest
                .component_id
                .clone();
            let subscription_id = candidate.subscription.id.clone();
            let mapped = source
                .map_range(ParserTextRange::new(0, source.virtual_source().len()))
                .expect("the complete virtual source range is always valid");
            let parent = mapped.primary_origin().and_then(|origin| origin.expansion);
            let syntax_context = parent
                .and_then(|id| source.expansions().get(id))
                .map_or(0, |expansion| u64::from(expansion.syntax_context.get()));
            let mut context = request.context.clone();
            context.subscription_id = subscription_id.clone();
            context.expansion = parent.map(|id| u64::from(id.get()));
            context.syntax_context = syntax_context;
            let input = TextMacroInput {
                context,
                text: source.virtual_source().to_owned(),
                span: mapped_span_to_wit(mapped),
            };

            let state_invocation = transaction.begin_invocation(component_id.clone())?;
            let dynamic_update = self
                .dynamic_syntax_registry
                .as_ref()
                .map(|registry| {
                    registry.begin_document_update(
                        component_id.clone(),
                        self.components[candidate.component_index].load_order,
                        &document_id,
                        document_revision,
                    )
                })
                .transpose()?;
            let call =
                {
                    let entry = &mut self.components[candidate.component_index];
                    if entry.store.data().invocation.is_some()
                        || entry.store.data().dynamic_syntax_update.is_some()
                    {
                        return Err(StateError::Internal {
                            message: format!(
                                "component {component_id} already has an active host transaction"
                            ),
                        }
                        .into());
                    }
                    entry.store.data_mut().invocation = Some(state_invocation);
                    entry.store.data_mut().dynamic_syntax_update = dynamic_update;
                    if let Err(error) = prepare_store(
                        &mut entry.store,
                        self.config.fuel_per_call,
                        self.config.deadline_ticks(&component_id),
                        &component_id,
                        "text macro",
                    ) {
                        entry
                            .store
                            .data_mut()
                            .invocation
                            .take()
                            .expect("the invocation was just installed")
                            .rollback();
                        entry.store.data_mut().dynamic_syntax_update.take();
                        return Err(error);
                    }
                    let call = entry
                        .bindings
                        .nlaocs_skript_parser_addon_text_macro()
                        .call_expand(&mut entry.store, &input);
                    let state_invocation =
                        entry.store.data_mut().invocation.take().expect(
                            "the invocation remains installed for the duration of the call",
                        );
                    let dynamic_update = entry.store.data_mut().dynamic_syntax_update.take();
                    (call, state_invocation, dynamic_update)
                };

            let (call, state_invocation, dynamic_update) = call;
            let accesses = state_invocation.read_write_set();
            let mut output = match call {
                Ok(Ok(output)) => output,
                Ok(Err(mut addon_error)) => {
                    let diagnostic_error = normalize_text_macro_diagnostics(
                        &source,
                        &mut addon_error.diagnostics,
                        "addon-error.diagnostics",
                    );
                    state_invocation.rollback();
                    drop(dynamic_update);
                    calls.push(TextMacroCall {
                        component_id: component_id.clone(),
                        subscription_id: subscription_id.clone(),
                        accepted: false,
                        expansion: None,
                        state_accesses: accesses,
                    });
                    let error = match diagnostic_error {
                        Ok(()) => {
                            effects.diagnostics.extend(addon_error.diagnostics);
                            HostError::AddonFailure {
                                component_id: component_id.clone(),
                                message: addon_error.message,
                            }
                        }
                        Err(message) => HostError::InvalidTextMacroOutput {
                            component_id: component_id.clone(),
                            subscription_id: subscription_id.clone(),
                            message,
                        },
                    };
                    failures.push(ComponentFailure {
                        component_id,
                        subscription_id,
                        error,
                    });
                    continue;
                }
                Err(error) => {
                    state_invocation.rollback();
                    drop(dynamic_update);
                    let error = classify_wasmtime_error(component_id.clone(), "text macro", error);
                    if error.disables_component() {
                        self.components[candidate.component_index].disabled = true;
                        if let Some(registry) = &self.dynamic_syntax_registry {
                            registry.remove_component(&component_id)?;
                        }
                    }
                    calls.push(TextMacroCall {
                        component_id: component_id.clone(),
                        subscription_id: subscription_id.clone(),
                        accepted: false,
                        expansion: None,
                        state_accesses: accesses,
                    });
                    failures.push(ComponentFailure {
                        component_id,
                        subscription_id,
                        error,
                    });
                    continue;
                }
            };

            output_bytes = output_bytes.saturating_add(text_macro_output_size(&output));
            if output_bytes > self.config.max_generated_output_bytes {
                state_invocation.rollback();
                drop(dynamic_update);
                return Err(HostError::GeneratedOutputQuotaExceeded {
                    limit: self.config.max_generated_output_bytes,
                });
            }

            if matches!(output.decision, HookDecision::NotApplicable) {
                state_invocation.rollback();
                drop(dynamic_update);
                calls.push(TextMacroCall {
                    component_id,
                    subscription_id,
                    accepted: false,
                    expansion: None,
                    state_accesses: accesses,
                });
                continue;
            }

            if let Err(message) = normalize_text_macro_output_spans(&source, &mut output) {
                state_invocation.rollback();
                drop(dynamic_update);
                calls.push(TextMacroCall {
                    component_id: component_id.clone(),
                    subscription_id: subscription_id.clone(),
                    accepted: false,
                    expansion: None,
                    state_accesses: accesses,
                });
                failures.push(ComponentFailure {
                    component_id: component_id.clone(),
                    subscription_id: subscription_id.clone(),
                    error: HostError::InvalidTextMacroOutput {
                        component_id,
                        subscription_id,
                        message,
                    },
                });
                continue;
            }

            let TextMacroOutput {
                decision: macro_decision,
                edits,
                effects: mut macro_effects,
            } = output;
            stamp_parse_result_attachments(&mut macro_effects, &component_id);
            if matches!(macro_decision, HookDecision::Reject(_)) {
                state_invocation.rollback();
                drop(dynamic_update);
                calls.push(TextMacroCall {
                    component_id,
                    subscription_id,
                    accepted: false,
                    expansion: None,
                    state_accesses: accesses,
                });
                merge_effects(&mut effects, macro_effects);
                decision = macro_decision;
                break;
            }

            let parser_edits = match parser_text_edits(edits) {
                Ok(edits) => edits,
                Err(message) => {
                    state_invocation.rollback();
                    drop(dynamic_update);
                    calls.push(TextMacroCall {
                        component_id: component_id.clone(),
                        subscription_id: subscription_id.clone(),
                        accepted: false,
                        expansion: None,
                        state_accesses: accesses,
                    });
                    failures.push(ComponentFailure {
                        component_id: component_id.clone(),
                        subscription_id: subscription_id.clone(),
                        error: HostError::InvalidTextMacroOutput {
                            component_id,
                            subscription_id,
                            message,
                        },
                    });
                    continue;
                }
            };
            let application = match source.apply_text_edits(
                parser_edits,
                TextExpansion::new(component_id.clone(), subscription_id.clone()),
            ) {
                Ok(application) => application,
                Err(error) => {
                    state_invocation.rollback();
                    drop(dynamic_update);
                    calls.push(TextMacroCall {
                        component_id: component_id.clone(),
                        subscription_id: subscription_id.clone(),
                        accepted: false,
                        expansion: None,
                        state_accesses: accesses,
                    });
                    failures.push(ComponentFailure {
                        component_id: component_id.clone(),
                        subscription_id: subscription_id.clone(),
                        error: HostError::InvalidTextMacroOutput {
                            component_id,
                            subscription_id,
                            message: error.to_string(),
                        },
                    });
                    continue;
                }
            };
            let next_expansions = expansions + usize::from(application.expansion.is_some());
            if next_expansions > self.config.max_text_macro_expansions {
                state_invocation.rollback();
                drop(dynamic_update);
                return Err(HostError::TextMacroExpansionQuotaExceeded {
                    limit: self.config.max_text_macro_expansions,
                });
            }
            let next_generated = generated_bytes.saturating_add(application.generated_bytes);
            if next_generated > self.config.max_text_macro_generated_bytes {
                state_invocation.rollback();
                drop(dynamic_update);
                return Err(HostError::TextMacroGeneratedBytesQuotaExceeded {
                    limit: self.config.max_text_macro_generated_bytes,
                });
            }
            if application.source.virtual_source().len() > self.config.max_virtual_source_bytes {
                state_invocation.rollback();
                drop(dynamic_update);
                return Err(HostError::VirtualSourceQuotaExceeded {
                    limit: self.config.max_virtual_source_bytes,
                });
            }

            state_invocation.commit()?;
            if let Some(update) = dynamic_update {
                update.commit()?;
            }
            expansions = next_expansions;
            generated_bytes = next_generated;
            source = application.source;
            calls.push(TextMacroCall {
                component_id,
                subscription_id,
                accepted: true,
                expansion: application.expansion,
                state_accesses: accesses,
            });
            merge_effects(&mut effects, macro_effects);
            if matches!(macro_decision, HookDecision::Handled) {
                decision = macro_decision;
                break;
            }
        }

        Ok(TextMacroResult {
            decision,
            source,
            effects,
            calls,
            failures,
        })
    }

    /// Runs recursive Tree macros in an automatically committed parse transaction.
    pub fn expand_tree(
        &mut self,
        project_uri: &str,
        request: TreeMacroRequest,
    ) -> Result<TreeMacroResult, HostError> {
        let transaction = self.begin_parse(
            project_uri,
            &request.context.document_id,
            request.context.document_revision,
        )?;
        match self.expand_tree_in_parse(&transaction, request) {
            Ok(result) if matches!(result.decision, HookDecision::Reject(_)) => {
                transaction.cancel()?;
                Ok(result)
            }
            Ok(result) => {
                if self.dynamic_syntax_registry.is_some()
                    && let Err(error) = self.dynamic_syntax_snapshot(&transaction)
                {
                    let _ = transaction.cancel();
                    return Err(error);
                }
                transaction.commit()?;
                Ok(result)
            }
            Err(error) => {
                let _ = transaction.cancel();
                Err(error)
            }
        }
    }

    /// Runs recursive pre-order Tree expansion inside a caller-owned transaction.
    pub fn expand_tree_in_parse(
        &mut self,
        transaction: &ParseTransaction,
        request: TreeMacroRequest,
    ) -> Result<TreeMacroResult, HostError> {
        let document_id = transaction.document_id()?;
        let document_revision = transaction.document_revision()?;
        if document_id != request.context.document_id
            || document_revision != request.context.document_revision
        {
            return Err(StateError::InvalidInput {
                message: format!(
                    "tree macro context {}@{} does not match parse transaction {}@{}",
                    request.context.document_id,
                    request.context.document_revision,
                    document_id,
                    document_revision
                ),
            }
            .into());
        }
        if request.source.virtual_source().len() > self.config.max_virtual_source_bytes {
            return Err(HostError::VirtualSourceQuotaExceeded {
                limit: self.config.max_virtual_source_bytes,
            });
        }
        if request.tree.nodes.len() > self.config.max_tree_macro_nodes {
            return Err(HostError::TreeMacroNodeQuotaExceeded {
                limit: self.config.max_tree_macro_nodes,
            });
        }
        if raw_tree_depth(&request.tree) > self.config.max_raw_tree_depth {
            return Err(HostError::RawTreeDepthQuotaExceeded {
                limit: self.config.max_raw_tree_depth,
            });
        }

        let original_source = request.source.clone();
        let original_tree = request.tree.clone();
        let state_savepoint = transaction.savepoint()?;
        let result = self.expand_tree_with_transaction(transaction, request);
        match result {
            Ok(mut result) if matches!(result.decision, HookDecision::Reject(_)) => {
                transaction.rollback_to(&state_savepoint)?;
                mark_tree_macro_result_rolled_back(&original_source, &original_tree, &mut result);
                Ok(result)
            }
            Ok(result) => Ok(result),
            Err(error) => {
                transaction.rollback_to(&state_savepoint)?;
                Err(error)
            }
        }
    }

    fn expand_tree_with_transaction(
        &mut self,
        transaction: &ParseTransaction,
        request: TreeMacroRequest,
    ) -> Result<TreeMacroResult, HostError> {
        let candidates = self.registry.matching_capability(
            &DispatchTarget::ParseStage,
            HookPhase::Tree,
            CAPABILITY_TREE_MACRO,
        );
        let mut source = request.source;
        let mut tree = request.tree;
        let mut pipeline = TreeMacroPipeline::new();
        let mut root_index = 0usize;
        let mut decision = HookDecision::ContinueProcessing;

        while root_index < tree.roots.len() {
            let path = vec![root_index];
            match self.expand_tree_node(
                transaction,
                &request.context,
                &candidates,
                &mut source,
                &mut tree,
                &path,
                0,
                &mut pipeline,
            )? {
                TreeWalk::Continue { sibling_count } => {
                    root_index = root_index.saturating_add(sibling_count);
                }
                TreeWalk::Reject(rejection) => {
                    decision = rejection;
                    break;
                }
            }
        }

        if matches!(decision, HookDecision::ContinueProcessing) && pipeline.handled {
            decision = HookDecision::Handled;
        }

        Ok(TreeMacroResult {
            decision,
            source,
            tree,
            effects: pipeline.effects,
            calls: pipeline.calls,
            failures: pipeline.failures,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_tree_node(
        &mut self,
        transaction: &ParseTransaction,
        request_context: &InvocationContext,
        candidates: &[RegisteredSubscription],
        source: &mut MappedSource,
        tree: &mut ParserRawTree,
        path: &[usize],
        depth: usize,
        pipeline: &mut TreeMacroPipeline,
    ) -> Result<TreeWalk, HostError> {
        if depth > self.config.max_tree_macro_expansion_depth {
            return Err(HostError::TreeMacroExpansionDepthQuotaExceeded {
                limit: self.config.max_tree_macro_expansion_depth,
            });
        }
        if path.len() > self.config.max_raw_tree_depth {
            return Err(HostError::RawTreeDepthQuotaExceeded {
                limit: self.config.max_raw_tree_depth,
            });
        }

        let Some(_) = raw_node_at_path(tree, path) else {
            return Err(HostError::InvalidTreeMacroOutput {
                component_id: "<host>".to_owned(),
                subscription_id: "<tree-walk>".to_owned(),
                message: format!("tree path {path:?} no longer resolves"),
            });
        };

        let mut stop_current_node = false;
        for candidate in candidates {
            if stop_current_node {
                break;
            }
            if self.components[candidate.component_index].disabled
                || self.components[candidate.component_index].unloaded
            {
                continue;
            }
            if pipeline.calls.len() >= self.config.max_tree_macro_calls {
                return Err(HostError::TreeMacroCallQuotaExceeded {
                    limit: self.config.max_tree_macro_calls,
                });
            }

            let component_id = self.components[candidate.component_index]
                .manifest
                .component_id
                .clone();
            let subscription_id = candidate.subscription.id.clone();
            let current_target =
                raw_node_at_path(tree, path).ok_or_else(|| HostError::InvalidTreeMacroOutput {
                    component_id: component_id.clone(),
                    subscription_id: subscription_id.clone(),
                    message: format!("tree path {path:?} disappeared before invocation"),
                })?;
            let node = tree
                .get(current_target)
                .expect("a resolved RawNodeId must exist")
                .clone();
            let cycle_key =
                tree_macro_cycle_key(&component_id, &subscription_id, tree, current_target);
            if pipeline.active.contains(&cycle_key) {
                let error = HostError::TreeMacroCycleDetected {
                    component_id: component_id.clone(),
                    subscription_id: subscription_id.clone(),
                };
                pipeline
                    .effects
                    .diagnostics
                    .push(tree_macro_cycle_diagnostic(
                        &node,
                        &component_id,
                        &subscription_id,
                    ));
                pipeline.calls.push(TreeMacroCall {
                    component_id: component_id.clone(),
                    subscription_id: subscription_id.clone(),
                    target: current_target,
                    accepted: false,
                    expansion: None,
                    state_accesses: StateReadWriteSet::default(),
                });
                pipeline.failures.push(ComponentFailure {
                    component_id,
                    subscription_id,
                    error,
                });
                continue;
            }
            pipeline.active.push(cycle_key);

            let parent = node
                .span
                .primary_origin()
                .and_then(|origin| origin.expansion);
            let mut context = request_context.clone();
            context.subscription_id = subscription_id.clone();
            context.expansion = parent.map(|id| u64::from(id.get()));
            context.syntax_context = u64::from(node.syntax_context.get());
            let input = TreeMacroInput {
                context,
                tree: parser_raw_tree_to_wit(tree),
                target: current_target.get(),
                depth: u32::try_from(depth).unwrap_or(u32::MAX),
            };

            let state_invocation = transaction.begin_invocation(component_id.clone())?;
            let call =
                {
                    let entry = &mut self.components[candidate.component_index];
                    if entry.store.data().invocation.is_some()
                        || entry.store.data().dynamic_syntax_update.is_some()
                    {
                        pipeline.active.pop();
                        return Err(StateError::Internal {
                            message: format!(
                                "component {component_id} already has an active host transaction"
                            ),
                        }
                        .into());
                    }
                    entry.store.data_mut().invocation = Some(state_invocation);
                    if let Err(error) = prepare_store(
                        &mut entry.store,
                        self.config.fuel_per_call,
                        self.config.deadline_ticks(&component_id),
                        &component_id,
                        "tree macro",
                    ) {
                        entry
                            .store
                            .data_mut()
                            .invocation
                            .take()
                            .expect("the invocation was just installed")
                            .rollback();
                        pipeline.active.pop();
                        return Err(error);
                    }
                    let call = entry
                        .bindings
                        .nlaocs_skript_parser_addon_tree_macro()
                        .call_expand(&mut entry.store, &input);
                    let state_invocation =
                        entry.store.data_mut().invocation.take().expect(
                            "the invocation remains installed for the duration of the call",
                        );
                    (call, state_invocation)
                };

            let (call, state_invocation) = call;
            let accesses = state_invocation.read_write_set();
            let mut output = match call {
                Ok(Ok(output)) => output,
                Ok(Err(mut addon_error)) => {
                    let diagnostic_error = normalize_text_macro_diagnostics(
                        source,
                        &mut addon_error.diagnostics,
                        "addon-error.diagnostics",
                    );
                    state_invocation.rollback();
                    pipeline.active.pop();
                    pipeline.calls.push(TreeMacroCall {
                        component_id: component_id.clone(),
                        subscription_id: subscription_id.clone(),
                        target: current_target,
                        accepted: false,
                        expansion: None,
                        state_accesses: accesses,
                    });
                    let error = match diagnostic_error {
                        Ok(()) => {
                            pipeline.effects.diagnostics.extend(addon_error.diagnostics);
                            HostError::AddonFailure {
                                component_id: component_id.clone(),
                                message: addon_error.message,
                            }
                        }
                        Err(message) => HostError::InvalidTreeMacroOutput {
                            component_id: component_id.clone(),
                            subscription_id: subscription_id.clone(),
                            message,
                        },
                    };
                    pipeline.failures.push(ComponentFailure {
                        component_id,
                        subscription_id,
                        error,
                    });
                    continue;
                }
                Err(error) => {
                    state_invocation.rollback();
                    pipeline.active.pop();
                    let error = classify_wasmtime_error(component_id.clone(), "tree macro", error);
                    if error.disables_component() {
                        self.components[candidate.component_index].disabled = true;
                        if let Some(registry) = &self.dynamic_syntax_registry {
                            registry.remove_component(&component_id)?;
                        }
                    }
                    pipeline.calls.push(TreeMacroCall {
                        component_id: component_id.clone(),
                        subscription_id: subscription_id.clone(),
                        target: current_target,
                        accepted: false,
                        expansion: None,
                        state_accesses: accesses,
                    });
                    pipeline.failures.push(ComponentFailure {
                        component_id,
                        subscription_id,
                        error,
                    });
                    continue;
                }
            };

            pipeline.output_bytes = pipeline
                .output_bytes
                .saturating_add(tree_macro_output_size(&output));
            if pipeline.output_bytes > self.config.max_generated_output_bytes {
                state_invocation.rollback();
                pipeline.active.pop();
                return Err(HostError::GeneratedOutputQuotaExceeded {
                    limit: self.config.max_generated_output_bytes,
                });
            }

            if let Err(message) = normalize_tree_macro_output_spans(source, &mut output) {
                state_invocation.rollback();
                pipeline.active.pop();
                pipeline.calls.push(TreeMacroCall {
                    component_id: component_id.clone(),
                    subscription_id: subscription_id.clone(),
                    target: current_target,
                    accepted: false,
                    expansion: None,
                    state_accesses: accesses,
                });
                pipeline.failures.push(ComponentFailure {
                    component_id: component_id.clone(),
                    subscription_id: subscription_id.clone(),
                    error: HostError::InvalidTreeMacroOutput {
                        component_id,
                        subscription_id,
                        message,
                    },
                });
                continue;
            }

            let TreeMacroOutput {
                decision,
                edit,
                mut effects,
            } = output;
            if matches!(decision, HookDecision::NotApplicable) {
                state_invocation.rollback();
                pipeline.active.pop();
                pipeline.calls.push(TreeMacroCall {
                    component_id,
                    subscription_id,
                    target: current_target,
                    accepted: false,
                    expansion: None,
                    state_accesses: accesses,
                });
                continue;
            }
            stamp_parse_result_attachments(&mut effects, &component_id);
            if matches!(decision, HookDecision::Reject(_)) {
                state_invocation.rollback();
                pipeline.active.pop();
                pipeline.calls.push(TreeMacroCall {
                    component_id,
                    subscription_id,
                    target: current_target,
                    accepted: false,
                    expansion: None,
                    state_accesses: accesses,
                });
                merge_effects(&mut pipeline.effects, effects);
                return Ok(TreeWalk::Reject(decision));
            }

            let Some(edit) = edit else {
                state_invocation.commit()?;
                pipeline.active.pop();
                pipeline.calls.push(TreeMacroCall {
                    component_id,
                    subscription_id,
                    target: current_target,
                    accepted: true,
                    expansion: None,
                    state_accesses: accesses,
                });
                merge_effects(&mut pipeline.effects, effects);
                if matches!(decision, HookDecision::Handled) {
                    pipeline.handled = true;
                    stop_current_node = true;
                }
                continue;
            };

            let resulting_depth = match &edit {
                WitTreeEdit::ReplaceNode(_) => path
                    .len()
                    .saturating_sub(1)
                    .saturating_add(wit_tree_edit_depth(&edit)),
                WitTreeEdit::ReplaceChildren(_) => {
                    path.len().saturating_add(wit_tree_edit_depth(&edit))
                }
            };
            if resulting_depth > self.config.max_raw_tree_depth {
                state_invocation.rollback();
                pipeline.active.pop();
                return Err(HostError::RawTreeDepthQuotaExceeded {
                    limit: self.config.max_raw_tree_depth,
                });
            }
            let parser_edit = parser_tree_edit(edit);
            let replaces_node = matches!(parser_edit, ParserTreeEdit::ReplaceNode { .. });
            let application = match apply_tree_edit(
                source,
                tree,
                current_target,
                parser_edit,
                TreeEditMetadata {
                    component: component_id.clone(),
                    hook: subscription_id.clone(),
                },
            ) {
                Ok(application) => application,
                Err(error) => {
                    state_invocation.rollback();
                    pipeline.active.pop();
                    pipeline.calls.push(TreeMacroCall {
                        component_id: component_id.clone(),
                        subscription_id: subscription_id.clone(),
                        target: current_target,
                        accepted: false,
                        expansion: None,
                        state_accesses: accesses,
                    });
                    pipeline.failures.push(ComponentFailure {
                        component_id: component_id.clone(),
                        subscription_id: subscription_id.clone(),
                        error: HostError::InvalidTreeMacroOutput {
                            component_id,
                            subscription_id,
                            message: error.to_string(),
                        },
                    });
                    continue;
                }
            };
            if application.tree.nodes.len() > self.config.max_tree_macro_nodes {
                state_invocation.rollback();
                pipeline.active.pop();
                return Err(HostError::TreeMacroNodeQuotaExceeded {
                    limit: self.config.max_tree_macro_nodes,
                });
            }

            state_invocation.commit()?;
            let expansion = application.expansion;
            let replacement_roots = application.replacement_roots;
            *source = application.source;
            *tree = application.tree;
            pipeline.calls.push(TreeMacroCall {
                component_id,
                subscription_id,
                target: current_target,
                accepted: true,
                expansion: Some(expansion),
                state_accesses: accesses,
            });
            merge_effects(&mut pipeline.effects, effects);
            if matches!(decision, HookDecision::Handled) {
                pipeline.handled = true;
                stop_current_node = true;
            }

            if replaces_node {
                let base_index = *path
                    .last()
                    .expect("tree node paths always include a sibling index");
                let mut final_count = 0usize;
                for _ in 0..replacement_roots {
                    let mut generated_path = path.to_vec();
                    *generated_path
                        .last_mut()
                        .expect("tree node paths always include a sibling index") =
                        base_index.saturating_add(final_count);
                    match self.expand_tree_node(
                        transaction,
                        request_context,
                        candidates,
                        source,
                        tree,
                        &generated_path,
                        depth.saturating_add(1),
                        pipeline,
                    )? {
                        TreeWalk::Continue { sibling_count } => {
                            final_count = final_count.saturating_add(sibling_count);
                        }
                        TreeWalk::Reject(rejection) => {
                            pipeline.active.pop();
                            return Ok(TreeWalk::Reject(rejection));
                        }
                    }
                }
                pipeline.active.pop();
                return Ok(TreeWalk::Continue {
                    sibling_count: final_count,
                });
            }

            pipeline.active.pop();
        }

        let Some(current_target) = raw_node_at_path(tree, path) else {
            return Ok(TreeWalk::Continue { sibling_count: 0 });
        };
        let mut child_index = 0usize;
        while child_index
            < tree
                .get(current_target)
                .map_or(0, |node| node.children.len())
        {
            let mut child_path = path.to_vec();
            child_path.push(child_index);
            match self.expand_tree_node(
                transaction,
                request_context,
                candidates,
                source,
                tree,
                &child_path,
                depth,
                pipeline,
            )? {
                TreeWalk::Continue { sibling_count } => {
                    child_index = child_index.saturating_add(sibling_count);
                }
                TreeWalk::Reject(rejection) => return Ok(TreeWalk::Reject(rejection)),
            }
        }

        Ok(TreeWalk::Continue { sibling_count: 1 })
    }
    fn execute_parse_requests(
        &mut self,
        transaction: &ParseTransaction,
        context: &InvocationContext,
        parent_context: Option<&WitParseContext>,
        requests: Vec<ParseRequest>,
    ) -> Result<Vec<ExecutedParseResult>, HostError> {
        let mut results = Vec::with_capacity(requests.len());
        let mut node_count = 0usize;
        for request in requests {
            let request = inherit_parse_request_context(request, parent_context);
            let key = ParseRequestKey::new(&request, context);
            let mut result = if self.active_parse_requests.contains(&key) {
                ExecutedParseResult {
                    wire: failed_parse_result(
                        &request,
                        "parser.request-cycle",
                        "recursive parser request cycle detected",
                    ),
                    expression_roots: BTreeMap::new(),
                }
            } else {
                self.active_parse_requests.push(key);
                let result = self.execute_parse_request(transaction, context, &request);
                self.active_parse_requests.pop();
                result?
            };
            validate_parse_result(&request, &result.wire)?;
            node_count = node_count.saturating_add(result.wire.nodes.len());
            if node_count > self.config.max_parse_result_nodes {
                return Err(HostError::ParseResultNodeQuotaExceeded {
                    limit: self.config.max_parse_result_nodes,
                });
            }
            result.wire.host_token = self.next_parse_result_token;
            self.next_parse_result_token = self
                .next_parse_result_token
                .checked_add(1)
                .ok_or(HostError::ParseResultTokenExhausted)?;
            results.push(result);
        }
        Ok(results)
    }

    fn execute_parse_request(
        &mut self,
        transaction: &ParseTransaction,
        context: &InvocationContext,
        request: &ParseRequest,
    ) -> Result<ExecutedParseResult, HostError> {
        match request.parser_id.as_str() {
            skript_parser::HOST_EXPRESSION_PARSER_ID => {
                self.execute_expression_parse_request(transaction, context, request)
            }
            skript_parser::HOST_CONDITION_PARSER_ID => {
                self.execute_condition_parse_request(transaction, context, request)
            }
            _ => self.execute_addon_parse_request(transaction, context, request),
        }
    }

    fn execute_expression_parse_request(
        &mut self,
        transaction: &ParseTransaction,
        context: &InvocationContext,
        request: &ParseRequest,
    ) -> Result<ExecutedParseResult, HostError> {
        let source = MappedSource::identity(request.input.clone());
        let parsed = self.parse_expression_in_parse(
            transaction,
            context.clone(),
            ExpressionParseRequest {
                source: &source,
                range: ParserTextRange::new(0, request.input.len()),
                expected_types: request
                    .expected_types
                    .iter()
                    .map(|expected| skript_parser::ExpressionExpectedType {
                        class_name: ClassName(expected.class_name.clone()),
                        plural: expected.plural,
                    })
                    .collect(),
                context: parse_request_context(request, context).map_err(|message| {
                    HostError::InvalidParseResult {
                        parser_id: request.parser_id.clone(),
                        message,
                    }
                })?,
            },
            ExpressionParserConfig {
                root_mode: parse_request_root_mode(request).map_err(|message| {
                    HostError::InvalidParseResult {
                        parser_id: request.parser_id.clone(),
                        message,
                    }
                })?,
                ..ExpressionParserConfig::default()
            },
        )?;
        let Some(selected) = parsed.matches.selected else {
            let message = parsed.matches.failure.as_ref().map_or_else(
                || "input is not a valid Expression".to_owned(),
                |failure| format!("Expression parse failed: {:?}", failure.kind),
            );
            return Ok(ExecutedParseResult {
                wire: failed_parse_result(request, "parser.expression-no-match", &message),
                expression_roots: BTreeMap::new(),
            });
        };
        let wire = expression_parse_result(
            request,
            &selected.node,
            self.config.syntax_catalog.as_deref(),
        );
        let root_id = *wire
            .roots
            .first()
            .expect("successful Expression results have one root");
        let child = rebase_expression_node(selected.node, request)?;
        Ok(ExecutedParseResult {
            wire,
            expression_roots: BTreeMap::from([(root_id, child)]),
        })
    }

    fn execute_condition_parse_request(
        &mut self,
        transaction: &ParseTransaction,
        context: &InvocationContext,
        request: &ParseRequest,
    ) -> Result<ExecutedParseResult, HostError> {
        let source = MappedSource::identity(request.input.clone());
        let parsed = self.parse_condition_in_parse(
            transaction,
            context.clone(),
            ConditionParseRequest {
                source: &source,
                range: ParserTextRange::new(0, request.input.len()),
                context: parse_request_context(request, context).map_err(|message| {
                    HostError::InvalidParseResult {
                        parser_id: request.parser_id.clone(),
                        message,
                    }
                })?,
            },
            ConditionParserConfig::default(),
        )?;
        let Some(selected) = parsed.matches.selected else {
            return Ok(ExecutedParseResult {
                wire: failed_parse_result(
                    request,
                    "parser.condition-no-match",
                    "input is not a valid Condition",
                ),
                expression_roots: BTreeMap::new(),
            });
        };
        Ok(ExecutedParseResult {
            wire: condition_parse_result(
                request,
                &selected.node,
                self.config.syntax_catalog.as_deref(),
            ),
            expression_roots: BTreeMap::new(),
        })
    }

    fn execute_addon_parse_request(
        &mut self,
        transaction: &ParseTransaction,
        context: &InvocationContext,
        request: &ParseRequest,
    ) -> Result<ExecutedParseResult, HostError> {
        let mut result = self.dispatch_with_transaction(
            transaction,
            DispatchRequest {
                context: context.clone(),
                target: DispatchTarget::Parser(request.parser_id.clone()),
                phase: HookPhase::Parser,
                payload: HookPayload::Parser(request.clone()),
            },
        )?;
        if !matches!(result.decision, HookDecision::Handled) {
            if !result.effects.parse_results.is_empty() {
                return Err(HostError::InvalidParseResult {
                    parser_id: request.parser_id.clone(),
                    message: "a parser returned results without handling the request".to_owned(),
                });
            }
            return Ok(ExecutedParseResult {
                wire: failed_parse_result(
                    request,
                    "parser.unhandled",
                    "no registered parser handled the request",
                ),
                expression_roots: BTreeMap::new(),
            });
        }
        if result.effects.parse_results.len() != 1 {
            return Err(HostError::InvalidParseResult {
                parser_id: request.parser_id.clone(),
                message: format!(
                    "a handled request must return exactly one result, got {}",
                    result.effects.parse_results.len()
                ),
            });
        }
        let parsed = result
            .effects
            .parse_results
            .pop()
            .expect("length was checked");
        validate_parse_result(request, &parsed)?;
        Ok(ExecutedParseResult {
            wire: parsed,
            expression_roots: BTreeMap::new(),
        })
    }

    fn dispatch_with_transaction(
        &mut self,
        transaction: &ParseTransaction,
        request: DispatchRequest,
    ) -> Result<DispatchResult, HostError> {
        let capability_id = match request.phase {
            HookPhase::Expression => CAPABILITY_EXPRESSION_PARSER,
            HookPhase::Condition => CAPABILITY_CONDITION_PARSER,
            HookPhase::Effect => CAPABILITY_EFFECT_PARSER,
            HookPhase::Section => CAPABILITY_SECTION_PARSER,
            HookPhase::Structure => CAPABILITY_STRUCTURE_PARSER,
            HookPhase::Parser => CAPABILITY_ADDITIONAL_PARSE,
            _ => CAPABILITY_HOOKS,
        };
        let candidates =
            self.registry
                .matching_capability(&request.target, request.phase, capability_id);
        let document_id = transaction.document_id()?;
        let document_revision = transaction.document_revision()?;
        let mut payload = request.payload;
        apply_catalog_annotations(&self.components, &request.target, &mut payload);
        let mut effects = empty_effects();
        let mut calls = Vec::new();
        let mut failures = Vec::new();
        let mut available_parse_results = BTreeMap::new();
        let mut generated_output = 0usize;
        let mut decision = HookDecision::ContinueProcessing;

        'candidate: for candidate in candidates {
            if self.components[candidate.component_index].disabled
                || self.components[candidate.component_index].unloaded
            {
                continue;
            }
            if selector_match(
                &candidate.subscription.selector,
                &payload,
                self.config.syntax_catalog.as_deref(),
            ) == SelectorMatch::NoMatch
            {
                continue;
            }

            let component_id = self.components[candidate.component_index]
                .manifest
                .component_id
                .clone();
            let subscription_id = candidate.subscription.id.clone();
            let mut parse_results = Vec::new();
            let mut candidate_parse_results = BTreeMap::new();
            let mut continuation_base = None;
            let (output, state_invocation, dynamic_update) = loop {
                if calls.len() >= self.config.max_calls_per_dispatch {
                    return Err(HostError::CallQuotaExceeded {
                        limit: self.config.max_calls_per_dispatch,
                    });
                }
                let mut context = request.context.clone();
                context.subscription_id = subscription_id.clone();
                let invocation =
                    crate::bindings::nlaocs::skript_parser_addon::types::HookInvocation {
                        context,
                        target: candidate.subscription.target.clone(),
                        phase: request.phase,
                        payload: payload.clone(),
                        parse_results: parse_results.clone(),
                    };
                calls.push(HookCall {
                    component_id: component_id.clone(),
                    subscription_id: subscription_id.clone(),
                });
                let state_invocation = transaction.begin_invocation(component_id.clone())?;
                let dynamic_update = if is_dynamic_prepass_phase(request.phase) {
                    self.dynamic_syntax_registry
                        .as_ref()
                        .map(|registry| {
                            registry.begin_document_update(
                                component_id.clone(),
                                self.components[candidate.component_index].load_order,
                                &document_id,
                                document_revision,
                            )
                        })
                        .transpose()?
                } else {
                    None
                };
                let call = {
                    let entry = &mut self.components[candidate.component_index];
                    if entry.store.data().invocation.is_some()
                        || entry.store.data().dynamic_syntax_update.is_some()
                    {
                        return Err(StateError::Internal {
                            message: format!(
                                "component {component_id} already has an active host transaction"
                            ),
                        }
                        .into());
                    }
                    entry.store.data_mut().invocation = Some(state_invocation);
                    entry.store.data_mut().dynamic_syntax_update = dynamic_update;
                    let prepared = prepare_store(
                        &mut entry.store,
                        self.config.fuel_per_call,
                        self.config.deadline_ticks(&component_id),
                        &component_id,
                        "hook",
                    );
                    if let Err(error) = prepared {
                        entry
                            .store
                            .data_mut()
                            .invocation
                            .take()
                            .expect("the invocation was just installed")
                            .rollback();
                        entry.store.data_mut().dynamic_syntax_update.take();
                        return Err(error);
                    }
                    let call = entry
                        .bindings
                        .nlaocs_skript_parser_addon_hooks()
                        .call_invoke(&mut entry.store, &invocation);
                    let state_invocation =
                        entry.store.data_mut().invocation.take().expect(
                            "the invocation remains installed for the duration of the call",
                        );
                    let dynamic_update = entry.store.data_mut().dynamic_syntax_update.take();
                    (call, state_invocation, dynamic_update)
                };
                let (call, state_invocation, dynamic_update) = call;
                let mut output = match call {
                    Ok(Ok(output)) => output,
                    Ok(Err(addon_error)) => {
                        state_invocation.rollback();
                        drop(dynamic_update);
                        if let Some(base) = continuation_base.as_ref() {
                            transaction.rollback_to(base)?;
                        }
                        effects.diagnostics.extend(addon_error.diagnostics);
                        failures.push(ComponentFailure {
                            component_id: component_id.clone(),
                            subscription_id: subscription_id.clone(),
                            error: HostError::AddonFailure {
                                component_id: component_id.clone(),
                                message: addon_error.message,
                            },
                        });
                        continue 'candidate;
                    }
                    Err(error) => {
                        state_invocation.rollback();
                        drop(dynamic_update);
                        if let Some(base) = continuation_base.as_ref() {
                            transaction.rollback_to(base)?;
                        }
                        let error = classify_wasmtime_error(component_id.clone(), "hook", error);
                        if error.disables_component() {
                            self.components[candidate.component_index].disabled = true;
                            if let Some(registry) = &self.dynamic_syntax_registry {
                                registry.remove_component(&component_id)?;
                            }
                        }
                        failures.push(ComponentFailure {
                            component_id: component_id.clone(),
                            subscription_id: subscription_id.clone(),
                            error,
                        });
                        continue 'candidate;
                    }
                };

                generated_output = generated_output.saturating_add(hook_output_size(&output));
                if generated_output > self.config.max_generated_output_bytes {
                    state_invocation.rollback();
                    drop(dynamic_update);
                    return Err(HostError::GeneratedOutputQuotaExceeded {
                        limit: self.config.max_generated_output_bytes,
                    });
                }

                if matches!(output.decision, HookDecision::NotApplicable) {
                    state_invocation.rollback();
                    drop(dynamic_update);
                    if let Some(base) = continuation_base.as_ref() {
                        transaction.rollback_to(base)?;
                    }
                    continue 'candidate;
                }

                stamp_parse_result_attachments(&mut output.effects, &component_id);

                if let Some(replacement) = output.replacement.as_mut()
                    && let Err(message) =
                        normalize_hook_metadata(&payload, replacement, &component_id)
                {
                    state_invocation.rollback();
                    drop(dynamic_update);
                    if let Some(base) = continuation_base.as_ref() {
                        transaction.rollback_to(base)?;
                    }
                    failures.push(ComponentFailure {
                        component_id: component_id.clone(),
                        subscription_id: subscription_id.clone(),
                        error: HostError::InvalidHookOutput {
                            component_id,
                            subscription_id,
                            message,
                        },
                    });
                    continue 'candidate;
                }

                if output.effects.parse_requests.is_empty() {
                    break (output, state_invocation, dynamic_update);
                }
                if calls
                    .iter()
                    .rev()
                    .take_while(|call| {
                        call.component_id == component_id && call.subscription_id == subscription_id
                    })
                    .count()
                    >= self.config.max_parser_rounds
                {
                    state_invocation.rollback();
                    drop(dynamic_update);
                    return Err(HostError::ParserRoundQuotaExceeded {
                        limit: self.config.max_parser_rounds,
                    });
                }
                let requests = mem::take(&mut output.effects.parse_requests);
                if requests.len() > self.config.max_parse_requests_per_hook {
                    state_invocation.rollback();
                    drop(dynamic_update);
                    return Err(HostError::ParseRequestQuotaExceeded {
                        limit: self.config.max_parse_requests_per_hook,
                    });
                }
                state_invocation.rollback();
                drop(dynamic_update);
                if continuation_base.is_none() {
                    continuation_base = Some(transaction.savepoint()?);
                }
                let parent_context = hook_payload_parse_context(&payload, &request.context)
                    .map_err(|message| HostError::InvalidParseResult {
                        parser_id: "context inheritance".to_owned(),
                        message,
                    })?;
                let next_parse_results = match self.execute_parse_requests(
                    transaction,
                    &request.context,
                    parent_context.as_ref(),
                    requests,
                ) {
                    Ok(results) => {
                        for result in &results {
                            candidate_parse_results.insert(result.wire.host_token, result.clone());
                        }
                        results
                            .into_iter()
                            .map(|result| result.wire)
                            .collect::<Vec<_>>()
                    }
                    Err(error) => {
                        transaction.rollback_to(
                            continuation_base
                                .as_ref()
                                .expect("the continuation savepoint was just created"),
                        )?;
                        return Err(error);
                    }
                };
                parse_results.extend(next_parse_results);
            };

            let previous_payload = payload.clone();
            match apply_hook_output(candidate.subscription.mode, output, payload.clone()) {
                Ok(applied) => {
                    let AppliedOutput {
                        payload: mut next_payload,
                        decision: next_decision,
                        effects: next_effects,
                        terminal,
                    } = applied;
                    normalize_structure_hook_payload(
                        &previous_payload,
                        &mut next_payload,
                        &next_effects,
                    )?;
                    if matches!(next_decision, Some(HookDecision::Reject(_))) {
                        state_invocation.rollback();
                        drop(dynamic_update);
                        if let Some(base) = continuation_base.as_ref() {
                            transaction.rollback_to(base)?;
                        }
                        if let Some(final_decision) = next_decision {
                            decision = final_decision;
                        }
                        break;
                    }
                    state_invocation.commit()?;
                    if let Some(update) = dynamic_update {
                        update.commit()?;
                    }
                    available_parse_results.append(&mut candidate_parse_results);
                    payload = next_payload;
                    merge_effects(&mut effects, next_effects);
                    if let Some(final_decision) = next_decision {
                        decision = final_decision;
                    }
                    if terminal {
                        break;
                    }
                }
                Err(message) => {
                    state_invocation.rollback();
                    drop(dynamic_update);
                    if let Some(base) = continuation_base.as_ref() {
                        transaction.rollback_to(base)?;
                    }
                    failures.push(ComponentFailure {
                        component_id: component_id.clone(),
                        subscription_id: subscription_id.clone(),
                        error: HostError::InvalidHookOutput {
                            component_id,
                            subscription_id,
                            message,
                        },
                    });
                }
            }
        }

        Ok(DispatchResult {
            decision,
            payload,
            effects,
            calls,
            failures,
            available_parse_results,
        })
    }

    fn load_component(
        &mut self,
        bytes: &[u8],
        mandatory_core: bool,
    ) -> Result<ComponentInfo, HostError> {
        if bytes.is_empty() && mandatory_core {
            return Err(HostError::CoreLibraryMissing);
        }
        let loading_id = if mandatory_core {
            CORE_LIBRARY_COMPONENT_ID
        } else {
            "<loading>"
        };
        let component = self.runtime.component(bytes, loading_id)?;
        let mut store = create_store(
            &self.runtime.engine,
            &self.config,
            self.type_user_input_matchers.clone(),
        );
        prepare_store(
            &mut store,
            self.config.fuel_per_call,
            self.config.deadline_ticks(loading_id),
            loading_id,
            "instantiate",
        )?;
        let bindings = ParserAddon::instantiate(&mut store, &component, &self.runtime.linker)
            .map_err(|error| {
                classify_component_error(loading_id.to_owned(), "instantiate", error)
            })?;

        prepare_store(
            &mut store,
            self.config.fuel_per_call,
            self.config.deadline_ticks(loading_id),
            loading_id,
            "manifest",
        )?;
        let mut manifest = bindings
            .nlaocs_skript_parser_addon_addon()
            .call_manifest(&mut store)
            .map_err(|error| classify_wasmtime_error(loading_id.to_owned(), "manifest", error))?;
        validate_manifest(&manifest, &self.capabilities)?;
        if let Some(catalog) = self.config.syntax_catalog.as_deref() {
            validate_manifest_catalog_bindings(&manifest, catalog)?;
        }
        stamp_catalog_annotation_owners(&mut manifest);

        if mandatory_core && manifest.component_id != CORE_LIBRARY_COMPONENT_ID {
            return Err(HostError::InvalidCoreLibrary {
                expected: CORE_LIBRARY_COMPONENT_ID.to_owned(),
                actual: manifest.component_id,
            });
        }
        if self
            .components
            .iter()
            .any(|entry| entry.manifest.component_id == manifest.component_id)
        {
            return Err(HostError::DuplicateComponent {
                component_id: manifest.component_id,
            });
        }

        prepare_store(
            &mut store,
            self.config.fuel_per_call,
            self.config.deadline_ticks(&manifest.component_id),
            &manifest.component_id,
            "initialize",
        )?;
        let load_order = self.components.len();
        store.data_mut().dynamic_syntax_update = self
            .dynamic_syntax_registry
            .as_ref()
            .map(|registry| {
                registry.begin_initial_update(manifest.component_id.clone(), load_order)
            })
            .transpose()?;
        let registered_handler_bindings =
            resolve_registered_handler_bindings(&manifest, self.config.syntax_catalog.as_deref());
        store.data_mut().registered_handler_bindings = registered_handler_bindings.clone();
        let profile = host_profile(
            &self.capabilities,
            &self.config.runtime_profile,
            &registered_handler_bindings,
        );
        let initialization = bindings
            .nlaocs_skript_parser_addon_addon()
            .call_initialize(&mut store, &profile)
            .map_err(|error| {
                classify_wasmtime_error(manifest.component_id.clone(), "initialize", error)
            });
        let dynamic_update = store.data_mut().dynamic_syntax_update.take();
        match initialization? {
            Ok(()) => {}
            Err(error) => {
                drop(dynamic_update);
                return Err(HostError::InitializationRejected {
                    component_id: manifest.component_id,
                    message: error.message,
                });
            }
        }
        let namespaces = namespace_declarations(&manifest);
        self.state_store
            .register_component(&manifest.component_id, &namespaces)
            .map_err(|error| HostError::InvalidManifest {
                message: format!(
                    "{} has invalid StateStore namespaces: {error}",
                    manifest.component_id
                ),
            })?;
        if let Some(update) = dynamic_update {
            update.commit()?;
        }

        let component_index = self.components.len();
        let info = ComponentInfo {
            component_id: manifest.component_id.clone(),
            component_version: manifest.component_version.clone(),
            load_order,
            disabled: false,
        };
        self.registry
            .register(component_index, load_order, &manifest.subscriptions);
        self.components.push(ComponentEntry {
            manifest,
            registered_handler_bindings,
            store,
            bindings,
            load_order,
            disabled: false,
            unloaded: false,
        });
        Ok(info)
    }
}

fn is_dynamic_prepass_phase(phase: HookPhase) -> bool {
    matches!(phase, HookPhase::Document | HookPhase::Preprocess)
}

fn namespace_declarations(manifest: &ComponentManifest) -> Vec<NamespaceDeclaration> {
    manifest
        .state_namespaces
        .iter()
        .map(|namespace| NamespaceDeclaration {
            name: namespace.name.clone(),
            visibility: namespace_visibility(namespace.visibility),
            schema_id: namespace.schema_id.clone(),
            schema_version: namespace.schema_version,
            readers: namespace.readers.iter().cloned().collect(),
            writers: namespace.writers.iter().cloned().collect(),
        })
        .collect()
}

fn state_scope(scope: WitStateScope) -> StateScope {
    match scope {
        WitStateScope::Invocation => StateScope::Invocation,
        WitStateScope::Parse => StateScope::Parse,
        WitStateScope::Document => StateScope::Document,
        WitStateScope::Project => StateScope::Project,
        WitStateScope::PersistentProject => StateScope::PersistentProject,
    }
}

fn namespace_visibility(visibility: WitNamespaceVisibility) -> NamespaceVisibility {
    match visibility {
        WitNamespaceVisibility::Private => NamespaceVisibility::Private,
        WitNamespaceVisibility::Shared => NamespaceVisibility::Shared,
    }
}

fn state_value(value: WitStateValue) -> StateValue {
    StateValue::new(
        value.schema_id,
        match value.encoding {
            WitStateEncoding::Raw => StateEncoding::Raw,
            WitStateEncoding::Cbor => StateEncoding::Cbor,
            WitStateEncoding::Json => StateEncoding::Json,
        },
        value.bytes,
    )
}

fn wit_state_value(value: StateValue) -> WitStateValue {
    WitStateValue {
        schema_id: value.schema_id,
        encoding: match value.encoding {
            StateEncoding::Raw => WitStateEncoding::Raw,
            StateEncoding::Cbor => WitStateEncoding::Cbor,
            StateEncoding::Json => WitStateEncoding::Json,
        },
        bytes: value.bytes,
    }
}

fn wit_state_error(error: StateError) -> WitStateError {
    let kind = match &error {
        StateError::NoActiveTransaction => WitStateErrorKind::NoActiveTransaction,
        StateError::InvalidInput { .. } => WitStateErrorKind::InvalidInput,
        StateError::UnknownNamespace { .. } => WitStateErrorKind::UnknownNamespace,
        StateError::AccessDenied { .. } => WitStateErrorKind::AccessDenied,
        StateError::SchemaMismatch { .. } => WitStateErrorKind::SchemaMismatch,
        StateError::QuotaExceeded { .. } => WitStateErrorKind::QuotaExceeded,
        StateError::StaleDocumentRevision { .. } => WitStateErrorKind::StaleDocumentRevision,
        StateError::TransactionConflict { .. } => WitStateErrorKind::TransactionConflict,
        StateError::Persistence { .. } => WitStateErrorKind::Persistence,
        StateError::TransactionClosed
        | StateError::ForeignSavepoint
        | StateError::Internal { .. } => WitStateErrorKind::Internal,
    };
    WitStateError {
        kind,
        message: error.to_string(),
    }
}

/// Returns the capabilities advertised by a host without a syntax catalog.
///
/// Dynamic syntax is advertised only by configured hosts that receive a Catalog.
pub fn host_capabilities() -> Vec<Capability> {
    [
        CAPABILITY_HOOKS,
        CAPABILITY_STATE_STORE,
        CAPABILITY_TEXT_MACRO,
        CAPABILITY_TREE_MACRO,
        CAPABILITY_CONTEXT_UPDATES,
        CAPABILITY_ADDITIONAL_PARSE,
        CAPABILITY_EXPRESSION_PARSER,
        CAPABILITY_CONDITION_PARSER,
        CAPABILITY_EFFECT_PARSER,
        CAPABILITY_SECTION_PARSER,
        CAPABILITY_STRUCTURE_PARSER,
    ]
    .map(|id| Capability::new(id, 1))
    .to_vec()
}

fn configured_host_capabilities(
    dynamic_syntax_available: bool,
    catalog_source_available: bool,
) -> Vec<Capability> {
    let mut capabilities = host_capabilities();
    if catalog_source_available {
        capabilities.push(Capability::new(CAPABILITY_CATALOG_DATA, 2));
    }
    if dynamic_syntax_available {
        capabilities.push(Capability::new(CAPABILITY_DYNAMIC_SYNTAX, 1));
    }
    capabilities
}

fn host_profile(
    capabilities: &[Capability],
    runtime: &RuntimeProfile,
    registered_handler_bindings: &[WitRegisteredHandlerBinding],
) -> crate::bindings::nlaocs::skript_parser_addon::types::HostProfile {
    use crate::bindings::nlaocs::skript_parser_addon::types::{
        AbiVersion as WitAbiVersion, Capability as WitCapability, HostProfile,
        RuntimePlugin as WitRuntimePlugin, RuntimeProfile as WitRuntimeProfile,
    };
    HostProfile {
        abi: WitAbiVersion {
            major: ABI_VERSION.major,
            minor: ABI_VERSION.minor,
        },
        capabilities: capabilities
            .iter()
            .map(|capability| WitCapability {
                id: capability.id.clone(),
                version: capability.version,
            })
            .collect(),
        runtime: WitRuntimeProfile {
            snapshot_schema_version: runtime.snapshot_schema_version,
            snapshot_id: runtime.snapshot_id.clone(),
            server_name: runtime.server_name.clone(),
            server_version: runtime.server_version.clone(),
            minecraft_version: runtime.minecraft_version.clone(),
            java_version: runtime.java_version.clone(),
            language: runtime.language.clone(),
            skript_version: runtime.skript_version.clone(),
            plugins: runtime
                .plugins
                .iter()
                .map(|plugin| WitRuntimePlugin {
                    load_order: u64::try_from(plugin.load_order).unwrap_or(u64::MAX),
                    name: plugin.name.clone(),
                    version: plugin.version.clone(),
                    main: plugin.main.clone(),
                })
                .collect(),
        },
        registered_handler_bindings: registered_handler_bindings.to_vec(),
    }
}

fn resolve_registered_handler_bindings(
    manifest: &ComponentManifest,
    catalog: Option<&Catalog>,
) -> Vec<WitRegisteredHandlerBinding> {
    manifest
        .registered_syntax_handlers
        .iter()
        .map(|handler| {
            let (definition_ids, registration_ids) = resolve_registered_handler_target(
                handler,
                catalog,
                manifest.component_id == CORE_LIBRARY_COMPONENT_ID,
            );
            WitRegisteredHandlerBinding {
                handler_id: handler.handler_id.clone(),
                definition_ids,
                registration_ids,
            }
        })
        .collect()
}

fn resolve_registered_handler_target(
    handler: &crate::bindings::nlaocs::skript_parser_addon::types::RegisteredSyntaxHandler,
    catalog: Option<&Catalog>,
    skript_owned_only: bool,
) -> (Vec<String>, Vec<String>) {
    let mut definition_ids = BTreeSet::new();
    let mut registration_ids = BTreeSet::new();
    for target in &handler.targets {
        match target {
            RegisteredSyntaxHandlerTarget::DynamicHandler(_) => {}
            RegisteredSyntaxHandlerTarget::Definition(id) => {
                definition_ids.insert(id.clone());
                if let Some(catalog) = catalog {
                    registration_ids.extend(
                        catalog
                            .syntaxes()
                            .iter()
                            .filter(|syntax| {
                                !skript_owned_only || syntax_is_owned_by_skript(syntax)
                            })
                            .filter(|syntax| syntax.kind() == catalog_syntax_kind(handler.kind))
                            .filter(|syntax| syntax.definition_id().as_str() == id)
                            .map(|syntax| syntax.registration_id().as_str().to_owned()),
                    );
                }
            }
            RegisteredSyntaxHandlerTarget::Registration(id) => {
                registration_ids.insert(id.clone());
                if let Some(catalog) = catalog {
                    definition_ids.extend(
                        catalog
                            .syntaxes()
                            .iter()
                            .filter(|syntax| {
                                !skript_owned_only || syntax_is_owned_by_skript(syntax)
                            })
                            .filter(|syntax| syntax.kind() == catalog_syntax_kind(handler.kind))
                            .filter(|syntax| syntax.registration_id().as_str() == id)
                            .map(|syntax| syntax.definition_id().as_str().to_owned()),
                    );
                }
            }
            RegisteredSyntaxHandlerTarget::ParserClass(parser_class) => {
                if let Some(catalog) = catalog {
                    for syntax in catalog.syntaxes().iter().filter(|syntax| {
                        (!skript_owned_only || syntax_is_owned_by_skript(syntax))
                            && syntax.kind() == catalog_syntax_kind(handler.kind)
                            && syntax_parser_class(syntax)
                                .is_some_and(|class| class.as_str() == parser_class)
                    }) {
                        definition_ids.insert(syntax.definition_id().as_str().to_owned());
                        registration_ids.insert(syntax.registration_id().as_str().to_owned());
                    }
                }
            }
            RegisteredSyntaxHandlerTarget::ClassSuffix(suffix) => {
                if let Some(catalog) = catalog {
                    for syntax in catalog.syntaxes().iter().filter(|syntax| {
                        (!skript_owned_only || syntax_is_owned_by_skript(syntax))
                            && syntax.kind() == catalog_syntax_kind(handler.kind)
                            && syntax_element_class(syntax)
                                .is_some_and(|class| class.as_str().ends_with(suffix))
                    }) {
                        definition_ids.insert(syntax.definition_id().as_str().to_owned());
                        registration_ids.insert(syntax.registration_id().as_str().to_owned());
                    }
                }
            }
            RegisteredSyntaxHandlerTarget::SuperClass(super_class) => {
                if let Some(catalog) = catalog {
                    for syntax in catalog.syntaxes().iter().filter(|syntax| {
                        (!skript_owned_only || syntax_is_owned_by_skript(syntax))
                            && syntax.kind() == catalog_syntax_kind(handler.kind)
                            && syntax_super_class(syntax)
                                .is_some_and(|class| class.as_str() == super_class)
                    }) {
                        definition_ids.insert(syntax.definition_id().as_str().to_owned());
                        registration_ids.insert(syntax.registration_id().as_str().to_owned());
                    }
                }
            }
        }
    }
    (
        definition_ids.into_iter().collect(),
        registration_ids.into_iter().collect(),
    )
}

fn syntax_is_owned_by_skript(syntax: &Syntax) -> bool {
    let addon = match syntax {
        Syntax::Event(value) => &value.common.addon,
        Syntax::Condition(value) => &value.common.addon,
        Syntax::Effect(value) => &value.common.addon,
        Syntax::Expression(value) => &value.common.addon,
        Syntax::Type(value) => &value.addon,
        Syntax::Function(value) => &value.addon,
        Syntax::Section(value) => &value.common.addon,
        Syntax::Structure(value) => &value.common.addon,
    };
    addon.name.eq_ignore_ascii_case("Skript")
}

fn validate_manifest(
    manifest: &ComponentManifest,
    host_capabilities: &[Capability],
) -> Result<(), HostError> {
    if manifest.component_id.trim().is_empty() {
        return Err(HostError::InvalidManifest {
            message: "component-id must not be blank".to_owned(),
        });
    }
    if manifest.component_id.contains('/') {
        return Err(HostError::InvalidManifest {
            message: format!(
                "{} uses the reserved '/' metadata namespace separator",
                manifest.component_id
            ),
        });
    }
    if manifest.component_version.trim().is_empty() {
        return Err(HostError::InvalidManifest {
            message: format!("{} has a blank component-version", manifest.component_id),
        });
    }

    let requirements = manifest
        .capabilities
        .iter()
        .map(|capability| CapabilityRequirement {
            id: capability.id.clone(),
            minimum_version: capability.minimum_version,
            required: capability.required,
        })
        .collect::<Vec<_>>();
    validate_compatibility(
        ABI_VERSION,
        AbiVersion::new(manifest.abi.major, manifest.abi.minor),
        &requirements,
        host_capabilities,
    )
    .map_err(|source| HostError::Compatibility {
        component_id: manifest.component_id.clone(),
        source,
    })?;

    let available = host_capabilities
        .iter()
        .map(|capability| (capability.id.as_str(), capability.version))
        .collect::<BTreeMap<_, _>>();
    let declared = manifest
        .capabilities
        .iter()
        .map(|capability| (capability.id.as_str(), capability.minimum_version))
        .collect::<BTreeMap<_, _>>();
    let mut subscription_ids = BTreeSet::new();
    for subscription in &manifest.subscriptions {
        if subscription.id.trim().is_empty() {
            return Err(HostError::InvalidManifest {
                message: format!("{} has a blank subscription ID", manifest.component_id),
            });
        }
        if let HookTarget::Parser(parser_id) = &subscription.target {
            if parser_id.trim().is_empty() {
                return Err(HostError::InvalidManifest {
                    message: format!("subscription {} has a blank parser ID", subscription.id),
                });
            }
            if subscription.phase != HookPhase::Parser
                || subscription.capability_id != CAPABILITY_ADDITIONAL_PARSE
                || subscription.mode != HookMode::Override
            {
                return Err(HostError::InvalidManifest {
                    message: format!(
                        "parser subscription {} must use the parser phase, additional-parse capability, and override mode",
                        subscription.id
                    ),
                });
            }
        } else if subscription.phase == HookPhase::Parser {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "parser-phase subscription {} must target a parser ID",
                    subscription.id
                ),
            });
        }
        if !subscription_ids.insert(subscription.id.as_str()) {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "{} declares subscription {} more than once",
                    manifest.component_id, subscription.id
                ),
            });
        }
        let Some(minimum) = declared.get(subscription.capability_id.as_str()) else {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "subscription {} uses undeclared capability {}",
                    subscription.id, subscription.capability_id
                ),
            });
        };
        if available
            .get(subscription.capability_id.as_str())
            .is_none_or(|actual| actual < minimum)
        {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "subscription {} uses unavailable capability {} version {}",
                    subscription.id, subscription.capability_id, minimum
                ),
            });
        }
        if subscription.capability_id == CAPABILITY_TEXT_MACRO
            && (!matches!(subscription.target, HookTarget::ParseStage)
                || !matches!(subscription.phase, HookPhase::Preprocess)
                || !matches!(subscription.mode, HookMode::Transform)
                || !selector_is_empty(&subscription.selector))
        {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "text macro subscription {} must target parse-stage in the preprocess phase with transform mode",
                    subscription.id
                ),
            });
        }
        if subscription.capability_id == CAPABILITY_TREE_MACRO
            && (!matches!(subscription.target, HookTarget::ParseStage)
                || !matches!(subscription.phase, HookPhase::Tree)
                || !matches!(subscription.mode, HookMode::Transform)
                || !selector_is_empty(&subscription.selector))
        {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "tree macro subscription {} must target parse-stage in the tree phase with transform mode",
                    subscription.id
                ),
            });
        }
        if subscription.capability_id == CAPABILITY_EXPRESSION_PARSER
            && (!matches!(
                subscription.target,
                HookTarget::ParseStage
                    | HookTarget::SyntaxKind(SyntaxKind::Expression)
                    | HookTarget::SyntaxKind(SyntaxKind::Type)
                    | HookTarget::Definition(_)
                    | HookTarget::Registration(_)
                    | HookTarget::Pattern(_)
            ) || !matches!(subscription.phase, HookPhase::Expression)
                || !matches!(subscription.mode, HookMode::Transform))
        {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "Expression parser subscription {} must target parse-stage, Expression syntax, or Type syntax in the Expression phase with transform mode",
                    subscription.id
                ),
            });
        }
        if subscription.capability_id == CAPABILITY_EFFECT_PARSER
            && (!matches!(subscription.phase, HookPhase::Effect)
                || !matches!(
                    subscription.target,
                    HookTarget::SyntaxKind(SyntaxKind::Effect)
                        | HookTarget::Definition(_)
                        | HookTarget::Registration(_)
                        | HookTarget::Pattern(_)
                ))
        {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "Effect parser subscription {} must target Effect syntax or an exact registration in the Effect phase",
                    subscription.id
                ),
            });
        }
        if subscription.capability_id == CAPABILITY_CONDITION_PARSER
            && (!matches!(subscription.phase, HookPhase::Condition)
                || !matches!(
                    subscription.target,
                    HookTarget::SyntaxKind(SyntaxKind::Condition)
                        | HookTarget::Definition(_)
                        | HookTarget::Registration(_)
                        | HookTarget::Pattern(_)
                ))
        {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "Condition parser subscription {} must target Condition syntax or an exact registration in the Condition phase",
                    subscription.id
                ),
            });
        }
        if subscription.capability_id == CAPABILITY_SECTION_PARSER
            && (!matches!(subscription.phase, HookPhase::Section)
                || !matches!(
                    subscription.target,
                    HookTarget::SyntaxKind(SyntaxKind::Section)
                        | HookTarget::Definition(_)
                        | HookTarget::Registration(_)
                        | HookTarget::Pattern(_)
                ))
        {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "Section parser subscription {} must target Section syntax or an exact registration in the Section phase",
                    subscription.id
                ),
            });
        }
        if subscription.capability_id == CAPABILITY_STRUCTURE_PARSER
            && (!matches!(subscription.phase, HookPhase::Structure)
                || !matches!(
                    subscription.target,
                    HookTarget::SyntaxKind(SyntaxKind::Structure)
                        | HookTarget::Definition(_)
                        | HookTarget::Registration(_)
                        | HookTarget::Pattern(_)
                ))
        {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "Structure parser subscription {} must target Structure syntax or an exact registration in the Structure phase",
                    subscription.id
                ),
            });
        }
        match &subscription.target {
            HookTarget::Definition(id) if id.trim().is_empty() => {
                return Err(HostError::InvalidManifest {
                    message: format!("subscription {} has a blank definition ID", subscription.id),
                });
            }
            HookTarget::Registration(id) if id.trim().is_empty() => {
                return Err(HostError::InvalidManifest {
                    message: format!(
                        "subscription {} has a blank registration ID",
                        subscription.id
                    ),
                });
            }
            HookTarget::Pattern(pattern) if pattern.registration_id.trim().is_empty() => {
                return Err(HostError::InvalidManifest {
                    message: format!(
                        "subscription {} has a blank pattern registration ID",
                        subscription.id
                    ),
                });
            }
            _ => {}
        }
        if let Some(return_type) = subscription.selector.return_type.as_ref()
            && return_type.class_name.trim().is_empty()
        {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "subscription {} has a blank selector return type",
                    subscription.id
                ),
            });
        }
        if subscription
            .selector
            .tags
            .iter()
            .any(|tag| tag.trim().is_empty())
        {
            return Err(HostError::InvalidManifest {
                message: format!("subscription {} has a blank selector tag", subscription.id),
            });
        }
        if subscription
            .selector
            .metadata
            .iter()
            .any(|entry| entry.key.trim().is_empty())
        {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "subscription {} has a blank selector metadata key",
                    subscription.id
                ),
            });
        }
    }
    let mut registered_handlers = BTreeSet::new();
    for handler in &manifest.registered_syntax_handlers {
        if handler.handler_id.trim().is_empty() {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "{} declares a blank registered syntax handler ID",
                    manifest.component_id
                ),
            });
        }
        if handler.targets.is_empty() {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "{} declares registered syntax handler {} without a target",
                    manifest.component_id, handler.handler_id,
                ),
            });
        }
        let mut target_keys = BTreeSet::new();
        let mut target_descriptions = Vec::with_capacity(handler.targets.len());
        for target in &handler.targets {
            let (target_kind, target_value) = match target {
                RegisteredSyntaxHandlerTarget::Definition(value) => ("definition", value.as_str()),
                RegisteredSyntaxHandlerTarget::Registration(value) => {
                    ("registration", value.as_str())
                }
                RegisteredSyntaxHandlerTarget::ParserClass(_)
                    if handler.kind != SyntaxKind::Type =>
                {
                    return Err(HostError::InvalidManifest {
                        message: format!(
                            "{} handler {} uses a parser-class target for a non-Type syntax",
                            manifest.component_id, handler.handler_id
                        ),
                    });
                }
                RegisteredSyntaxHandlerTarget::ParserClass(value) => {
                    ("parser class", value.as_str())
                }
                RegisteredSyntaxHandlerTarget::ClassSuffix(value) => {
                    ("class suffix", value.as_str())
                }
                RegisteredSyntaxHandlerTarget::SuperClass(value) => ("superclass", value.as_str()),
                RegisteredSyntaxHandlerTarget::DynamicHandler(value) => {
                    ("dynamic handler", value.as_str())
                }
            };
            if target_value.trim().is_empty() {
                return Err(HostError::InvalidManifest {
                    message: format!(
                        "{} declares a blank registered syntax {target_kind}",
                        manifest.component_id,
                    ),
                });
            }
            if !target_keys.insert(format!("{target_kind}\0{target_value}")) {
                return Err(HostError::InvalidManifest {
                    message: format!(
                        "{} repeats registered syntax {target_kind} {target_value}",
                        manifest.component_id,
                    ),
                });
            }
            target_descriptions.push(format!("{target_kind} {target_value}"));
        }
        let target_description = target_descriptions.join(", ");
        let kind = catalog_syntax_kind(handler.kind);
        if handler
            .pattern_sources
            .iter()
            .any(|source| source.trim().is_empty())
            || handler
                .required_tags
                .iter()
                .chain(&handler.forbidden_tags)
                .any(|tag| tag.trim().is_empty())
        {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "{} declares a blank pattern source for handler {}",
                    manifest.component_id, handler.handler_id
                ),
            });
        }
        if handler
            .pattern_indices
            .iter()
            .collect::<BTreeSet<_>>()
            .len()
            != handler.pattern_indices.len()
            || handler
                .pattern_sources
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != handler.pattern_sources.len()
            || handler.required_tags.iter().collect::<BTreeSet<_>>().len()
                != handler.required_tags.len()
            || handler.forbidden_tags.iter().collect::<BTreeSet<_>>().len()
                != handler.forbidden_tags.len()
            || handler.marks.iter().collect::<BTreeSet<_>>().len() != handler.marks.len()
        {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "{} repeats a pattern predicate for handler {}",
                    manifest.component_id, handler.handler_id
                ),
            });
        }
        if handler
            .required_tags
            .iter()
            .any(|tag| handler.forbidden_tags.contains(tag))
        {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "{} both requires and forbids a tag for handler {}",
                    manifest.component_id, handler.handler_id
                ),
            });
        }
        let mut context_requirements = BTreeSet::new();
        for requirement in &handler.context_requirements {
            if requirement.trim().is_empty() {
                return Err(HostError::InvalidManifest {
                    message: format!(
                        "{} declares a blank context requirement for handler {}",
                        manifest.component_id, target_description
                    ),
                });
            }
            if requirement != REGISTERED_CONTEXT_ALL_TYPE_OPTIONS {
                return Err(HostError::InvalidManifest {
                    message: format!(
                        "{} declares unsupported context requirement {} for handler {}",
                        manifest.component_id, requirement, target_description
                    ),
                });
            }
            if !context_requirements.insert(requirement.as_str()) {
                return Err(HostError::InvalidManifest {
                    message: format!(
                        "{} repeats context requirement {} for handler {}",
                        manifest.component_id, requirement, target_description
                    ),
                });
            }
        }
        let mut bindings = BTreeSet::new();
        for binding in &handler.capture_parsers {
            if binding.parser_id.trim().is_empty() {
                return Err(HostError::InvalidManifest {
                    message: format!(
                        "{} declares a blank parser ID for handler {}",
                        manifest.component_id, target_description
                    ),
                });
            }
            if !bindings.insert((binding.capture_index, binding.parser_id.as_str())) {
                return Err(HostError::InvalidManifest {
                    message: format!(
                        "{} repeats parser {} for capture {} in handler {}",
                        manifest.component_id,
                        binding.parser_id,
                        binding.capture_index,
                        target_description
                    ),
                });
            }
            let mut option_keys = BTreeSet::new();
            if let Some(option) = binding
                .options
                .iter()
                .find(|option| !option_keys.insert(option.key.as_str()))
            {
                return Err(HostError::InvalidManifest {
                    message: format!(
                        "{} repeats option {} for parser {} in handler {}",
                        manifest.component_id, option.key, binding.parser_id, target_description
                    ),
                });
            }
        }
        if !registered_handlers.insert(handler.handler_id.as_str()) {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "{} declares registered syntax handler ID {} more than once",
                    manifest.component_id, handler.handler_id
                ),
            });
        }
        let has_subscription = manifest
            .subscriptions
            .iter()
            .any(|subscription| match kind {
                CatalogSyntaxKind::Expression | CatalogSyntaxKind::Type => {
                    subscription.capability_id == CAPABILITY_EXPRESSION_PARSER
                        && subscription.phase == HookPhase::Expression
                        && matches!(subscription.mode, HookMode::Transform)
                }
                CatalogSyntaxKind::Condition => {
                    subscription.capability_id == CAPABILITY_CONDITION_PARSER
                        && subscription.phase == HookPhase::Condition
                }
                CatalogSyntaxKind::Effect => {
                    subscription.capability_id == CAPABILITY_EFFECT_PARSER
                        && subscription.phase == HookPhase::Effect
                }
                CatalogSyntaxKind::Section => {
                    subscription.capability_id == CAPABILITY_SECTION_PARSER
                        && subscription.phase == HookPhase::Section
                }
                CatalogSyntaxKind::Structure => {
                    subscription.capability_id == CAPABILITY_STRUCTURE_PARSER
                        && subscription.phase == HookPhase::Structure
                }
                _ => false,
            });
        if !has_subscription {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "{} declares registered syntax handler {} without its parser subscription",
                    manifest.component_id, target_description
                ),
            });
        }
    }
    let mut annotation_keys = BTreeSet::new();
    for annotation in &manifest.catalog_annotations {
        let (target_kind, target_id, pattern_index) = match &annotation.target {
            CatalogAnnotationTarget::Definition(id) => ("definition", id.as_str(), None),
            CatalogAnnotationTarget::Registration(id) => ("registration", id.as_str(), None),
            CatalogAnnotationTarget::Pattern(pattern) => (
                "pattern",
                pattern.registration_id.as_str(),
                Some(pattern.pattern_index),
            ),
        };
        if target_id.trim().is_empty() {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "{} declares a catalog annotation with a blank {target_kind} ID",
                    manifest.component_id
                ),
            });
        }
        let mut local_keys = BTreeSet::new();
        for metadata in &annotation.metadata {
            if metadata.key.trim().is_empty() {
                return Err(HostError::InvalidManifest {
                    message: format!(
                        "{} declares a blank metadata key for catalog {target_kind} {target_id}",
                        manifest.component_id
                    ),
                });
            }
            if metadata.owner_component_id.is_some() {
                return Err(HostError::InvalidManifest {
                    message: format!(
                        "{} must not set catalog metadata ownership",
                        manifest.component_id
                    ),
                });
            }
            if !local_keys.insert(metadata.key.as_str())
                || !annotation_keys.insert((
                    target_kind,
                    target_id,
                    pattern_index,
                    metadata.key.as_str(),
                ))
            {
                return Err(HostError::InvalidManifest {
                    message: format!(
                        "{} repeats catalog metadata {} for {target_kind} {target_id}",
                        manifest.component_id, metadata.key
                    ),
                });
            }
        }
    }
    Ok(())
}

fn stamp_catalog_annotation_owners(manifest: &mut ComponentManifest) {
    let component_id = manifest.component_id.clone();
    for annotation in &mut manifest.catalog_annotations {
        for metadata in &mut annotation.metadata {
            metadata.owner_component_id = Some(component_id.clone());
        }
    }
}

fn selector_is_empty(selector: &HookSelector) -> bool {
    selector.pattern_index.is_none()
        && selector.pattern_source.is_none()
        && selector.mark.is_none()
        && selector.tags.is_empty()
        && selector.captures.is_empty()
        && selector.return_type.is_none()
        && selector.multiplicity.is_none()
        && selector.metadata.is_empty()
}

fn validate_manifest_catalog_bindings(
    manifest: &ComponentManifest,
    catalog: &Catalog,
) -> Result<(), HostError> {
    for subscription in &manifest.subscriptions {
        let registration_id = match &subscription.target {
            HookTarget::Registration(id) => Some(id.as_str()),
            HookTarget::Pattern(pattern) => Some(pattern.registration_id.as_str()),
            _ => None,
        };
        if let Some(registration_id) = registration_id
            && catalog.syntax_by_registration_id(registration_id).len() > 1
        {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "subscription {} targets ambiguous registration ID {}",
                    subscription.id, registration_id
                ),
            });
        }
    }

    for annotation in &manifest.catalog_annotations {
        let registration_id = match &annotation.target {
            CatalogAnnotationTarget::Registration(id) => Some(id.as_str()),
            CatalogAnnotationTarget::Pattern(pattern) => Some(pattern.registration_id.as_str()),
            CatalogAnnotationTarget::Definition(_) => None,
        };
        if let Some(registration_id) = registration_id
            && catalog.syntax_by_registration_id(registration_id).len() > 1
        {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "catalog annotation targets ambiguous registration ID {}",
                    registration_id
                ),
            });
        }
    }

    for handler in &manifest.registered_syntax_handlers {
        for target in &handler.targets {
            match target {
                RegisteredSyntaxHandlerTarget::Registration(registration_id)
                    if catalog.syntax_by_registration_id(registration_id).len() > 1 =>
                {
                    return Err(HostError::InvalidManifest {
                        message: format!(
                            "{} handler targets ambiguous registration ID {}",
                            manifest.component_id, registration_id
                        ),
                    });
                }
                RegisteredSyntaxHandlerTarget::ClassSuffix(suffix) => {
                    let kind = catalog_syntax_kind(handler.kind);
                    let definitions = catalog
                        .syntaxes()
                        .iter()
                        .filter(|syntax| syntax.kind() == kind)
                        .filter_map(|syntax| {
                            syntax_element_class(syntax)
                                .filter(|element_class| element_class.as_str().ends_with(suffix))
                                .map(|_| syntax.definition_id().as_str())
                        })
                        .collect::<BTreeSet<_>>();
                    if definitions.len() > 1 {
                        return Err(HostError::InvalidManifest {
                            message: format!(
                                "{} handler suffix {} ambiguously matches {} definition IDs",
                                manifest.component_id,
                                suffix,
                                definitions.len()
                            ),
                        });
                    }
                }
                RegisteredSyntaxHandlerTarget::Definition(_)
                | RegisteredSyntaxHandlerTarget::DynamicHandler(_)
                | RegisteredSyntaxHandlerTarget::ParserClass(_)
                | RegisteredSyntaxHandlerTarget::Registration(_)
                | RegisteredSyntaxHandlerTarget::SuperClass(_) => {}
            }
        }
    }
    Ok(())
}

fn syntax_element_class(syntax: &syntaxes::Syntax) -> Option<&ClassName> {
    match syntax {
        syntaxes::Syntax::Event(value) => Some(&value.common.element_class),
        syntaxes::Syntax::Condition(value) => Some(&value.common.element_class),
        syntaxes::Syntax::Effect(value) => Some(&value.common.element_class),
        syntaxes::Syntax::Expression(value) => Some(&value.common.element_class),
        syntaxes::Syntax::Type(value) => Some(&value.original_class),
        syntaxes::Syntax::Function(_) => None,
        syntaxes::Syntax::Section(value) => Some(&value.common.element_class),
        syntaxes::Syntax::Structure(value) => Some(&value.common.element_class),
    }
}

fn syntax_parser_class(syntax: &syntaxes::Syntax) -> Option<&ClassName> {
    match syntax {
        syntaxes::Syntax::Type(value) => value.parser_class.as_ref(),
        _ => None,
    }
}

fn syntax_super_class(syntax: &syntaxes::Syntax) -> Option<&ClassName> {
    match syntax {
        syntaxes::Syntax::Event(value) => value.common.super_class.as_ref(),
        syntaxes::Syntax::Condition(value) => value.common.super_class.as_ref(),
        syntaxes::Syntax::Effect(value) => value.common.super_class.as_ref(),
        syntaxes::Syntax::Expression(value) => value.common.super_class.as_ref(),
        syntaxes::Syntax::Type(value) => value.super_class.as_ref(),
        syntaxes::Syntax::Function(_) => None,
        syntaxes::Syntax::Section(value) => value.common.super_class.as_ref(),
        syntaxes::Syntax::Structure(value) => value.common.super_class.as_ref(),
    }
}

fn build_type_user_input_matchers(catalog: Option<&Catalog>) -> Arc<[TypeUserInputMatcher]> {
    let Some(catalog) = catalog else {
        return Arc::from([]);
    };
    let options: Arc<[WitExpressionTypeOption]> = catalog
        .types()
        .map(|value| expression_type_option(Some(catalog), value))
        .collect::<Vec<_>>()
        .into();
    let cache_key = catalog.source().map(|source| source.source_digest.clone());
    if let Some(key) = cache_key.as_ref() {
        let cache = TYPE_USER_INPUT_MATCHER_CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(matchers) = cache.get(key).and_then(|variants| {
            variants
                .iter()
                .find(|cached| same_expression_type_options(&cached.options, &options))
        }) {
            return matchers.matchers.clone();
        }
    }
    let matchers: Arc<[TypeUserInputMatcher]> = options
        .iter()
        .filter_map(|option| {
            let patterns = option
                .user_input_patterns
                .iter()
                .filter_map(|source| Regex::new(&format!("(?i)^(?:{source})$")).ok())
                .collect::<Vec<_>>();
            (!patterns.is_empty()).then(|| TypeUserInputMatcher {
                option: option.clone(),
                patterns,
            })
        })
        .collect::<Vec<_>>()
        .into();
    if let Some(key) = cache_key {
        let mut cache = TYPE_USER_INPUT_MATCHER_CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache
            .entry(key)
            .or_default()
            .push(CachedTypeUserInputMatchers {
                options,
                matchers: matchers.clone(),
            });
    }
    matchers
}

fn create_store(
    engine: &Engine,
    config: &HostConfig,
    type_user_input_matchers: Arc<[TypeUserInputMatcher]>,
) -> Store<StoreData> {
    let mut store = Store::new(
        engine,
        StoreData {
            limits: HostResourceLimiter {
                memory_bytes: config.max_memory_bytes,
                table_elements: config.max_table_elements,
                instances: config.max_instances_per_component,
                tables: config.max_tables_per_component,
                memories: config.max_memories_per_component,
            },
            invocation: None,
            dynamic_syntax_update: None,
            dynamic_syntax_available: config.syntax_catalog.is_some(),
            catalog: config.syntax_catalog.clone(),
            registered_handler_bindings: Vec::new(),
            language_patterns: HashMap::new(),
            type_user_input_matchers,
            max_catalog_response_bytes: config.max_catalog_response_bytes,
        },
    );
    store.limiter(|data| &mut data.limits);
    store
}

fn prepare_store(
    store: &mut Store<StoreData>,
    fuel: u64,
    deadline_ticks: u64,
    component_id: &str,
    operation: &'static str,
) -> Result<(), HostError> {
    store
        .set_fuel(fuel)
        .map_err(|error| classify_wasmtime_error(component_id.to_owned(), operation, error))?;
    store.set_epoch_deadline(deadline_ticks);
    Ok(())
}

fn classify_component_error(
    component_id: String,
    operation: &'static str,
    error: wasmtime::Error,
) -> HostError {
    if error.downcast_ref::<Trap>().is_some() {
        classify_wasmtime_error(component_id, operation, error)
    } else if operation == "compile" {
        HostError::ComponentCompile {
            component_id,
            message: format!("{error:#}"),
        }
    } else {
        HostError::ComponentInstantiation {
            component_id,
            message: format!("{error:#}"),
        }
    }
}

fn classify_wasmtime_error(
    component_id: String,
    operation: &'static str,
    error: wasmtime::Error,
) -> HostError {
    if error.downcast_ref::<GuestResourceLimit>().is_some() {
        return HostError::ResourceLimit {
            component_id,
            operation,
            message: error.to_string(),
        };
    }
    match error.downcast_ref::<Trap>() {
        Some(Trap::Interrupt) => HostError::Timeout {
            component_id,
            operation,
        },
        Some(Trap::OutOfFuel) => HostError::FuelExhausted {
            component_id,
            operation,
        },
        Some(Trap::AllocationTooLarge | Trap::MemoryOutOfBounds | Trap::TableOutOfBounds) => {
            HostError::ResourceLimit {
                component_id,
                operation,
                message: error.to_string(),
            }
        }
        Some(_) => HostError::Trap {
            component_id,
            operation,
            message: error.to_string(),
        },
        None => HostError::Runtime {
            component_id,
            operation,
            message: error.to_string(),
        },
    }
}

#[derive(Debug)]
struct AppliedOutput {
    payload: HookPayload,
    decision: Option<HookDecision>,
    effects: HookEffects,
    terminal: bool,
}

fn normalize_hook_metadata(
    original: &HookPayload,
    replacement: &mut HookPayload,
    component_id: &str,
) -> Result<(), String> {
    match (original, replacement) {
        (HookPayload::Matching(original), HookPayload::Matching(replacement)) => {
            merge_owned_metadata(&original.metadata, &mut replacement.metadata, component_id)
        }
        (
            HookPayload::RegisteredExpression(original),
            HookPayload::RegisteredExpression(replacement),
        ) => {
            if !same_registered_expression_identity(replacement, original) {
                return Err(
                    "registered Expression hook changed immutable request fields".to_owned(),
                );
            }
            validate_selected_property_options(replacement)?;
            public_data::validate(&replacement.public_data)?;
            merge_owned_metadata(&original.metadata, &mut replacement.metadata, component_id)
        }
        (HookPayload::Expression(original), HookPayload::Expression(replacement)) => {
            for candidate in &mut replacement.candidates {
                public_data::validate(&candidate.public_data)?;
                let previous = original.candidates.iter().find(|previous| {
                    previous.parser_id == candidate.parser_id
                        && previous.range.start == candidate.range.start
                        && previous.range.end == candidate.range.end
                });
                merge_owned_metadata(
                    previous.map_or(&[], |previous| previous.metadata.as_slice()),
                    &mut candidate.metadata,
                    component_id,
                )?;
            }
            Ok(())
        }
        (HookPayload::Condition(original), HookPayload::Condition(replacement)) => {
            merge_owned_metadata(
                &original.candidate.metadata,
                &mut replacement.candidate.metadata,
                component_id,
            )
        }
        (HookPayload::Effect(original), HookPayload::Effect(replacement)) => {
            if let Some(candidate) = replacement.candidate.as_mut() {
                let previous = original.candidate.as_ref().filter(|previous| {
                    previous.definition_id == candidate.definition_id
                        && previous.registration_id == candidate.registration_id
                        && previous.pattern_index == candidate.pattern_index
                });
                merge_owned_metadata(
                    previous.map_or(&[], |previous| previous.metadata.as_slice()),
                    &mut candidate.metadata,
                    component_id,
                )?;
            } else if let Some(candidate) = replacement.near_match.as_mut() {
                let previous = original.near_match.as_ref().filter(|previous| {
                    previous.definition_id == candidate.definition_id
                        && previous.registration_id == candidate.registration_id
                });
                merge_owned_metadata(
                    previous.map_or(&[], |previous| previous.metadata.as_slice()),
                    &mut candidate.metadata,
                    component_id,
                )?;
            }
            Ok(())
        }
        (HookPayload::Section(original), HookPayload::Section(replacement)) => {
            merge_owned_metadata(
                &original.candidate.metadata,
                &mut replacement.candidate.metadata,
                component_id,
            )
        }
        (HookPayload::Structure(original), HookPayload::Structure(replacement)) => {
            merge_owned_metadata(
                &original.candidate.metadata,
                &mut replacement.candidate.metadata,
                component_id,
            )
        }
        _ => Ok(()),
    }
}

fn merge_owned_metadata(
    original: &[WitMetadataEntry],
    replacement: &mut Vec<WitMetadataEntry>,
    component_id: &str,
) -> Result<(), String> {
    for entry in replacement.iter_mut() {
        if entry.key.trim().is_empty() {
            return Err("metadata keys must not be blank".to_owned());
        }
        match entry.owner_component_id.as_deref() {
            Some(owner) if owner != component_id => {
                let unchanged = original.iter().any(|previous| {
                    previous.owner_component_id.as_deref() == Some(owner)
                        && previous.key == entry.key
                        && previous.value == entry.value
                });
                if !unchanged {
                    return Err(format!(
                        "component {component_id} cannot write metadata owned by {owner}"
                    ));
                }
            }
            Some(_) => {}
            None => {
                let unchanged = original.iter().any(|previous| {
                    previous.owner_component_id.is_none()
                        && previous.key == entry.key
                        && previous.value == entry.value
                });
                if !unchanged {
                    entry.owner_component_id = Some(component_id.to_owned());
                }
            }
        }
    }

    for previous in original {
        let retained = replacement.iter().any(|entry| {
            entry.owner_component_id == previous.owner_component_id && entry.key == previous.key
        });
        if !retained {
            replacement.push(previous.clone());
        }
    }

    let mut keys = BTreeSet::new();
    if let Some(duplicate) = replacement
        .iter()
        .find(|entry| !keys.insert((entry.owner_component_id.as_deref(), entry.key.as_str())))
    {
        return Err(format!(
            "metadata key {} is repeated for owner {}",
            duplicate.key,
            duplicate
                .owner_component_id
                .as_deref()
                .unwrap_or("<catalog>")
        ));
    }
    Ok(())
}

fn apply_hook_output(
    mode: HookMode,
    output: HookOutput,
    mut payload: HookPayload,
) -> Result<AppliedOutput, String> {
    let HookOutput {
        decision,
        replacement,
        effects,
    } = output;
    if let Some(replacement) = replacement.as_ref()
        && mem::discriminant(replacement) != mem::discriminant(&payload)
    {
        return Err("replacement payload kind differs from the input payload".to_owned());
    }

    match mode {
        HookMode::Observe => {
            if replacement.is_some() {
                return Err("observe hooks cannot replace payloads".to_owned());
            }
            if !matches!(decision, HookDecision::ContinueProcessing) {
                return Err("observe hooks cannot control parser flow".to_owned());
            }
            Ok(AppliedOutput {
                payload,
                decision: None,
                effects,
                terminal: false,
            })
        }
        HookMode::Transform => {
            if matches!(decision, HookDecision::NotApplicable) {
                return Err("not-applicable must be handled before applying hook output".to_owned());
            }
            if matches!(decision, HookDecision::Handled) {
                return Err("transform hooks cannot mark input as handled".to_owned());
            }
            if let Some(replacement) = replacement {
                payload = replacement;
            }
            let terminal = matches!(decision, HookDecision::Reject(_));
            Ok(AppliedOutput {
                payload,
                decision: terminal.then_some(decision),
                effects,
                terminal,
            })
        }
        HookMode::Override => match decision {
            HookDecision::NotApplicable => {
                Err("not-applicable must be handled before applying hook output".to_owned())
            }
            HookDecision::ContinueProcessing => {
                if replacement.is_some() {
                    return Err("continuing override hooks cannot replace payloads".to_owned());
                }
                Ok(AppliedOutput {
                    payload,
                    decision: None,
                    effects,
                    terminal: false,
                })
            }
            decision @ (HookDecision::Handled | HookDecision::Reject(_)) => {
                if let Some(replacement) = replacement {
                    payload = replacement;
                }
                Ok(AppliedOutput {
                    payload,
                    decision: Some(decision),
                    effects,
                    terminal: true,
                })
            }
        },
    }
}

fn normalize_structure_hook_payload(
    previous: &HookPayload,
    next: &mut HookPayload,
    effects: &HookEffects,
) -> Result<(), HostError> {
    let (HookPayload::Structure(previous), HookPayload::Structure(next)) = (previous, next) else {
        return Ok(());
    };
    if !same_structure_payload_identity(next, previous)
        || previous.timing == WitStructureTiming::ExitBody
            && (next.candidate.body_mode != previous.candidate.body_mode
                || !next.candidate.declarations.is_empty())
    {
        return Err(StructureParseError::Environment {
            message: "Structure hook changed immutable candidate fields".to_owned(),
        }
        .into());
    }
    next.context = previous.context.clone();
    apply_wit_structure_context_updates(&mut next.context, &effects.context_updates)
        .map_err(|message| StructureParseError::Environment { message }.into())
}

fn apply_wit_structure_context_updates(
    context: &mut WitParseContext,
    updates: &[ContextUpdate],
) -> Result<(), String> {
    let mut values = context
        .values
        .drain(..)
        .map(|entry| (entry.key, entry.value))
        .collect::<BTreeMap<_, _>>();
    for update in updates {
        if update.syntax_context != context.syntax_context {
            continue;
        }
        if update.key == "parser.event-classes" {
            context.event_classes = update
                .value
                .as_deref()
                .map(std::str::from_utf8)
                .transpose()
                .map_err(|_| "Structure Event classes are not UTF-8".to_owned())?
                .map(|value| {
                    value
                        .split(';')
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            continue;
        }
        if let Some(value) = update.value.as_deref() {
            values.insert(
                update.key.clone(),
                std::str::from_utf8(value)
                    .map_err(|_| "Structure context update is not UTF-8".to_owned())?
                    .to_owned(),
            );
        } else {
            values.remove(&update.key);
        }
    }
    context.values = values
        .into_iter()
        .map(|(key, value)| WitParseContextValue { key, value })
        .collect();
    Ok(())
}

fn raw_node_at_path(tree: &ParserRawTree, path: &[usize]) -> Option<ParserRawNodeId> {
    let mut siblings = &tree.roots;
    let mut current = None;
    for index in path {
        let id = *siblings.get(*index)?;
        current = Some(id);
        siblings = &tree.get(id)?.children;
    }
    current
}

fn tree_macro_cycle_key(
    component_id: &str,
    subscription_id: &str,
    tree: &ParserRawTree,
    target: ParserRawNodeId,
) -> TreeMacroCycleKey {
    let node = tree
        .get(target)
        .expect("cycle keys are only built for resolved nodes");
    let mut original_ranges = node
        .span
        .origins
        .iter()
        .map(|origin| (origin.original_range.start, origin.original_range.end))
        .collect::<Vec<_>>();
    original_ranges.sort_unstable();
    original_ranges.dedup();

    TreeMacroCycleKey {
        component_id: component_id.to_owned(),
        subscription_id: subscription_id.to_owned(),
        original_ranges,
        fingerprint: raw_subtree_fingerprint(tree, target),
    }
}

fn raw_subtree_fingerprint(tree: &ParserRawTree, root: ParserRawNodeId) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut pending = vec![root];
    let mut visited = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            bytes.push(u8::MAX);
            continue;
        }
        let Some(node) = tree.get(id) else {
            bytes.push(u8::MAX - 1);
            bytes.extend_from_slice(&id.get().to_le_bytes());
            continue;
        };
        bytes.push(match node.kind {
            ParserRawNodeKind::Blank => 0,
            ParserRawNodeKind::Comment => 1,
            ParserRawNodeKind::Simple => 2,
            ParserRawNodeKind::Section => 3,
            ParserRawNodeKind::Invalid => 4,
        });
        bytes.extend_from_slice(&(node.text.len() as u64).to_le_bytes());
        bytes.extend_from_slice(node.text.as_bytes());
        bytes.extend_from_slice(&(node.children.len() as u64).to_le_bytes());
        pending.extend(node.children.iter().rev().copied());
    }
    bytes
}

fn raw_tree_depth(tree: &ParserRawTree) -> usize {
    let mut maximum = 0usize;
    let mut visited = BTreeSet::new();
    let mut pending = tree
        .roots
        .iter()
        .rev()
        .map(|id| (*id, 1usize))
        .collect::<Vec<_>>();
    while let Some((id, depth)) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        maximum = maximum.max(depth);
        if let Some(node) = tree.get(id) {
            pending.extend(
                node.children
                    .iter()
                    .rev()
                    .map(|child| (*child, depth.saturating_add(1))),
            );
        }
    }
    maximum
}

fn tree_macro_cycle_diagnostic(
    node: &skript_parser::RawNode,
    component_id: &str,
    subscription_id: &str,
) -> Diagnostic {
    Diagnostic {
        code: "tree-macro-cycle".to_owned(),
        message: format!(
            "tree macro {component_id}:{subscription_id} generated an expansion cycle"
        ),
        severity: DiagnosticSeverity::Error,
        span: mapped_span_to_wit(node.span.clone()),
        related: Vec::new(),
    }
}

fn parser_raw_tree_to_wit(tree: &ParserRawTree) -> RawTree {
    use crate::bindings::nlaocs::skript_parser_addon::types::{
        Indentation, RawDiagnostic, RawLine, RawRelatedSpan, RawTrivia, UnexpectedIndentation,
    };

    RawTree {
        roots: tree.roots.iter().map(|id| id.get()).collect(),
        nodes: tree
            .nodes
            .iter()
            .map(|node| RawTreeNode {
                id: node.id.get(),
                kind: match node.kind {
                    ParserRawNodeKind::Blank => WitRawNodeKind::Blank,
                    ParserRawNodeKind::Comment => WitRawNodeKind::Comment,
                    ParserRawNodeKind::Simple => WitRawNodeKind::Simple,
                    ParserRawNodeKind::Section => WitRawNodeKind::Section,
                    ParserRawNodeKind::Invalid => WitRawNodeKind::Invalid,
                },
                text: node.text.clone(),
                span: mapped_span_to_wit(node.span.clone()),
                line: RawLine {
                    number: node.line.number as u64,
                    raw_text: node.line.raw_text.clone(),
                    line_ending: match node.line.line_ending {
                        ParserLineEnding::None => WitLineEnding::None,
                        ParserLineEnding::Lf => WitLineEnding::Lf,
                        ParserLineEnding::CrLf => WitLineEnding::CrLf,
                        ParserLineEnding::Cr => WitLineEnding::Cr,
                    },
                    span: mapped_span_to_wit(node.line.span.clone()),
                    content_span: mapped_span_to_wit(node.line.content_span.clone()),
                    line_ending_span: mapped_span_to_wit(node.line.line_ending_span.clone()),
                    indentation: RawTrivia {
                        kind: WitRawTriviaKind::Whitespace,
                        text: node.line.indentation.text.clone(),
                        span: mapped_span_to_wit(node.line.indentation.span.clone()),
                    },
                    trailing_trivia: node
                        .line
                        .trailing_trivia
                        .iter()
                        .map(|trivia| RawTrivia {
                            kind: match trivia.kind {
                                ParserRawTriviaKind::Whitespace => WitRawTriviaKind::Whitespace,
                                ParserRawTriviaKind::LineComment => WitRawTriviaKind::LineComment,
                                ParserRawTriviaKind::BlockComment => WitRawTriviaKind::BlockComment,
                                ParserRawTriviaKind::LineEnding => WitRawTriviaKind::LineEnding,
                            },
                            text: trivia.text.clone(),
                            span: mapped_span_to_wit(trivia.span.clone()),
                        })
                        .collect(),
                },
                code_span: node.code_span.clone().map(mapped_span_to_wit),
                header_span: node.header_span.clone().map(mapped_span_to_wit),
                body_span: node.body_span.clone().map(mapped_span_to_wit),
                indent_level: node.indent_level,
                invalid_reason: node.invalid_reason.as_ref().map(|reason| match reason {
                    ParserRawInvalidReason::MixedIndentation => {
                        WitRawInvalidReason::MixedIndentation
                    }
                    ParserRawInvalidReason::InvalidIndentation => {
                        WitRawInvalidReason::InvalidIndentation
                    }
                    ParserRawInvalidReason::UnexpectedIndentation {
                        expected_level,
                        actual_level,
                    } => WitRawInvalidReason::UnexpectedIndentation(UnexpectedIndentation {
                        expected_level: *expected_level,
                        actual_level: *actual_level,
                    }),
                }),
                syntax_context: u64::from(node.syntax_context.get()),
                parent: node.parent.map(|id| id.get()),
                children: node.children.iter().map(|id| id.get()).collect(),
            })
            .collect(),
        diagnostics: tree
            .diagnostics
            .iter()
            .map(|diagnostic| RawDiagnostic {
                code: match diagnostic.code {
                    ParserRawDiagnosticCode::MixedIndentation => {
                        WitRawDiagnosticCode::MixedIndentation
                    }
                    ParserRawDiagnosticCode::InvalidIndentation => {
                        WitRawDiagnosticCode::InvalidIndentation
                    }
                    ParserRawDiagnosticCode::UnexpectedIndentation => {
                        WitRawDiagnosticCode::UnexpectedIndentation
                    }
                    ParserRawDiagnosticCode::EmptySection => WitRawDiagnosticCode::EmptySection,
                    ParserRawDiagnosticCode::UnclosedBlockComment => {
                        WitRawDiagnosticCode::UnclosedBlockComment
                    }
                },
                severity: match diagnostic.severity {
                    ParserRawDiagnosticSeverity::Error => WitRawDiagnosticSeverity::Error,
                    ParserRawDiagnosticSeverity::Warning => WitRawDiagnosticSeverity::Warning,
                },
                message: diagnostic.message.clone(),
                span: mapped_span_to_wit(diagnostic.span.clone()),
                related: diagnostic
                    .related
                    .iter()
                    .map(|related| RawRelatedSpan {
                        message: related.message.clone(),
                        span: mapped_span_to_wit(related.span.clone()),
                    })
                    .collect(),
            })
            .collect(),
        indentation: tree.indentation.as_ref().map(|indentation| Indentation {
            kind: match indentation.kind {
                ParserIndentKind::Space => WitIndentKind::Space,
                ParserIndentKind::Tab => WitIndentKind::Tab,
            },
            unit: indentation.unit.clone(),
        }),
    }
}

fn parser_raw_subtree_to_wit(tree: &ParserRawTree, root: ParserRawNodeId) -> RawTree {
    let Some(root_node) = tree.get(root) else {
        return RawTree {
            roots: Vec::new(),
            nodes: Vec::new(),
            diagnostics: Vec::new(),
            indentation: tree.indentation.as_ref().map(|indentation| {
                crate::bindings::nlaocs::skript_parser_addon::types::Indentation {
                    kind: match indentation.kind {
                        ParserIndentKind::Space => WitIndentKind::Space,
                        ParserIndentKind::Tab => WitIndentKind::Tab,
                    },
                    unit: indentation.unit.clone(),
                }
            }),
        };
    };
    let mut pending = vec![root];
    let mut included = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if included.insert(id)
            && let Some(node) = tree.get(id)
        {
            pending.extend(node.children.iter().copied());
        }
    }
    let subtree_end = root_node
        .body_span
        .as_ref()
        .map_or(root_node.span.virtual_range.end, |span| {
            span.virtual_range.end
        });
    let subtree_range = ParserTextRange::new(root_node.span.virtual_range.start, subtree_end);
    let subtree = ParserRawTree {
        roots: vec![root],
        nodes: tree
            .nodes
            .iter()
            .filter(|node| included.contains(&node.id))
            .cloned()
            .collect(),
        diagnostics: tree
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                let range = diagnostic.span.virtual_range;
                range.start >= subtree_range.start && range.end <= subtree_range.end
            })
            .cloned()
            .collect(),
        indentation: tree.indentation.clone(),
    };
    parser_raw_tree_to_wit(&subtree)
}

fn parser_tree_edit(edit: WitTreeEdit) -> ParserTreeEdit {
    match edit {
        WitTreeEdit::ReplaceNode(edit) => ParserTreeEdit::ReplaceNode {
            replacement: parser_generated_raw_tree(edit.replacement),
            retained_children: edit
                .retained_children
                .map(|retained| ParserRetainedChildren {
                    parent: ParserGeneratedRawNodeId::new(retained.target),
                    placement: match retained.placement {
                        WitRetainedChildrenPlacement::Prepend => {
                            ParserRetainedChildrenPlacement::Prepend
                        }
                        WitRetainedChildrenPlacement::Append => {
                            ParserRetainedChildrenPlacement::Append
                        }
                    },
                }),
        },
        WitTreeEdit::ReplaceChildren(replacement) => ParserTreeEdit::ReplaceChildren {
            replacement: parser_generated_raw_tree(replacement),
        },
    }
}

fn parser_generated_raw_tree(
    tree: crate::bindings::nlaocs::skript_parser_addon::types::GeneratedRawTree,
) -> ParserGeneratedRawTree {
    ParserGeneratedRawTree {
        roots: tree
            .roots
            .into_iter()
            .map(ParserGeneratedRawNodeId::new)
            .collect(),
        nodes: tree
            .nodes
            .into_iter()
            .map(|node| ParserGeneratedRawNode {
                id: ParserGeneratedRawNodeId::new(node.id),
                kind: match node.kind {
                    WitGeneratedRawNodeKind::Blank => ParserGeneratedRawNodeKind::Blank,
                    WitGeneratedRawNodeKind::Comment => ParserGeneratedRawNodeKind::Comment,
                    WitGeneratedRawNodeKind::Simple => ParserGeneratedRawNodeKind::Simple,
                    WitGeneratedRawNodeKind::Section => ParserGeneratedRawNodeKind::Section,
                },
                text: node.text,
                children: node
                    .children
                    .into_iter()
                    .map(ParserGeneratedRawNodeId::new)
                    .collect(),
            })
            .collect(),
    }
}

fn normalize_tree_macro_output_spans(
    source: &MappedSource,
    output: &mut TreeMacroOutput,
) -> Result<(), String> {
    normalize_text_macro_effects(source, &mut output.effects, "effects")?;
    if let HookDecision::Reject(rejection) = &mut output.decision {
        normalize_text_macro_diagnostics(
            source,
            &mut rejection.diagnostics,
            "rejection.diagnostics",
        )?;
    }
    Ok(())
}

fn mark_tree_macro_result_rolled_back(
    original_source: &MappedSource,
    original_tree: &ParserRawTree,
    result: &mut TreeMacroResult,
) {
    for call in &mut result.calls {
        call.accepted = false;
        call.expansion = None;
    }
    result.source = original_source.clone();
    result.tree = original_tree.clone();
    result.effects.context_updates.clear();
    result.effects.parse_requests.clear();
    retain_known_diagnostic_expansions(original_source, &mut result.effects.diagnostics);
    if let HookDecision::Reject(rejection) = &mut result.decision {
        retain_known_diagnostic_expansions(original_source, &mut rejection.diagnostics);
    }
}

fn tree_macro_output_size(output: &TreeMacroOutput) -> usize {
    output
        .edit
        .as_ref()
        .map_or(0, wit_tree_edit_size)
        .saturating_add(hook_effects_size(&output.effects))
        .saturating_add(match &output.decision {
            HookDecision::Reject(rejection) => rejection_size(rejection),
            HookDecision::NotApplicable
            | HookDecision::ContinueProcessing
            | HookDecision::Handled => 0,
        })
}

fn wit_tree_edit_depth(edit: &WitTreeEdit) -> usize {
    let tree = match edit {
        WitTreeEdit::ReplaceNode(edit) => &edit.replacement,
        WitTreeEdit::ReplaceChildren(tree) => tree,
    };
    let by_id = tree
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<BTreeMap<_, _>>();
    let mut maximum = 0usize;
    let mut visited = BTreeSet::new();
    let mut pending = tree
        .roots
        .iter()
        .rev()
        .map(|id| (*id, 1usize))
        .collect::<Vec<_>>();
    while let Some((id, depth)) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        maximum = maximum.max(depth);
        if let Some(node) = by_id.get(&id) {
            pending.extend(
                node.children
                    .iter()
                    .rev()
                    .map(|child| (*child, depth.saturating_add(1))),
            );
        }
    }
    maximum
}
fn wit_tree_edit_size(edit: &WitTreeEdit) -> usize {
    let tree = match edit {
        WitTreeEdit::ReplaceNode(edit) => &edit.replacement,
        WitTreeEdit::ReplaceChildren(tree) => tree,
    };
    tree.nodes
        .iter()
        .map(|node| {
            node.text
                .len()
                .saturating_add(node.children.len().saturating_mul(mem::size_of::<u64>()))
        })
        .fold(0usize, usize::saturating_add)
        .saturating_add(tree.roots.len().saturating_mul(mem::size_of::<u64>()))
}
fn parser_text_edits(edits: Vec<WitTextEdit>) -> Result<Vec<ParserTextEdit>, String> {
    edits
        .into_iter()
        .enumerate()
        .map(|(index, edit)| {
            let start = usize::try_from(edit.range.start)
                .map_err(|_| format!("text edit {index} start does not fit usize"))?;
            let end = usize::try_from(edit.range.end)
                .map_err(|_| format!("text edit {index} end does not fit usize"))?;
            let anchor = edit
                .anchor
                .map(usize::try_from)
                .transpose()
                .map_err(|_| format!("text edit {index} anchor does not fit usize"))?;
            Ok(ParserTextEdit {
                range: ParserTextRange::new(start, end),
                replacement: edit.replacement,
                anchor,
            })
        })
        .collect()
}

fn mapped_span_to_wit(span: skript_parser::MappedSpan) -> MappedSpan {
    MappedSpan {
        virtual_range: WitTextRange {
            start: span.virtual_range.start as u64,
            end: span.virtual_range.end as u64,
        },
        origins: span
            .origins
            .into_iter()
            .map(|origin| WitSourceOrigin {
                original_range: WitTextRange {
                    start: origin.original_range.start as u64,
                    end: origin.original_range.end as u64,
                },
                kind: match origin.kind {
                    ParserOriginKind::Exact => WitOriginKind::Exact,
                    ParserOriginKind::Replaced => WitOriginKind::Replaced,
                    ParserOriginKind::Anchored => WitOriginKind::Anchored,
                },
                expansion: origin.expansion.map(|id| u64::from(id.get())),
            })
            .collect(),
    }
}

fn normalize_text_macro_output_spans(
    source: &MappedSource,
    output: &mut TextMacroOutput,
) -> Result<(), String> {
    normalize_text_macro_effects(source, &mut output.effects, "effects")?;
    if let HookDecision::Reject(rejection) = &mut output.decision {
        normalize_text_macro_diagnostics(
            source,
            &mut rejection.diagnostics,
            "rejection.diagnostics",
        )?;
    }
    Ok(())
}

fn normalize_text_macro_effects(
    source: &MappedSource,
    effects: &mut HookEffects,
    path: &str,
) -> Result<(), String> {
    normalize_text_macro_diagnostics(
        source,
        &mut effects.diagnostics,
        &format!("{path}.diagnostics"),
    )?;
    for (request_index, request) in effects.parse_requests.iter_mut().enumerate() {
        request.span = normalize_text_macro_span(
            source,
            &request.span,
            &format!("{path}.parse-requests[{request_index}].span"),
        )?;
    }
    Ok(())
}

fn normalize_text_macro_diagnostics(
    source: &MappedSource,
    diagnostics: &mut [Diagnostic],
    path: &str,
) -> Result<(), String> {
    for (diagnostic_index, diagnostic) in diagnostics.iter_mut().enumerate() {
        diagnostic.span = normalize_text_macro_span(
            source,
            &diagnostic.span,
            &format!("{path}[{diagnostic_index}].span"),
        )?;
        for (related_index, related) in diagnostic.related.iter_mut().enumerate() {
            related.span = normalize_text_macro_span(
                source,
                &related.span,
                &format!("{path}[{diagnostic_index}].related[{related_index}].span"),
            )?;
        }
    }
    Ok(())
}

fn normalize_text_macro_span(
    source: &MappedSource,
    span: &MappedSpan,
    path: &str,
) -> Result<MappedSpan, String> {
    let start = usize::try_from(span.virtual_range.start)
        .map_err(|_| format!("{path} start does not fit usize"))?;
    let end = usize::try_from(span.virtual_range.end)
        .map_err(|_| format!("{path} end does not fit usize"))?;
    source
        .map_range(ParserTextRange::new(start, end))
        .map(mapped_span_to_wit)
        .map_err(|error| format!("{path}: {error}"))
}

fn mark_text_macro_result_rolled_back(
    original_source: &MappedSource,
    result: &mut TextMacroResult,
) {
    for call in &mut result.calls {
        call.accepted = false;
        call.expansion = None;
    }
    result.effects.context_updates.clear();
    result.effects.parse_requests.clear();
    retain_known_diagnostic_expansions(original_source, &mut result.effects.diagnostics);
    if let HookDecision::Reject(rejection) = &mut result.decision {
        retain_known_diagnostic_expansions(original_source, &mut rejection.diagnostics);
    }
}

fn retain_known_diagnostic_expansions(source: &MappedSource, diagnostics: &mut [Diagnostic]) {
    for diagnostic in diagnostics {
        retain_known_span_expansions(source, &mut diagnostic.span);
        for related in &mut diagnostic.related {
            retain_known_span_expansions(source, &mut related.span);
        }
    }
}

fn retain_known_span_expansions(source: &MappedSource, span: &mut MappedSpan) {
    for origin in &mut span.origins {
        let known = origin
            .expansion
            .and_then(|id| u32::try_from(id).ok())
            .map(ExpansionId::new)
            .is_some_and(|id| source.expansions().contains(id));
        if !known {
            origin.expansion = None;
        }
    }
}

const PARSE_CONTEXT_EVENT_CLASSES: &str = "context.event-classes";
const PARSE_CONTEXT_VALUE_PREFIX: &str = "context.value.";
const PARSE_MODE: &str = "parse.mode";

fn hook_payload_parse_context(
    payload: &HookPayload,
    invocation: &InvocationContext,
) -> Result<Option<WitParseContext>, String> {
    let context = match payload {
        HookPayload::Expression(payload) => Some(payload.context.clone()),
        HookPayload::RegisteredExpression(payload) => Some(payload.context.clone()),
        HookPayload::Condition(payload) => Some(payload.context.clone()),
        HookPayload::Effect(payload) => Some(payload.context.clone()),
        HookPayload::Section(payload) => Some(payload.context.clone()),
        HookPayload::Structure(payload) => Some(payload.context.clone()),
        HookPayload::Parser(request) => Some(parse_context_to_wit(&parse_request_context(
            request, invocation,
        )?)),
        _ => None,
    };
    Ok(context)
}

fn inherit_parse_request_context(
    mut request: ParseRequest,
    parent: Option<&WitParseContext>,
) -> ParseRequest {
    let Some(parent) = parent else {
        return request;
    };

    let has_event_override = request
        .options
        .iter()
        .any(|entry| entry.key == PARSE_CONTEXT_EVENT_CLASSES);
    if !has_event_override && !parent.event_classes.is_empty() {
        request.options.push(WitMetadataEntry {
            key: PARSE_CONTEXT_EVENT_CLASSES.to_owned(),
            value: parent.event_classes.join(";"),
            owner_component_id: None,
        });
    }
    for value in &parent.values {
        let key = format!("{PARSE_CONTEXT_VALUE_PREFIX}{}", value.key);
        if request.options.iter().any(|entry| entry.key == key) {
            continue;
        }
        request.options.push(WitMetadataEntry {
            key,
            value: value.value.clone(),
            owner_component_id: None,
        });
    }
    request
}

fn parse_request_root_mode(request: &ParseRequest) -> Result<ExpressionRootMode, String> {
    let Some(mode) = request
        .options
        .iter()
        .find(|option| option.key == PARSE_MODE)
        .map(|option| option.value.as_str())
    else {
        return Ok(ExpressionRootMode::All);
    };
    match mode {
        "all" => Ok(ExpressionRootMode::All),
        "expressions-only" => Ok(ExpressionRootMode::ExpressionsOnly),
        "literals-only" => Ok(ExpressionRootMode::LiteralsOnly),
        _ => Err(format!(
            "parse request declares unknown parse mode `{mode}`"
        )),
    }
}

fn parse_request_context(
    request: &ParseRequest,
    invocation: &InvocationContext,
) -> Result<ExpressionParseContext, String> {
    let mut context = ExpressionParseContext {
        syntax_context: invocation.syntax_context,
        ..Default::default()
    };
    for option in &request.options {
        if option.key == PARSE_CONTEXT_EVENT_CLASSES {
            context.event_classes = option
                .value
                .split(';')
                .map(str::trim)
                .filter(|class| !class.is_empty())
                .map(|class| ClassName(class.to_owned()))
                .collect();
            if context.event_classes.is_empty() {
                return Err("parse request context declares no Event classes".to_owned());
            }
            continue;
        }
        let Some(key) = option.key.strip_prefix(PARSE_CONTEXT_VALUE_PREFIX) else {
            continue;
        };
        if key.is_empty() {
            return Err("parse request context value key is empty".to_owned());
        }
        context.values.insert(key.to_owned(), option.value.clone());
    }
    Ok(context)
}

fn failed_parse_result(request: &ParseRequest, code: &str, message: &str) -> ParseResult {
    ParseResult {
        host_token: 0,
        request_id: request.request_id,
        parser_id: request.parser_id.clone(),
        status: WitParseResultStatus::Failed,
        roots: Vec::new(),
        nodes: Vec::new(),
        diagnostics: vec![Diagnostic {
            code: code.to_owned(),
            message: message.to_owned(),
            severity: DiagnosticSeverity::Error,
            span: request.span.clone(),
            related: Vec::new(),
        }],
    }
}

fn expression_parse_result(
    request: &ParseRequest,
    root: &ExpressionNode,
    catalog: Option<&Catalog>,
) -> ParseResult {
    let mut arena = ParseResultArena::new(request, catalog);
    let root_id = arena.push_expression(root);
    arena.nodes[root_id as usize].expected_types = request.expected_types.clone();
    ParseResult {
        host_token: 0,
        request_id: request.request_id,
        parser_id: request.parser_id.clone(),
        status: WitParseResultStatus::Success,
        roots: vec![root_id],
        nodes: arena.nodes,
        diagnostics: Vec::new(),
    }
}

fn rebase_expression_node(
    mut node: ExpressionNode,
    request: &ParseRequest,
) -> Result<ExpressionNode, HostError> {
    node.try_map_spans(&mut |span| rebase_match_span(span, request))?;
    Ok(node)
}

fn rebase_match_span(span: &MatchSpan, request: &ParseRequest) -> Result<MatchSpan, HostError> {
    let span = nested_span_to_request(span, request);
    let invalid = |message| HostError::InvalidParseResult {
        parser_id: request.parser_id.clone(),
        message,
    };
    let start = usize::try_from(span.virtual_range.start)
        .map_err(|_| invalid("rebased span start does not fit usize".to_owned()))?;
    let end = usize::try_from(span.virtual_range.end)
        .map_err(|_| invalid("rebased span end does not fit usize".to_owned()))?;
    let origins = span
        .origins
        .into_iter()
        .map(|origin| {
            let original_start = usize::try_from(origin.original_range.start)
                .map_err(|_| invalid("origin start does not fit usize".to_owned()))?;
            let original_end = usize::try_from(origin.original_range.end)
                .map_err(|_| invalid("origin end does not fit usize".to_owned()))?;
            let expansion = origin
                .expansion
                .map(|value| {
                    u32::try_from(value)
                        .map(ExpansionId::new)
                        .map_err(|_| invalid("expansion ID does not fit u32".to_owned()))
                })
                .transpose()?;
            Ok(skript_parser::SourceOrigin {
                original_range: ParserTextRange::new(original_start, original_end),
                kind: match origin.kind {
                    WitOriginKind::Exact => ParserOriginKind::Exact,
                    WitOriginKind::Replaced => ParserOriginKind::Replaced,
                    WitOriginKind::Anchored => ParserOriginKind::Anchored,
                },
                expansion,
            })
        })
        .collect::<Result<Vec<_>, HostError>>()?;
    let range = ParserTextRange::new(start, end);
    Ok(MatchSpan {
        local_range: range,
        mapped: skript_parser::MappedSpan {
            virtual_range: range,
            origins,
        },
    })
}

fn condition_parse_result(
    request: &ParseRequest,
    root: &ConditionNode,
    catalog: Option<&Catalog>,
) -> ParseResult {
    let mut arena = ParseResultArena::new(request, catalog);
    let root_id = arena.push_condition(root);
    ParseResult {
        host_token: 0,
        request_id: request.request_id,
        parser_id: request.parser_id.clone(),
        status: WitParseResultStatus::Success,
        roots: vec![root_id],
        nodes: arena.nodes,
        diagnostics: Vec::new(),
    }
}

struct ParseResultArena<'a> {
    request: &'a ParseRequest,
    catalog: Option<&'a Catalog>,
    nodes: Vec<ParseResultNode>,
}

impl<'a> ParseResultArena<'a> {
    fn new(request: &'a ParseRequest, catalog: Option<&'a Catalog>) -> Self {
        Self {
            request,
            catalog,
            nodes: Vec::new(),
        }
    }

    fn push_expression(&mut self, node: &ExpressionNode) -> u64 {
        let children = node
            .parsed_captures()
            .iter()
            .filter_map(|capture| self.push_capture(capture))
            .collect();
        let (kind, definition_id, registration_id, pattern_index) = match &node.kind {
            ExpressionNodeKind::Grouped => ("grouped", None, None, None),
            ExpressionNodeKind::List { .. } => ("list", None, None, None),
            ExpressionNodeKind::Registered {
                definition_id,
                registration_id,
                pattern_index,
            } => (
                "registered-expression",
                Some(definition_id.clone()),
                Some(registration_id.clone()),
                Some(*pattern_index as u64),
            ),
            ExpressionNodeKind::Variable { .. } => ("variable", None, None, None),
            ExpressionNodeKind::Literal { .. } => ("literal", None, None, None),
            ExpressionNodeKind::Function { .. } => ("function", None, None, None),
            ExpressionNodeKind::Arithmetic { .. } => ("arithmetic", None, None, None),
            ExpressionNodeKind::Custom { .. } => ("custom", None, None, None),
        };
        self.push_node(ParseResultNode {
            node_id: 0,
            parser_id: skript_parser::HOST_EXPRESSION_PARSER_ID.to_owned(),
            kind: kind.to_owned(),
            status: WitParseResultStatus::Success,
            text: node
                .span
                .local_range
                .slice(&self.request.input)
                .unwrap_or_default()
                .to_owned(),
            span: nested_span_to_request(&node.span, self.request),
            expected_types: Vec::new(),
            summary: Some(WitParseSummary {
                kind: "expression".to_owned(),
                definition_id,
                registration_id,
                element_class: expression_node_element_class(node, self.catalog),
                pattern_index,
                return_type: node
                    .return_type
                    .as_ref()
                    .map(|value| value.as_str().to_owned()),
                possible_return_types: node
                    .possible_return_types
                    .iter()
                    .map(|value| value.as_str().to_owned())
                    .collect(),
                possible_return_types_state: match node.possible_return_types_state {
                    PossibleReturnTypesState::Complete => WitPossibleReturnTypesState::Complete,
                    PossibleReturnTypesState::Partial => WitPossibleReturnTypesState::Partial,
                    PossibleReturnTypesState::Unresolved => WitPossibleReturnTypesState::Unresolved,
                },
                multiplicity: node.multiplicity.map(multiplicity_to_wit),
                public_data: public_data::to_wit(&node.public_data),
                metadata: metadata_to_wit(&node.metadata),
            }),
            children,
            attachments: Vec::new(),
            diagnostics: Vec::new(),
            metadata: metadata_to_wit(&node.metadata),
        })
    }

    fn push_condition(&mut self, node: &ConditionNode) -> u64 {
        let mut children = node
            .expressions
            .iter()
            .map(|child| self.push_expression(child))
            .collect::<Vec<_>>();
        children.extend(node.children.iter().map(|child| self.push_condition(child)));
        let (kind, definition_id, registration_id, pattern_index) = match &node.kind {
            ConditionNodeKind::Grouped => ("grouped-condition", None, None, None),
            ConditionNodeKind::Registered {
                definition_id,
                registration_id,
                pattern_index,
                ..
            } => (
                "registered-condition",
                Some(definition_id.clone()),
                Some(registration_id.clone()),
                Some(*pattern_index as u64),
            ),
        };
        self.push_node(ParseResultNode {
            node_id: 0,
            parser_id: skript_parser::HOST_CONDITION_PARSER_ID.to_owned(),
            kind: kind.to_owned(),
            status: WitParseResultStatus::Success,
            text: node
                .span
                .local_range
                .slice(&self.request.input)
                .unwrap_or_default()
                .to_owned(),
            span: nested_span_to_request(&node.span, self.request),
            expected_types: Vec::new(),
            summary: Some(WitParseSummary {
                kind: "condition".to_owned(),
                definition_id,
                registration_id,
                element_class: None,
                pattern_index,
                return_type: None,
                possible_return_types: Vec::new(),
                possible_return_types_state: WitPossibleReturnTypesState::Complete,
                multiplicity: None,
                public_data: Vec::new(),
                metadata: metadata_to_wit(&node.metadata),
            }),
            children,
            attachments: Vec::new(),
            diagnostics: Vec::new(),
            metadata: metadata_to_wit(&node.metadata),
        })
    }

    fn push_capture(&mut self, capture: &ParserParsedCapture) -> Option<u64> {
        let value = capture.result.value.as_ref()?;
        let id = match value {
            skript_parser::ParsedCaptureValue::Expression(node) => self.push_expression(node),
            skript_parser::ParsedCaptureValue::Condition(node) => self.push_condition(node),
            skript_parser::ParsedCaptureValue::Effect(_) => {
                self.push_opaque_capture(capture, "effect", &capture.result.span)
            }
            skript_parser::ParsedCaptureValue::Event(_) => {
                self.push_opaque_capture(capture, "event", &capture.result.span)
            }
            skript_parser::ParsedCaptureValue::Section(_) => {
                self.push_opaque_capture(capture, "section", &capture.result.span)
            }
            skript_parser::ParsedCaptureValue::Raw(_) => {
                self.push_opaque_capture(capture, "raw", &capture.result.span)
            }
        };
        let target = &mut self.nodes[id as usize];
        target.parser_id.clone_from(&capture.result.parser_id);
        target.attachments = capture
            .result
            .attachments
            .iter()
            .map(|attachment| WitAddonAttachment {
                owner_component_id: attachment.owner_component_id.clone(),
                schema_id: attachment.schema_id.clone(),
                schema_version: attachment.schema_version,
                encoding: attachment.encoding.clone(),
                bytes: attachment.bytes.clone(),
            })
            .collect();
        Some(id)
    }

    fn push_opaque_capture(
        &mut self,
        capture: &ParserParsedCapture,
        kind: &str,
        span: &MatchSpan,
    ) -> u64 {
        let summary = capture
            .result
            .summary
            .as_ref()
            .map(|summary| WitParseSummary {
                kind: summary.kind.clone(),
                definition_id: summary.definition_id.clone(),
                registration_id: summary.registration_id.clone(),
                element_class: summary
                    .element_class
                    .as_ref()
                    .map(|value| value.as_str().to_owned()),
                pattern_index: summary.pattern_index.map(|value| value as u64),
                return_type: summary
                    .return_type
                    .as_ref()
                    .map(|value| value.as_str().to_owned()),
                possible_return_types: summary
                    .possible_return_types
                    .iter()
                    .map(|value| value.as_str().to_owned())
                    .collect(),
                possible_return_types_state: match summary.possible_return_types_state {
                    PossibleReturnTypesState::Complete => WitPossibleReturnTypesState::Complete,
                    PossibleReturnTypesState::Partial => WitPossibleReturnTypesState::Partial,
                    PossibleReturnTypesState::Unresolved => WitPossibleReturnTypesState::Unresolved,
                },
                multiplicity: summary.multiplicity.map(multiplicity_to_wit),
                public_data: public_data::to_wit(&summary.public_data),
                metadata: metadata_to_wit(&summary.metadata),
            });
        self.push_node(ParseResultNode {
            node_id: 0,
            parser_id: capture.result.parser_id.clone(),
            kind: kind.to_owned(),
            status: match capture.result.status {
                ParserParsedCaptureStatus::Success => WitParseResultStatus::Success,
                ParserParsedCaptureStatus::Partial => WitParseResultStatus::Partial,
                ParserParsedCaptureStatus::Failed => WitParseResultStatus::Failed,
            },
            text: span
                .local_range
                .slice(&self.request.input)
                .unwrap_or_default()
                .to_owned(),
            span: nested_span_to_request(span, self.request),
            expected_types: Vec::new(),
            summary,
            children: Vec::new(),
            attachments: Vec::new(),
            diagnostics: Vec::new(),
            metadata: Vec::new(),
        })
    }

    fn push_node(&mut self, mut node: ParseResultNode) -> u64 {
        let id = self.nodes.len() as u64;
        node.node_id = id;
        self.nodes.push(node);
        id
    }
}

fn metadata_to_wit(metadata: &BTreeMap<String, String>) -> Vec<WitMetadataEntry> {
    metadata
        .iter()
        .map(|(key, value)| {
            let (owner_component_id, key) = key
                .split_once('/')
                .filter(|(owner, key)| !owner.is_empty() && !key.is_empty())
                .map_or_else(
                    || (None, key.as_str()),
                    |(owner, key)| (Some(owner.to_owned()), key),
                );
            WitMetadataEntry {
                owner_component_id,
                key: key.to_owned(),
                value: value.clone(),
            }
        })
        .collect()
}

fn nested_span_to_request(span: &MatchSpan, request: &ParseRequest) -> MappedSpan {
    // Nested matcher frames keep `local_range` relative to their own pattern
    // input. The mapped range remains absolute within the parse-request input.
    let start = span.mapped.virtual_range.start as u64;
    let end = span.mapped.virtual_range.end as u64;
    let virtual_start = request.span.virtual_range.start.saturating_add(start);
    let virtual_end = request.span.virtual_range.start.saturating_add(end);
    let input_len = request.input.len() as u64;
    let origins = request
        .span
        .origins
        .iter()
        .map(|origin| {
            let origin_len = origin
                .original_range
                .end
                .saturating_sub(origin.original_range.start);
            let original_range = if origin_len >= input_len {
                WitTextRange {
                    start: origin.original_range.start.saturating_add(start),
                    end: origin.original_range.start.saturating_add(end),
                }
            } else {
                origin.original_range
            };
            WitSourceOrigin {
                original_range,
                kind: origin.kind,
                expansion: origin.expansion,
            }
        })
        .collect();
    MappedSpan {
        virtual_range: WitTextRange {
            start: virtual_start,
            end: virtual_end,
        },
        origins,
    }
}

fn validate_parse_result(request: &ParseRequest, result: &ParseResult) -> Result<(), HostError> {
    let invalid = |message: String| HostError::InvalidParseResult {
        parser_id: request.parser_id.clone(),
        message,
    };
    if result.request_id != request.request_id || result.parser_id != request.parser_id {
        return Err(invalid("request ID or parser ID does not match".to_owned()));
    }
    if matches!(result.status, WitParseResultStatus::Success) && result.roots.is_empty() {
        return Err(invalid(
            "a successful result must have at least one root".to_owned(),
        ));
    }
    let mut nodes = HashMap::with_capacity(result.nodes.len());
    for (index, node) in result.nodes.iter().enumerate() {
        if let Some(summary) = &node.summary {
            public_data::validate(&summary.public_data).map_err(&invalid)?;
        }
        if nodes.insert(node.node_id, index).is_some() {
            return Err(invalid(format!("duplicate node ID {}", node.node_id)));
        }
        if node.span.virtual_range.start > node.span.virtual_range.end
            || node.span.virtual_range.start < request.span.virtual_range.start
            || node.span.virtual_range.end > request.span.virtual_range.end
        {
            return Err(invalid(format!(
                "node {} has a span outside the request",
                node.node_id
            )));
        }
    }
    for root in &result.roots {
        if !nodes.contains_key(root) {
            return Err(invalid(format!("unknown root node ID {root}")));
        }
    }
    for node in &result.nodes {
        for child in &node.children {
            if !nodes.contains_key(child) {
                return Err(invalid(format!(
                    "node {} references unknown child {child}",
                    node.node_id
                )));
            }
        }
    }
    let mut states = HashMap::<u64, u8>::new();
    for node in &result.nodes {
        let mut stack = vec![(node.node_id, false)];
        while let Some((id, leaving)) = stack.pop() {
            if leaving {
                states.insert(id, 2);
                continue;
            }
            match states.get(&id).copied().unwrap_or(0) {
                2 => continue,
                1 => {
                    return Err(invalid(format!(
                        "parse result graph contains a cycle at {id}"
                    )));
                }
                _ => {}
            }
            states.insert(id, 1);
            stack.push((id, true));
            let node = &result.nodes[nodes[&id]];
            stack.extend(node.children.iter().rev().map(|child| (*child, false)));
        }
    }
    Ok(())
}

fn empty_effects() -> HookEffects {
    HookEffects {
        diagnostics: Vec::new(),
        context_updates: Vec::new(),
        parse_requests: Vec::new(),
        parse_results: Vec::new(),
    }
}

fn retain_diagnostics_only(effects: &mut HookEffects, calls: &mut Vec<HookCall>) {
    effects.context_updates.clear();
    effects.parse_requests.clear();
    effects.parse_results.clear();
    calls.clear();
}

fn merge_effects(target: &mut HookEffects, source: HookEffects) {
    target.diagnostics.extend(source.diagnostics);
    target.context_updates.extend(source.context_updates);
    target.parse_requests.extend(source.parse_requests);
    target.parse_results.extend(source.parse_results);
}

fn stamp_parse_result_attachments(effects: &mut HookEffects, component_id: &str) {
    for attachment in effects
        .parse_results
        .iter_mut()
        .flat_map(|result| &mut result.nodes)
        .flat_map(|node| &mut node.attachments)
    {
        attachment.owner_component_id = component_id.to_owned();
    }
}

fn hook_output_size(output: &HookOutput) -> usize {
    output
        .replacement
        .as_ref()
        .map_or(0, hook_payload_size)
        .saturating_add(hook_effects_size(&output.effects))
        .saturating_add(match &output.decision {
            HookDecision::Reject(rejection) => rejection_size(rejection),
            HookDecision::NotApplicable
            | HookDecision::ContinueProcessing
            | HookDecision::Handled => 0,
        })
}

fn text_macro_output_size(output: &TextMacroOutput) -> usize {
    output
        .edits
        .iter()
        .map(|edit| edit.replacement.len())
        .fold(0usize, usize::saturating_add)
        .saturating_add(hook_effects_size(&output.effects))
        .saturating_add(match &output.decision {
            HookDecision::Reject(rejection) => rejection_size(rejection),
            HookDecision::NotApplicable
            | HookDecision::ContinueProcessing
            | HookDecision::Handled => 0,
        })
}

fn hook_payload_size(payload: &HookPayload) -> usize {
    match payload {
        HookPayload::Document(value) => value.text.len(),
        HookPayload::Preprocess(value) => value.text.len(),
        HookPayload::Line(value) => value.text.len(),
        HookPayload::Tree(value) => raw_tree_size(value),
        HookPayload::Node(value) => raw_node_size(&value.node),
        HookPayload::Matching(value) => value
            .input
            .len()
            .saturating_add(value.pattern.as_ref().map_or(0, String::len))
            .saturating_add(metadata_entries_size(&value.metadata)),
        HookPayload::Condition(value) => value
            .input
            .len()
            .saturating_add(parse_context_size(&value.context))
            .saturating_add(value.candidate.definition_id.len())
            .saturating_add(value.candidate.registration_id.len())
            .saturating_add(value.candidate.pattern.len())
            .saturating_add(
                value
                    .candidate
                    .children
                    .iter()
                    .map(|child| {
                        child
                            .text
                            .len()
                            .saturating_add(public_data::size(&child.public_data))
                    })
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(metadata_entries_size(&value.candidate.metadata)),
        HookPayload::Effect(value) => value
            .input
            .len()
            .saturating_add(parse_context_size(&value.context))
            .saturating_add(value.candidate.as_ref().map_or(0, effect_candidate_size))
            .saturating_add(
                value
                    .alternatives
                    .iter()
                    .map(effect_candidate_size)
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(value.failure.as_ref().map_or(0, |failure| {
                failure
                    .reasons
                    .iter()
                    .map(String::len)
                    .fold(0usize, usize::saturating_add)
            })),
        HookPayload::Section(value) => value
            .input
            .len()
            .saturating_add(parse_context_size(&value.context))
            .saturating_add(value.candidate.definition_id.len())
            .saturating_add(value.candidate.registration_id.len())
            .saturating_add(
                value
                    .candidate
                    .element_class
                    .as_ref()
                    .map_or(0, String::len),
            )
            .saturating_add(
                value
                    .candidate
                    .regex_captures
                    .iter()
                    .map(String::len)
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(
                value
                    .candidate
                    .parsed_captures
                    .iter()
                    .map(parsed_capture_size)
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(metadata_entries_size(&value.candidate.metadata)),
        HookPayload::Structure(value) => value
            .input
            .len()
            .saturating_add(parse_context_size(&value.context))
            .saturating_add(raw_tree_size(&value.body_tree))
            .saturating_add(value.candidate.definition_id.len())
            .saturating_add(value.candidate.registration_id.len())
            .saturating_add(value.candidate.pattern.len())
            .saturating_add(
                value
                    .candidate
                    .regex_captures
                    .iter()
                    .map(String::len)
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(
                value
                    .candidate
                    .parsed_captures
                    .iter()
                    .map(parsed_capture_size)
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(
                value
                    .candidate
                    .entries
                    .iter()
                    .map(|entry| {
                        entry
                            .key
                            .len()
                            .saturating_add(entry.entry_data_class.len())
                            .saturating_add(entry.source.len())
                            .saturating_add(
                                entry
                                    .value_summary
                                    .as_ref()
                                    .map_or(0, |summary| public_data::size(&summary.public_data)),
                            )
                    })
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(metadata_entries_size(&value.candidate.metadata)),
        HookPayload::Expression(value) => value
            .input
            .len()
            .saturating_add(
                value
                    .type_options
                    .iter()
                    .map(expression_type_option_size)
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(
                value
                    .literal_options
                    .iter()
                    .map(expression_literal_option_size)
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(
                value
                    .candidates
                    .iter()
                    .map(|candidate| {
                        candidate
                            .parser_id
                            .len()
                            .saturating_add(candidate.return_type.as_ref().map_or(0, String::len))
                            .saturating_add(public_data::size(&candidate.public_data))
                            .saturating_add(metadata_entries_size(&candidate.metadata))
                    })
                    .fold(0usize, usize::saturating_add),
            ),
        HookPayload::RegisteredExpression(value) => value
            .input
            .len()
            .saturating_add(public_data::size(&value.public_data))
            .saturating_add(parse_context_size(&value.context))
            .saturating_add(value.definition_id.len())
            .saturating_add(value.registration_id.len())
            .saturating_add(value.element_class.len())
            .saturating_add(value.pattern.len())
            .saturating_add(value.related_property.as_ref().map_or(0, String::len))
            .saturating_add(
                value
                    .expected_types
                    .iter()
                    .map(|expected| expected.class_name.len())
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(value.declared_return_type.as_ref().map_or(0, String::len))
            .saturating_add(
                value
                    .possible_return_types
                    .iter()
                    .map(String::len)
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(
                value
                    .regex_captures
                    .iter()
                    .map(String::len)
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(
                value
                    .tags
                    .iter()
                    .map(|tag| tag.value.len())
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(
                value
                    .children
                    .iter()
                    .map(|child| {
                        child
                            .text
                            .len()
                            .saturating_add(child.kind.len())
                            .saturating_add(child.parser_id.as_ref().map_or(0, String::len))
                            .saturating_add(child.definition_id.as_ref().map_or(0, String::len))
                            .saturating_add(child.registration_id.as_ref().map_or(0, String::len))
                            .saturating_add(child.element_class.as_ref().map_or(0, String::len))
                            .saturating_add(child.return_type.as_ref().map_or(0, String::len))
                            .saturating_add(public_data::size(&child.public_data))
                            .saturating_add(metadata_entries_size(&child.metadata))
                    })
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(
                value
                    .parsed_captures
                    .iter()
                    .map(parsed_capture_size)
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(
                value
                    .common_child_return_type
                    .as_ref()
                    .map_or(0, String::len),
            )
            .saturating_add(
                value
                    .type_options
                    .iter()
                    .map(expression_type_option_size)
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(
                value
                    .property_options
                    .iter()
                    .map(|option| {
                        option
                            .match_kind
                            .len()
                            .saturating_add(
                                option
                                    .source_record
                                    .as_ref()
                                    .map_or(0, catalog_record_ref_size),
                            )
                            .saturating_add(mem::size_of::<u64>() * 3)
                            .saturating_add(option.property_registration_id.len())
                            .saturating_add(option.property_name.len())
                            .saturating_add(option.property_handler_class.len())
                            .saturating_add(option.property_addon_name.len())
                            .saturating_add(option.property_addon_version.len())
                            .saturating_add(option.input_class.len())
                            .saturating_add(option.handler_class.len())
                            .saturating_add(option.handler_kind.len())
                            .saturating_add(
                                option.provider_addon_name.as_ref().map_or(0, String::len),
                            )
                            .saturating_add(
                                option
                                    .provider_addon_version
                                    .as_ref()
                                    .map_or(0, String::len),
                            )
                            .saturating_add(option.type_code_name.len())
                            .saturating_add(
                                option
                                    .element_types
                                    .iter()
                                    .map(String::len)
                                    .fold(0usize, usize::saturating_add),
                            )
                            .saturating_add(
                                option
                                    .return_types
                                    .iter()
                                    .map(String::len)
                                    .fold(0usize, usize::saturating_add),
                            )
                            .saturating_add(
                                option
                                    .supported_axes
                                    .iter()
                                    .map(String::len)
                                    .fold(0usize, usize::saturating_add),
                            )
                            .saturating_add(
                                option
                                    .accepted_changers
                                    .iter()
                                    .map(|change| {
                                        change.mode.len().saturating_add(
                                            change
                                                .accepted_types
                                                .iter()
                                                .map(String::len)
                                                .fold(0usize, usize::saturating_add),
                                        )
                                    })
                                    .fold(0usize, usize::saturating_add),
                            )
                    })
                    .fold(0usize, usize::saturating_add),
            )
            .saturating_add(
                value
                    .selected_property_option_indices
                    .len()
                    .saturating_mul(mem::size_of::<u64>()),
            )
            .saturating_add(value.effective_return_type.as_ref().map_or(0, String::len))
            .saturating_add(metadata_entries_size(&value.metadata)),
        HookPayload::Capture(value) => value
            .syntax_id
            .len()
            .saturating_add(captures_size(&value.captures)),
        HookPayload::Syntax(value) => value.syntax_id.len().saturating_add(
            value
                .patterns
                .iter()
                .map(String::len)
                .fold(0usize, usize::saturating_add),
        ),
        HookPayload::ExactRegistration(value) => value
            .registration_id
            .len()
            .saturating_add(value.definition.syntax_id.len())
            .saturating_add(
                value
                    .definition
                    .patterns
                    .iter()
                    .map(String::len)
                    .fold(0usize, usize::saturating_add),
            ),
        HookPayload::Candidate(value) => value
            .syntax_id
            .len()
            .saturating_add(value.registration_id.len()),
        HookPayload::Scope(_) => 0,
        HookPayload::Ast(value) => ast_tree_size(value),
        HookPayload::Diagnostic(value) => diagnostic_size(value),
        HookPayload::Parser(value) => parse_request_size(value),
    }
}

fn catalog_record_ref_size(record: &WitCatalogRecordRef) -> usize {
    record
        .source_digest
        .len()
        .saturating_add(record.snapshot_id.len())
        .saturating_add(record.document.len())
        .saturating_add(mem::size_of::<u64>() * 2)
}

fn expression_type_option_size(option: &WitExpressionTypeOption) -> usize {
    option
        .source_record
        .as_ref()
        .map_or(0, catalog_record_ref_size)
        .saturating_add(option.code_name.len())
        .saturating_add(option.class_name.len())
        .saturating_add(option.singular.len())
        .saturating_add(option.plural.len())
        .saturating_add(
            option
                .user_input_patterns
                .iter()
                .map(String::len)
                .fold(0usize, usize::saturating_add),
        )
        .saturating_add(
            option
                .parse_contexts
                .iter()
                .map(String::len)
                .fold(0usize, usize::saturating_add),
        )
}

fn expression_literal_option_size(option: &WitExpressionLiteralOption) -> usize {
    option
        .source_record
        .as_ref()
        .map_or(0, catalog_record_ref_size)
        .saturating_add(option.code_name.len())
        .saturating_add(option.class_name.len())
        .saturating_add(option.canonical_value.len())
        .saturating_add(option.addon_name.len())
        .saturating_add(option.addon_version.len())
        .saturating_add(option.parser_class.as_ref().map_or(0, String::len))
        .saturating_add(
            option
                .parse_contexts
                .iter()
                .map(String::len)
                .fold(0usize, usize::saturating_add),
        )
        .saturating_add(option.value_class.as_ref().map_or(0, String::len))
        .saturating_add(option.represented_class.as_ref().map_or(0, String::len))
        .saturating_add(option.variable_name.as_ref().map_or(0, String::len))
        .saturating_add(option.debug_text.as_ref().map_or(0, String::len))
        .saturating_add(option.enum_constant.as_ref().map_or(0, String::len))
}

fn effect_candidate_size(
    candidate: &crate::bindings::nlaocs::skript_parser_addon::types::EffectCandidate,
) -> usize {
    candidate
        .definition_id
        .len()
        .saturating_add(candidate.registration_id.len())
        .saturating_add(candidate.element_class.as_ref().map_or(0, String::len))
        .saturating_add(candidate.pattern.len())
        .saturating_add(candidate.handler.as_ref().map_or(0, String::len))
        .saturating_add(
            candidate
                .captures
                .iter()
                .map(|capture| match capture {
                    WitEffectCapture::Regex(capture) => capture.value.len(),
                    WitEffectCapture::Expression(capture) => capture
                        .expression
                        .len()
                        .saturating_add(capture.value.len())
                        .saturating_add(capture.resolution_id.as_ref().map_or(0, String::len)),
                })
                .fold(0usize, usize::saturating_add),
        )
        .saturating_add(
            candidate
                .tags
                .iter()
                .map(|tag| tag.value.len())
                .fold(0usize, usize::saturating_add),
        )
        .saturating_add(metadata_entries_size(&candidate.metadata))
        .saturating_add(
            candidate
                .parsed_captures
                .iter()
                .map(parsed_capture_size)
                .fold(0usize, usize::saturating_add),
        )
}

fn parse_context_size(context: &WitParseContext) -> usize {
    context
        .event_classes
        .iter()
        .map(String::len)
        .chain(
            context
                .values
                .iter()
                .map(|entry| entry.key.len().saturating_add(entry.value.len())),
        )
        .fold(0usize, usize::saturating_add)
}

fn metadata_entries_size(entries: &[WitMetadataEntry]) -> usize {
    entries
        .iter()
        .map(|entry| {
            entry
                .owner_component_id
                .as_ref()
                .map_or(0, String::len)
                .saturating_add(entry.key.len())
                .saturating_add(entry.value.len())
        })
        .fold(0usize, usize::saturating_add)
}

fn parsed_capture_size(capture: &WitParsedCapture) -> usize {
    capture
        .text
        .len()
        .saturating_add(capture.parser_id.len())
        .saturating_add(
            capture
                .expected_types
                .iter()
                .map(|expected| expected.class_name.len())
                .fold(0usize, usize::saturating_add),
        )
        .saturating_add(capture.summary.as_ref().map_or(0, |summary| {
            summary
                .kind
                .len()
                .saturating_add(summary.definition_id.as_ref().map_or(0, String::len))
                .saturating_add(summary.registration_id.as_ref().map_or(0, String::len))
                .saturating_add(summary.element_class.as_ref().map_or(0, String::len))
                .saturating_add(summary.return_type.as_ref().map_or(0, String::len))
                .saturating_add(public_data::size(&summary.public_data))
                .saturating_add(metadata_entries_size(&summary.metadata))
        }))
        .saturating_add(
            capture
                .attachments
                .iter()
                .map(|attachment| {
                    attachment
                        .owner_component_id
                        .len()
                        .saturating_add(attachment.schema_id.len())
                        .saturating_add(attachment.encoding.len())
                        .saturating_add(attachment.bytes.len())
                })
                .fold(0usize, usize::saturating_add),
        )
        .saturating_add(
            capture
                .diagnostics
                .iter()
                .map(diagnostic_size)
                .fold(0usize, usize::saturating_add),
        )
}
fn raw_tree_size(tree: &RawTree) -> usize {
    tree.nodes
        .iter()
        .map(raw_node_size)
        .fold(0usize, usize::saturating_add)
        .saturating_add(tree.roots.len().saturating_mul(mem::size_of::<u64>()))
}

fn raw_node_size(node: &RawTreeNode) -> usize {
    node.text
        .len()
        .saturating_add(node.children.len().saturating_mul(mem::size_of::<u64>()))
}

fn ast_tree_size(tree: &AstTree) -> usize {
    tree.nodes
        .iter()
        .map(ast_node_size)
        .fold(0usize, usize::saturating_add)
        .saturating_add(tree.roots.len().saturating_mul(mem::size_of::<u64>()))
}

fn ast_node_size(node: &AstNode) -> usize {
    node.syntax_id
        .len()
        .saturating_add(captures_size(&node.captures))
        .saturating_add(node.children.len().saturating_mul(mem::size_of::<u64>()))
}

fn captures_size(captures: &[Capture]) -> usize {
    captures
        .iter()
        .map(|capture| {
            capture.name.len().saturating_add(match &capture.value {
                CaptureValue::Text(value) => value.len(),
                CaptureValue::Nodes(values) => values.len().saturating_mul(mem::size_of::<u64>()),
                CaptureValue::Node(_) | CaptureValue::Span(_) => 0,
            })
        })
        .fold(0usize, usize::saturating_add)
}

fn hook_effects_size(effects: &HookEffects) -> usize {
    effects
        .diagnostics
        .iter()
        .map(diagnostic_size)
        .fold(0usize, usize::saturating_add)
        .saturating_add(
            effects
                .context_updates
                .iter()
                .map(context_update_size)
                .fold(0usize, usize::saturating_add),
        )
        .saturating_add(
            effects
                .parse_requests
                .iter()
                .map(parse_request_size)
                .fold(0usize, usize::saturating_add),
        )
        .saturating_add(
            effects
                .parse_results
                .iter()
                .map(parse_result_size)
                .fold(0usize, usize::saturating_add),
        )
}

fn diagnostic_size(diagnostic: &Diagnostic) -> usize {
    diagnostic
        .code
        .len()
        .saturating_add(diagnostic.message.len())
        .saturating_add(
            diagnostic
                .related
                .iter()
                .map(|related| related.message.len())
                .fold(0usize, usize::saturating_add),
        )
}

fn rejection_size(rejection: &Rejection) -> usize {
    rejection.reason.len().saturating_add(
        rejection
            .diagnostics
            .iter()
            .map(diagnostic_size)
            .fold(0usize, usize::saturating_add),
    )
}

fn context_update_size(update: &ContextUpdate) -> usize {
    update
        .key
        .len()
        .saturating_add(update.value.as_ref().map_or(0, Vec::len))
}

fn parse_request_size(request: &ParseRequest) -> usize {
    request
        .parser_id
        .len()
        .saturating_add(request.input.len())
        .saturating_add(
            request
                .expected_types
                .iter()
                .map(|expected| expected.class_name.len())
                .fold(0usize, usize::saturating_add),
        )
        .saturating_add(metadata_entries_size(&request.options))
}

fn parse_result_size(result: &ParseResult) -> usize {
    result
        .parser_id
        .len()
        .saturating_add(
            result
                .nodes
                .iter()
                .map(parse_result_node_size)
                .fold(0usize, usize::saturating_add),
        )
        .saturating_add(
            result
                .diagnostics
                .iter()
                .map(diagnostic_size)
                .fold(0usize, usize::saturating_add),
        )
}

fn parse_result_node_size(node: &ParseResultNode) -> usize {
    node.parser_id
        .len()
        .saturating_add(
            node.summary
                .as_ref()
                .map_or(0, |summary| public_data::size(&summary.public_data)),
        )
        .saturating_add(node.kind.len())
        .saturating_add(node.text.len())
        .saturating_add(metadata_entries_size(&node.metadata))
        .saturating_add(
            node.attachments
                .iter()
                .map(|attachment| {
                    attachment
                        .owner_component_id
                        .len()
                        .saturating_add(attachment.schema_id.len())
                        .saturating_add(attachment.encoding.len())
                        .saturating_add(attachment.bytes.len())
                })
                .fold(0usize, usize::saturating_add),
        )
        .saturating_add(
            node.diagnostics
                .iter()
                .map(diagnostic_size)
                .fold(0usize, usize::saturating_add),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core_test_config() -> HostConfig {
        HostConfig {
            runtime_profile: RuntimeProfile {
                skript_version: Some("2.15.4".to_owned()),
                ..RuntimeProfile::default()
            },
            ..HostConfig::default()
        }
    }

    use crate::bindings::nlaocs::skript_parser_addon::types::DocumentPayload;
    use std::path::Path;
    use wasm_encoder::{
        BlockType, CodeSection, ExportKind, ExportSection, Function, FunctionSection, Instruction,
        MemorySection, MemoryType, Module as EncodedModule, TypeSection, ValType,
    };
    use wasmtime::{Instance, Module as WasmtimeModule};

    #[test]
    fn epoch_deadline_only_bounds_untrusted_addons() {
        let config = HostConfig::default();
        assert_eq!(
            config.deadline_ticks(CORE_LIBRARY_COMPONENT_ID),
            CORE_LIBRARY_DEADLINE_TICKS
        );
        assert_eq!(config.deadline_ticks("addon.example"), 10);
    }

    #[test]
    fn hosts_with_the_same_epoch_tick_share_the_wasmtime_runtime() {
        let epoch_tick = HostConfig::default().epoch_tick;
        let first = shared_host_runtime(epoch_tick).expect("shared runtime must initialize");
        let second = shared_host_runtime(epoch_tick).expect("shared runtime must be reusable");

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn source_catalog_and_dynamic_syntax_capabilities_are_independent() {
        let normalized_only = configured_host_capabilities(true, false);
        assert!(
            normalized_only
                .iter()
                .any(|capability| capability.id == CAPABILITY_DYNAMIC_SYNTAX)
        );
        assert!(
            normalized_only
                .iter()
                .all(|capability| capability.id != CAPABILITY_CATALOG_DATA)
        );

        let source_catalog = configured_host_capabilities(true, true);
        assert!(
            source_catalog
                .iter()
                .any(|capability| capability.id == CAPABILITY_CATALOG_DATA)
        );
    }

    #[test]
    fn type_and_literal_options_reference_their_exact_ssg_record() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4");
        let catalog = ssg::load(fixture).unwrap().into_catalog();

        let type_options = all_expression_type_options(Some(&catalog));
        assert!(!type_options.is_empty());
        assert!(type_options.iter().all(|option| {
            option
                .source_record
                .as_ref()
                .is_some_and(|record| record.document == "Types.json")
        }));

        let literal_options = expression_literal_options(
            Some(&catalog),
            "zombie",
            ParserTextRange::new(0, 6),
            &[6],
            &[],
        );
        assert!(!literal_options.is_empty());
        assert!(literal_options.iter().all(|option| {
            option
                .source_record
                .as_ref()
                .is_some_and(|record| record.document == "Types.json")
        }));

        let entity_data_options = expression_literal_options(
            Some(&catalog),
            "players",
            ParserTextRange::new(0, 7),
            &[7],
            &[skript_parser::ExpressionExpectedType {
                class_name: ClassName("ch.njol.skript.entity.EntityData".to_owned()),
                plural: true,
            }],
        );
        assert!(
            entity_data_options.is_empty(),
            "schema 3 omits finite EntityData supplier values"
        );
    }

    #[test]
    fn catalog_pattern_matching_supports_type_user_input_patterns() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4");
        let config = HostConfig {
            syntax_catalog: Some(Arc::new(ssg::load(fixture).unwrap().into_catalog())),
            ..HostConfig::default()
        };
        let engine = Engine::default();
        let mut store = create_store(
            &engine,
            &config,
            build_type_user_input_matchers(config.syntax_catalog.as_deref()),
        );

        assert!(
            wit_catalog_data::Host::language_pattern_matches(
                store.data_mut(),
                "type.user-input-pattern:players?".to_owned(),
                "(?i:players?)".to_owned(),
                "players".to_owned(),
            )
            .unwrap()
        );
        assert!(
            !wit_catalog_data::Host::language_pattern_matches(
                store.data_mut(),
                "type.user-input-pattern:players?".to_owned(),
                "(?i:players?)".to_owned(),
                "all players".to_owned(),
            )
            .unwrap()
        );
    }

    #[test]
    fn core_struct_event_handler_resolves_fixture_registration() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4");
        let catalog = Arc::new(ssg::load(fixture).unwrap().into_catalog());
        let host = ParserHost::new(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../artifacts/core-library.wasm"
            )),
            HostConfig {
                syntax_catalog: Some(Arc::clone(&catalog)),
                ..HostConfig::default()
            },
        )
        .unwrap();
        let registration_id = catalog
            .structures()
            .find(|structure| {
                structure
                    .common
                    .element_class
                    .as_str()
                    .ends_with(".StructEvent")
            })
            .unwrap()
            .common
            .registration_id
            .as_str();
        let binding = host.components[0]
            .registered_handler_bindings
            .iter()
            .find(|binding| binding.handler_id == "core.structure.struct-event")
            .unwrap();

        assert!(
            binding
                .registration_ids
                .iter()
                .any(|resolved| resolved == registration_id),
            "StructEvent registration was not resolved: {binding:#?}"
        );
    }

    fn subscription(
        id: &str,
        target: HookTarget,
        priority: i32,
        mode: HookMode,
    ) -> HookSubscription {
        HookSubscription {
            id: id.to_owned(),
            target,
            phase: HookPhase::Document,
            priority,
            mode,
            capability_id: CAPABILITY_HOOKS.to_owned(),
            selector: empty_selector(),
        }
    }

    fn empty_selector() -> HookSelector {
        HookSelector {
            pattern_index: None,
            pattern_source: None,
            mark: None,
            tags: Vec::new(),
            captures: Vec::new(),
            return_type: None,
            multiplicity: None,
            metadata: Vec::new(),
        }
    }

    fn document(text: &str) -> HookPayload {
        HookPayload::Document(DocumentPayload {
            text: text.to_owned(),
        })
    }

    #[test]
    fn wit_structure_context_updates_compose_event_classes_and_values_deterministically() {
        let mut context = WitParseContext {
            syntax_context: 7,
            event_classes: vec!["old.Event".to_owned()],
            values: vec![
                WitParseContextValue {
                    key: "zeta".to_owned(),
                    value: "old-zeta".to_owned(),
                },
                WitParseContextValue {
                    key: "alpha".to_owned(),
                    value: "old-alpha".to_owned(),
                },
            ],
        };
        let updates = vec![
            ContextUpdate {
                syntax_context: 7,
                key: "parser.event-classes".to_owned(),
                value: Some(b"first.Event;second.Event".to_vec()),
            },
            ContextUpdate {
                syntax_context: 7,
                key: "zeta".to_owned(),
                value: Some(b"new-zeta".to_vec()),
            },
            ContextUpdate {
                syntax_context: 7,
                key: "alpha".to_owned(),
                value: None,
            },
            ContextUpdate {
                syntax_context: 7,
                key: "middle".to_owned(),
                value: Some(b"new-middle".to_vec()),
            },
            ContextUpdate {
                syntax_context: 999,
                key: "ignored".to_owned(),
                value: Some(b"stale".to_vec()),
            },
            ContextUpdate {
                syntax_context: 7,
                key: "parser.event-classes".to_owned(),
                value: None,
            },
            ContextUpdate {
                syntax_context: 7,
                key: "parser.event-classes".to_owned(),
                value: Some(b"final.Event;".to_vec()),
            },
            ContextUpdate {
                syntax_context: 7,
                key: "zeta".to_owned(),
                value: None,
            },
        ];

        apply_wit_structure_context_updates(&mut context, &updates)
            .expect("valid UTF-8 context updates must apply");

        assert_eq!(context.event_classes, ["final.Event"]);
        assert_eq!(context.values.len(), 1);
        assert_eq!(context.values[0].key, "middle");
        assert_eq!(context.values[0].value, "new-middle");
    }

    #[test]
    fn dynamic_structure_validator_rebuilds_nested_entries_and_json_defaults() {
        let validator = dynamic_entry_validator_value(WitStructureEntryValidator {
            entry_data: vec![
                WitStructureEntryData {
                    parent_entry_index: None,
                    key: "settings".to_owned(),
                    default_value: None,
                    optional: false,
                    multiple: false,
                    entry_data_class: "fixture.SettingsEntryData".to_owned(),
                    kind: WitStructureEntryKind::Container,
                    separator: None,
                    value_type: None,
                    string_mode: None,
                    return_types: Vec::new(),
                    flags: None,
                    nested_validator_present: true,
                },
                WitStructureEntryData {
                    parent_entry_index: Some(0),
                    key: "values".to_owned(),
                    default_value: Some(r#"{"items":[1,null]}"#.to_owned()),
                    optional: true,
                    multiple: true,
                    entry_data_class: "fixture.ValuesEntryData".to_owned(),
                    kind: WitStructureEntryKind::KeyValue,
                    separator: Some(": ".to_owned()),
                    value_type: Some("java.lang.Object".to_owned()),
                    string_mode: Some("raw".to_owned()),
                    return_types: vec!["java.lang.Object".to_owned()],
                    flags: Some(3),
                    nested_validator_present: false,
                },
            ],
        })
        .expect("flat Structure validator must be rebuilt");

        assert_eq!(validator.entry_data.len(), 1);
        let settings = &validator.entry_data[0];
        assert_eq!(settings.key, "settings");
        let nested = settings
            .nested_validator
            .as_ref()
            .expect("container must retain its nested validator");
        assert_eq!(nested.entry_data.len(), 1);
        let values = &nested.entry_data[0];
        assert_eq!(values.key, "values");
        assert_eq!(values.flags, Some(3));
        assert_eq!(
            values
                .default_value
                .as_ref()
                .expect("JSON default must be retained")
                .to_string(),
            r#"{"items":[1,null]}"#
        );
    }

    #[test]
    fn same_raw_tree_rejects_mutated_structure_body_tree() {
        let source = MappedSource::identity("root:\n    child\n");
        let tree = skript_parser::parse_raw_tree(
            &source,
            skript_parser::RawTreeOptions::for_skript_version(2, 15),
        );
        let root = *tree.roots.first().expect("fixture has a root node");
        let original = parser_raw_subtree_to_wit(&tree, root);
        let mut mutated = original.clone();
        mutated.nodes[0].text.push_str(" changed");

        // Structure hooks may update semantic context, but the body tree is an
        // immutable identity field and must not be replaced by a hook.
        assert!(!same_raw_tree(&original, &mutated));
    }

    fn registered_expression() -> HookPayload {
        HookPayload::RegisteredExpression(RegisteredExpressionPayload {
            input: "example".to_owned(),
            context: WitParseContext {
                syntax_context: 0,
                event_classes: Vec::new(),
                values: Vec::new(),
            },
            definition_id: "expression:test:definition".to_owned(),
            registration_id: "expression:test:registration".to_owned(),
            element_class: "example.ExprTest".to_owned(),
            related_property: None,
            pattern_index: 2,
            pattern: "example %objects%".to_owned(),
            span: MappedSpan {
                virtual_range: WitTextRange { start: 0, end: 7 },
                origins: Vec::new(),
            },
            expected_types: Vec::new(),
            declared_return_type: Some("java.lang.String".to_owned()),
            declared_multiplicity: Some(WitDynamicMultiplicity::Single),
            return_type_state: ExpressionReturnTypeState::Static,
            possible_return_types: vec!["java.lang.String".to_owned()],
            possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
            time: 0,
            regex_captures: Vec::new(),
            tags: vec![RegisteredExpressionTag {
                value: "test".to_owned(),
                implicit: false,
            }],
            mark: 4,
            children: Vec::new(),
            parsed_captures: Vec::new(),
            common_child_return_type: None,
            type_options: Vec::new(),
            property_options: Vec::new(),
            selected_property_option_indices: Vec::new(),
            effective_return_type: Some("java.lang.String".to_owned()),
            effective_possible_return_types: vec!["java.lang.String".to_owned()],
            effective_possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
            effective_multiplicity: Some(WitDynamicMultiplicity::Single),
            public_data: Vec::new(),
            metadata: vec![WitMetadataEntry {
                owner_component_id: None,
                key: "phase".to_owned(),
                value: "resolved".to_owned(),
            }],
        })
    }

    fn public_data_entry(json: &str) -> WitExpressionPublicData {
        WitExpressionPublicData {
            schema_id: "example.semantic".to_owned(),
            schema_version: 1,
            json: json.to_owned(),
        }
    }

    fn registered_expression_child(
        public_data: Vec<WitExpressionPublicData>,
    ) -> WitRegisteredExpressionChild {
        WitRegisteredExpressionChild {
            text: "child".to_owned(),
            kind: "expression".to_owned(),
            parser_id: Some("parser:test".to_owned()),
            definition_id: Some("expression:test:definition".to_owned()),
            registration_id: Some("expression:test:registration".to_owned()),
            pattern_index: Some(0),
            element_class: Some("example.ExprTest".to_owned()),
            return_type: Some("java.lang.String".to_owned()),
            possible_return_types: vec!["java.lang.String".to_owned()],
            possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
            multiplicity: Some(WitDynamicMultiplicity::Single),
            public_data,
            metadata: Vec::new(),
        }
    }

    fn empty_mapped_span() -> MappedSpan {
        MappedSpan {
            virtual_range: WitTextRange { start: 0, end: 0 },
            origins: Vec::new(),
        }
    }

    fn condition_with_children(children: Vec<WitRegisteredExpressionChild>) -> HookPayload {
        HookPayload::Condition(WitConditionPayload {
            input: "example".to_owned(),
            context: WitParseContext {
                syntax_context: 0,
                event_classes: Vec::new(),
                values: Vec::new(),
            },
            candidate: WitConditionCandidate {
                definition_id: "condition:test:definition".to_owned(),
                registration_id: "condition:test:registration".to_owned(),
                element_class: None,
                priority: 0,
                registration_order: 0,
                pattern_index: 0,
                pattern: "example".to_owned(),
                span: empty_mapped_span(),
                captures: Vec::new(),
                tags: Vec::new(),
                mark: 0,
                marks: Vec::new(),
                handler: None,
                metadata: Vec::new(),
                children,
            },
        })
    }

    fn structure_with_value_summary(public_data: Vec<WitExpressionPublicData>) -> HookPayload {
        HookPayload::Structure(WitStructurePayload {
            input: "example".to_owned(),
            body_tree: RawTree {
                roots: Vec::new(),
                nodes: Vec::new(),
                diagnostics: Vec::new(),
                indentation: None,
            },
            context: WitParseContext {
                syntax_context: 0,
                event_classes: Vec::new(),
                values: Vec::new(),
            },
            timing: WitStructureTiming::EnterBody,
            type_options: Vec::new(),
            candidate: WitStructureCandidate {
                raw_node_id: 0,
                definition_id: "structure:test:definition".to_owned(),
                registration_id: "structure:test:registration".to_owned(),
                element_class: None,
                priority: 0,
                registration_order: 0,
                pattern_index: 0,
                pattern: "example".to_owned(),
                span: empty_mapped_span(),
                declared_node_type: WitStructureNodeType::Simple,
                actual_node_type: WitStructureNodeType::Simple,
                regex_captures: Vec::new(),
                tags: Vec::new(),
                mark: 0,
                marks: Vec::new(),
                parsed_captures: Vec::new(),
                body_mode: WitStructureBodyMode::None,
                child_node_ids: Vec::new(),
                entries: vec![WitStructureEntry {
                    raw_node_id: None,
                    parent_entry: None,
                    key: "value".to_owned(),
                    entry_data_class: "example.EntryData".to_owned(),
                    kind: WitStructureEntryKind::Expression,
                    source: "source".to_owned(),
                    span: empty_mapped_span(),
                    defaulted: false,
                    value_kind: WitStructureEntryValueKind::Expression,
                    value_summary: Some(WitParseSummary {
                        kind: "grouped".to_owned(),
                        definition_id: None,
                        registration_id: None,
                        element_class: None,
                        pattern_index: None,
                        return_type: None,
                        possible_return_types: Vec::new(),
                        possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
                        multiplicity: None,
                        public_data,
                        metadata: Vec::new(),
                    }),
                }],
                handler: None,
                metadata: Vec::new(),
                declarations: Vec::new(),
            },
        })
    }

    #[test]
    fn registered_expression_child_public_data_mutation_is_rejected() {
        let mut original = registered_expression();
        let HookPayload::RegisteredExpression(value) = &mut original else {
            unreachable!();
        };
        value
            .children
            .push(registered_expression_child(vec![public_data_entry(
                r#"{"name":"before"}"#,
            )]));

        let mut replacement = original.clone();
        let HookPayload::RegisteredExpression(value) = &mut replacement else {
            unreachable!();
        };
        value.children[0].public_data[0].json = r#"{"name":"after"}"#.to_owned();

        assert!(normalize_hook_metadata(&original, &mut replacement, "another.addon").is_err());
    }

    #[test]
    fn condition_child_public_data_counts_toward_hook_size() {
        let public_data = vec![public_data_entry(r#"{"name":"value"}"#)];
        let without = condition_with_children(vec![registered_expression_child(Vec::new())]);
        let with = condition_with_children(vec![registered_expression_child(public_data.clone())]);

        assert_eq!(
            hook_payload_size(&with),
            hook_payload_size(&without).saturating_add(public_data::size(&public_data))
        );
    }

    #[test]
    fn structure_value_summary_public_data_counts_toward_hook_size() {
        let public_data = vec![public_data_entry(r#"{"name":"value"}"#)];
        let without = structure_with_value_summary(Vec::new());
        let with = structure_with_value_summary(public_data.clone());

        assert_eq!(
            hook_payload_size(&with),
            hook_payload_size(&without).saturating_add(public_data::size(&public_data))
        );
    }

    #[test]
    fn structure_entry_summary_unwraps_grouped_expression_public_data() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4");
        let mut host = ParserHost::new(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../artifacts/core-library.wasm"
            )),
            HostConfig {
                syntax_catalog: Some(Arc::new(
                    ssg::load(fixture)
                        .expect("schema 3 fixture must load")
                        .into_catalog(),
                )),
                ..core_test_config()
            },
        )
        .expect("core fixture must initialize");
        let transaction = host
            .begin_parse(
                "file:///workspace",
                "file:///workspace/structure-entry.sk",
                1,
            )
            .expect("parse transaction must initialize");
        let text = "({_balances::*})";
        let source = MappedSource::identity(text);
        let result = host
            .parse_expression_in_parse(
                &transaction,
                InvocationContext {
                    invocation_id: 1,
                    subscription_id: String::new(),
                    document_id: "file:///workspace/structure-entry.sk".to_owned(),
                    document_revision: 1,
                    expansion: None,
                    syntax_context: 7,
                },
                ExpressionParseRequest {
                    source: &source,
                    range: ParserTextRange::new(0, text.len()),
                    expected_types: vec![skript_parser::ExpressionExpectedType {
                        class_name: ClassName("java.lang.Object".to_owned()),
                        plural: true,
                    }],
                    context: ExpressionParseContext {
                        syntax_context: 7,
                        ..ExpressionParseContext::default()
                    },
                },
                ExpressionParserConfig::default(),
            )
            .expect("grouped expression must parse");
        let mut node = result
            .matches
            .selected
            .expect("grouped expression must select a candidate")
            .node;
        assert!(matches!(&node.kind, ExpressionNodeKind::Grouped));
        assert_eq!(node.children.len(), 1);
        let child_public_data = vec![skript_parser::ExpressionPublicData {
            schema_id: "example.semantic".to_owned(),
            schema_version: 1,
            json: r#"{"name":"value"}"#.to_owned(),
        }];
        node.children[0].public_data = child_public_data.clone();
        assert!(node.public_data.is_empty());
        assert_eq!(node.children[0].public_data, child_public_data);

        let entry = StructureEntry {
            raw_node_id: None,
            key: "value".to_owned(),
            entry_data_class: ClassName("example.EntryData".to_owned()),
            kind: EntryKind::Expression,
            source: text.to_owned(),
            span: node.span.clone(),
            defaulted: false,
            value: StructureEntryValue::Expression(Box::new(node)),
        };
        let projected = structure_entry_to_wit(&entry, None);
        let summary = projected
            .value_summary
            .expect("expression entries expose a value summary");
        assert_eq!(summary.public_data.len(), 1);
        assert_eq!(summary.public_data[0].schema_id, "example.semantic");
        assert_eq!(summary.public_data[0].json, r#"{"name":"value"}"#);

        transaction.cancel().expect("parse transaction must close");
    }

    #[test]
    fn registered_expression_time_is_immutable() {
        let HookPayload::RegisteredExpression(original) = registered_expression() else {
            unreachable!();
        };
        let mut changed = original.clone();
        changed.time = -1;

        assert!(!same_registered_expression_identity(&changed, &original));
    }

    #[test]
    fn public_expression_data_can_be_changed_or_removed_without_forging_metadata() {
        let mut original = registered_expression();
        let HookPayload::RegisteredExpression(value) = &mut original else {
            unreachable!()
        };
        value.public_data.push(WitExpressionPublicData {
            schema_id: "example.semantic".to_owned(),
            schema_version: 1,
            json: r#"{"name":"before"}"#.to_owned(),
        });
        let mut replacement = original.clone();
        let HookPayload::RegisteredExpression(value) = &mut replacement else {
            unreachable!()
        };
        value.public_data[0].json = r#"{"name":"after"}"#.to_owned();
        value.effective_return_type = Some("java.lang.Long".to_owned());
        normalize_hook_metadata(&original, &mut replacement, "another.addon").unwrap();
        let HookPayload::RegisteredExpression(value) = &mut replacement else {
            unreachable!()
        };
        value.public_data.clear();
        normalize_hook_metadata(&original, &mut replacement, "another.addon").unwrap();
        let HookPayload::RegisteredExpression(value) = replacement else {
            unreachable!()
        };
        assert!(value.public_data.is_empty());
        assert_eq!(
            value.effective_return_type.as_deref(),
            Some("java.lang.Long")
        );
        assert_eq!(value.metadata[0].key, "phase");
    }

    fn output(decision: HookDecision, replacement: Option<HookPayload>) -> HookOutput {
        HookOutput {
            decision,
            replacement,
            effects: empty_effects(),
        }
    }

    #[test]
    fn rejects_a_property_selection_outside_the_immutable_option_list() {
        let HookPayload::RegisteredExpression(mut payload) = registered_expression() else {
            unreachable!();
        };
        payload.selected_property_option_indices = vec![0];

        assert!(validate_selected_property_options(&payload).is_err());
    }

    #[test]
    fn registry_orders_by_specificity_priority_and_load_order() {
        let mut registry = SubscriptionRegistry::default();
        registry.register(
            0,
            0,
            &[
                subscription(
                    "syntax-first-component",
                    HookTarget::SyntaxKind(SyntaxKind::Expression),
                    -100,
                    HookMode::Observe,
                ),
                subscription(
                    "exact-first-component",
                    HookTarget::Registration("expr.test".to_owned()),
                    0,
                    HookMode::Observe,
                ),
            ],
        );
        registry.register(
            1,
            1,
            &[
                subscription(
                    "exact-higher-priority",
                    HookTarget::Registration("expr.test".to_owned()),
                    -10,
                    HookMode::Observe,
                ),
                subscription(
                    "exact-same-priority-later-load",
                    HookTarget::Registration("expr.test".to_owned()),
                    0,
                    HookMode::Observe,
                ),
                subscription(
                    "pattern-later-load",
                    HookTarget::Pattern(PatternRef {
                        registration_id: "expr.test".to_owned(),
                        pattern_index: 2,
                    }),
                    100,
                    HookMode::Observe,
                ),
            ],
        );

        let matched = registry.matching(
            &DispatchTarget::Pattern {
                definition_id: "expr.definition".to_owned(),
                registration_id: "expr.test".to_owned(),
                pattern_index: 2,
                syntax_kind: SyntaxKind::Expression,
            },
            HookPhase::Document,
        );
        let ids = matched
            .iter()
            .map(|entry| entry.subscription.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                "pattern-later-load",
                "exact-higher-priority",
                "exact-first-component",
                "exact-same-priority-later-load",
                "syntax-first-component",
            ]
        );
    }

    #[test]
    fn selectors_use_three_value_logic_and_current_payload() {
        let mut selector = empty_selector();
        selector.pattern_index = Some(2);
        selector.pattern_source = Some("example %objects%".to_owned());
        selector.mark = Some(4);
        selector.tags.push("test".to_owned());
        selector.return_type = Some(ReturnTypeSelector {
            class_name: "java.lang.String".to_owned(),
            relation: SelectorTypeRelation::Exact,
        });
        selector.multiplicity = Some(WitDynamicMultiplicity::Single);
        selector.metadata.push(WitMetadataEntry {
            owner_component_id: None,
            key: "phase".to_owned(),
            value: "resolved".to_owned(),
        });

        let mut payload = registered_expression();
        assert_eq!(
            selector_match(&selector, &payload, None),
            SelectorMatch::Match
        );

        let HookPayload::RegisteredExpression(value) = &mut payload else {
            unreachable!();
        };
        value.metadata[0].value = "changed".to_owned();
        assert_eq!(
            selector_match(&selector, &payload, None),
            SelectorMatch::NoMatch
        );
        assert_eq!(
            selector_match(&selector, &document("example"), None),
            SelectorMatch::Unknown
        );
    }

    #[test]
    fn metadata_is_owned_and_preserved_by_the_host() {
        let original = vec![WitMetadataEntry {
            owner_component_id: Some("addon.first".to_owned()),
            key: "state".to_owned(),
            value: "ready".to_owned(),
        }];
        let mut replacement = vec![WitMetadataEntry {
            owner_component_id: None,
            key: "result".to_owned(),
            value: "accepted".to_owned(),
        }];
        merge_owned_metadata(&original, &mut replacement, "addon.second")
            .expect("a component may add its own metadata");
        assert_eq!(
            replacement[0].owner_component_id.as_deref(),
            Some("addon.second")
        );
        assert!(replacement.iter().any(|entry| {
            entry.owner_component_id.as_deref() == Some("addon.first") && entry.key == "state"
        }));

        let mut spoofed = vec![WitMetadataEntry {
            owner_component_id: Some("addon.first".to_owned()),
            key: "state".to_owned(),
            value: "changed".to_owned(),
        }];
        assert!(merge_owned_metadata(&original, &mut spoofed, "addon.second").is_err());
    }

    #[test]
    fn registry_detects_only_matching_hook_subscriptions() {
        let mut registry = SubscriptionRegistry::default();
        registry.register(
            0,
            0,
            &[subscription(
                "document",
                HookTarget::ParseStage,
                0,
                HookMode::Observe,
            )],
        );
        assert!(!registry.has_matching_hooks());

        let mut matching = subscription(
            "matching",
            HookTarget::SyntaxKind(SyntaxKind::Expression),
            0,
            HookMode::Transform,
        );
        matching.phase = HookPhase::Matching;
        registry.register(1, 1, &[matching]);

        assert!(registry.has_matching_hooks());
    }

    #[test]
    fn pattern_hook_prefilter_matches_only_its_pattern_index() {
        let host = ParserHost::new(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../artifacts/core-library.wasm"
            )),
            core_test_config(),
        )
        .expect("core fixture must initialize");
        let mut registry = SubscriptionRegistry::default();
        let mut matching = subscription(
            "matching.pattern",
            HookTarget::Pattern(PatternRef {
                registration_id: "effect:fixture".to_owned(),
                pattern_index: 1,
            }),
            0,
            HookMode::Override,
        );
        matching.phase = HookPhase::Matching;
        registry.register(0, 0, &[matching]);

        assert!(registry.has_active_matching_handler_for_registration(
            &host.components,
            SyntaxKind::Effect,
            None,
            "effect:fixture",
            1,
        ));
        assert!(!registry.has_active_matching_handler_for_registration(
            &host.components,
            SyntaxKind::Effect,
            None,
            "effect:fixture",
            0,
        ));
    }

    #[test]
    fn host_only_advertises_capabilities_implemented_by_this_pipeline() {
        let ids = host_capabilities()
            .into_iter()
            .map(|capability| capability.id)
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                CAPABILITY_HOOKS,
                CAPABILITY_STATE_STORE,
                CAPABILITY_TEXT_MACRO,
                CAPABILITY_TREE_MACRO,
                CAPABILITY_CONTEXT_UPDATES,
                CAPABILITY_ADDITIONAL_PARSE,
                CAPABILITY_EXPRESSION_PARSER,
                CAPABILITY_CONDITION_PARSER,
                CAPABILITY_EFFECT_PARSER,
                CAPABILITY_SECTION_PARSER,
                CAPABILITY_STRUCTURE_PARSER,
            ]
        );
    }

    #[test]
    fn expression_type_options_preserve_supplier_metadata_and_identity() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4");
        let snapshot = ssg::load(path).expect("SSG fixture must load");
        let catalog = snapshot.catalog();
        let expected = catalog
            .types()
            .find(|value| value.has_supplier)
            .expect("fixture must contain a supplier-backed type");
        let options = all_expression_type_options(Some(catalog));
        let actual = options
            .iter()
            .find(|value| value.code_name == expected.code_name.as_str())
            .expect("supplier-backed type must be exposed");

        assert_eq!(actual.has_parser, expected.has_parser);
        assert_eq!(actual.has_supplier, expected.has_supplier);
        assert!(same_expression_type_options(&options, &options));

        let mut changed = options.clone();
        changed
            .iter_mut()
            .find(|value| value.code_name == expected.code_name.as_str())
            .expect("supplier-backed type must be exposed")
            .has_supplier = false;
        assert!(!same_expression_type_options(&options, &changed));

        let mut changed = options.clone();
        changed
            .iter_mut()
            .find(|value| value.code_name == expected.code_name.as_str())
            .expect("supplier-backed type must be exposed")
            .parser_class = Some("fixture.RewrittenParser".to_owned());
        assert!(!same_expression_type_options(&options, &changed));
    }

    #[test]
    fn expression_type_options_skip_unrelated_class_info_patterns() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4");
        let snapshot = ssg::load(path).expect("SSG fixture must load");
        let catalog = snapshot.catalog();
        let expected = [skript_parser::ExpressionExpectedType {
            class_name: ClassName("java.lang.Object".to_owned()),
            plural: false,
        }];

        let unrelated = expression_type_options(
            Some(catalog),
            "function fixture()",
            ParserTextRange::new(0, 18),
            &[18],
            &expected,
        );
        assert!(unrelated.is_empty());

        let player = expression_type_options(
            Some(catalog),
            "player",
            ParserTextRange::new(0, 6),
            &[6],
            &expected,
        );
        assert!(
            player
                .iter()
                .any(|option| option.class_name == "org.bukkit.entity.Player")
        );
    }

    #[test]
    fn reports_abi_mismatches_as_typed_compatibility_errors() {
        use crate::bindings::nlaocs::skript_parser_addon::types::AbiVersion as WitAbiVersion;
        let manifest = ComponentManifest {
            component_id: "test.incompatible".to_owned(),
            component_version: "1.0.0".to_owned(),
            abi: WitAbiVersion { major: 2, minor: 0 },
            capabilities: Vec::new(),
            subscriptions: Vec::new(),
            registered_syntax_handlers: Vec::new(),
            catalog_annotations: Vec::new(),
            state_namespaces: Vec::new(),
        };
        let error = validate_manifest(&manifest, &host_capabilities()).unwrap_err();
        assert!(matches!(
            error,
            HostError::Compatibility {
                source: CompatibilityError::AbiVersionMismatch { .. },
                ..
            }
        ));
    }

    #[test]
    fn text_macro_subscriptions_use_the_dedicated_pipeline_shape() {
        use crate::bindings::nlaocs::skript_parser_addon::types::{
            AbiVersion as WitAbiVersion, CapabilityRequirement as WitCapabilityRequirement,
        };

        let subscription = HookSubscription {
            id: "text.expand".to_owned(),
            target: HookTarget::ParseStage,
            phase: HookPhase::Preprocess,
            priority: 0,
            mode: HookMode::Transform,
            capability_id: CAPABILITY_TEXT_MACRO.to_owned(),
            selector: empty_selector(),
        };
        let manifest = |subscription| ComponentManifest {
            component_id: "test.text-macro-contract".to_owned(),
            component_version: "1.0.0".to_owned(),
            abi: WitAbiVersion {
                major: ABI_VERSION.major,
                minor: ABI_VERSION.minor,
            },
            capabilities: vec![WitCapabilityRequirement {
                id: CAPABILITY_TEXT_MACRO.to_owned(),
                minimum_version: 1,
                required: true,
            }],
            subscriptions: vec![subscription],
            registered_syntax_handlers: Vec::new(),
            catalog_annotations: Vec::new(),
            state_namespaces: Vec::new(),
        };

        validate_manifest(&manifest(subscription.clone()), &host_capabilities())
            .expect("the dedicated Text macro pipeline shape must be accepted");

        let mut invalid_target = subscription.clone();
        invalid_target.target = HookTarget::SyntaxKind(SyntaxKind::Expression);
        let mut invalid_phase = subscription.clone();
        invalid_phase.phase = HookPhase::Document;
        let mut invalid_mode = subscription;
        invalid_mode.mode = HookMode::Observe;

        for (name, invalid) in [
            ("target", invalid_target),
            ("phase", invalid_phase),
            ("mode", invalid_mode),
        ] {
            let error = validate_manifest(&manifest(invalid), &host_capabilities())
                .expect_err("an invalid Text macro subscription must be rejected");
            assert!(
                matches!(error, HostError::InvalidManifest { .. }),
                "unexpected {name} validation error: {error}"
            );
        }
    }

    #[test]
    fn tree_macro_subscriptions_use_the_dedicated_pipeline_shape() {
        use crate::bindings::nlaocs::skript_parser_addon::types::{
            AbiVersion as WitAbiVersion, CapabilityRequirement as WitCapabilityRequirement,
        };

        let subscription = HookSubscription {
            id: "tree.expand".to_owned(),
            target: HookTarget::ParseStage,
            phase: HookPhase::Tree,
            priority: 0,
            mode: HookMode::Transform,
            capability_id: CAPABILITY_TREE_MACRO.to_owned(),
            selector: empty_selector(),
        };
        let manifest = |subscription| ComponentManifest {
            component_id: "test.tree-macro-contract".to_owned(),
            component_version: "1.0.0".to_owned(),
            abi: WitAbiVersion {
                major: ABI_VERSION.major,
                minor: ABI_VERSION.minor,
            },
            capabilities: vec![WitCapabilityRequirement {
                id: CAPABILITY_TREE_MACRO.to_owned(),
                minimum_version: 1,
                required: true,
            }],
            subscriptions: vec![subscription],
            registered_syntax_handlers: Vec::new(),
            catalog_annotations: Vec::new(),
            state_namespaces: Vec::new(),
        };

        validate_manifest(&manifest(subscription.clone()), &host_capabilities())
            .expect("the dedicated Tree macro pipeline shape must be accepted");

        let mut invalid_target = subscription.clone();
        invalid_target.target = HookTarget::SyntaxKind(SyntaxKind::Section);
        let mut invalid_phase = subscription.clone();
        invalid_phase.phase = HookPhase::Node;
        let mut invalid_mode = subscription;
        invalid_mode.mode = HookMode::Override;

        for (name, invalid) in [
            ("target", invalid_target),
            ("phase", invalid_phase),
            ("mode", invalid_mode),
        ] {
            let error = validate_manifest(&manifest(invalid), &host_capabilities())
                .expect_err("an invalid Tree macro subscription must be rejected");
            assert!(
                matches!(error, HostError::InvalidManifest { .. }),
                "unexpected {name} validation error: {error}"
            );
        }
    }
    #[test]
    fn expression_subscriptions_use_the_dedicated_pipeline_shape() {
        use crate::bindings::nlaocs::skript_parser_addon::types::{
            AbiVersion as WitAbiVersion, CapabilityRequirement as WitCapabilityRequirement,
            RegisteredSyntaxHandler,
        };

        let subscription = HookSubscription {
            id: "expression.parse".to_owned(),
            target: HookTarget::ParseStage,
            phase: HookPhase::Expression,
            priority: 0,
            mode: HookMode::Transform,
            capability_id: CAPABILITY_EXPRESSION_PARSER.to_owned(),
            selector: empty_selector(),
        };
        let manifest = |subscription| ComponentManifest {
            component_id: "test.expression-contract".to_owned(),
            component_version: "1.0.0".to_owned(),
            abi: WitAbiVersion {
                major: ABI_VERSION.major,
                minor: ABI_VERSION.minor,
            },
            capabilities: vec![WitCapabilityRequirement {
                id: CAPABILITY_EXPRESSION_PARSER.to_owned(),
                minimum_version: 1,
                required: true,
            }],
            subscriptions: vec![subscription],
            registered_syntax_handlers: Vec::new(),
            catalog_annotations: Vec::new(),
            state_namespaces: Vec::new(),
        };

        validate_manifest(&manifest(subscription.clone()), &host_capabilities())
            .expect("the dedicated Expression pipeline shape must be accepted");

        let mut with_handler = manifest(subscription.clone());
        with_handler.registered_syntax_handlers = vec![RegisteredSyntaxHandler {
            handler_id: "test.expr-parse".to_owned(),
            kind: SyntaxKind::Expression,
            targets: vec![RegisteredSyntaxHandlerTarget::ClassSuffix(
                ".ExprParse".to_owned(),
            )],
            pattern_indices: Vec::new(),
            pattern_sources: Vec::new(),
            required_tags: Vec::new(),
            forbidden_tags: Vec::new(),
            marks: Vec::new(),
            capture_parsers: Vec::new(),
            context_requirements: vec![REGISTERED_CONTEXT_ALL_TYPE_OPTIONS.to_owned()],
        }];
        validate_manifest(&with_handler, &host_capabilities())
            .expect("an Expression transform may declare handled registrations");

        for suffixes in [vec![""], vec![".ExprParse", ".ExprParse"]] {
            let mut invalid = manifest(subscription.clone());
            invalid.registered_syntax_handlers = suffixes
                .into_iter()
                .map(|suffix| RegisteredSyntaxHandler {
                    handler_id: "test.expr-parse".to_owned(),
                    kind: SyntaxKind::Expression,
                    targets: vec![RegisteredSyntaxHandlerTarget::ClassSuffix(
                        suffix.to_owned(),
                    )],
                    pattern_indices: Vec::new(),
                    pattern_sources: Vec::new(),
                    required_tags: Vec::new(),
                    forbidden_tags: Vec::new(),
                    marks: Vec::new(),
                    capture_parsers: Vec::new(),
                    context_requirements: Vec::new(),
                })
                .collect();
            assert!(matches!(
                validate_manifest(&invalid, &host_capabilities()),
                Err(HostError::InvalidManifest { .. })
            ));
        }

        let mut missing_transform = with_handler;
        missing_transform.subscriptions.clear();
        assert!(matches!(
            validate_manifest(&missing_transform, &host_capabilities()),
            Err(HostError::InvalidManifest { .. })
        ));

        let mut invalid_target = subscription.clone();
        invalid_target.target = HookTarget::SyntaxKind(SyntaxKind::Effect);
        let mut invalid_phase = subscription.clone();
        invalid_phase.phase = HookPhase::Candidate;
        let mut invalid_mode = subscription;
        invalid_mode.mode = HookMode::Override;

        for invalid in [invalid_target, invalid_phase, invalid_mode] {
            assert!(matches!(
                validate_manifest(&manifest(invalid), &host_capabilities()),
                Err(HostError::InvalidManifest { .. })
            ));
        }
    }

    #[test]
    fn effect_subscriptions_use_effect_targets_and_phase() {
        use crate::bindings::nlaocs::skript_parser_addon::types::{
            AbiVersion as WitAbiVersion, CapabilityRequirement as WitCapabilityRequirement,
        };

        let subscription = HookSubscription {
            id: "effect.parse".to_owned(),
            target: HookTarget::SyntaxKind(SyntaxKind::Effect),
            phase: HookPhase::Effect,
            priority: 0,
            mode: HookMode::Transform,
            capability_id: CAPABILITY_EFFECT_PARSER.to_owned(),
            selector: empty_selector(),
        };
        let manifest = |subscription| ComponentManifest {
            component_id: "test.effect-contract".to_owned(),
            component_version: "1.0.0".to_owned(),
            abi: WitAbiVersion {
                major: ABI_VERSION.major,
                minor: ABI_VERSION.minor,
            },
            capabilities: vec![WitCapabilityRequirement {
                id: CAPABILITY_EFFECT_PARSER.to_owned(),
                minimum_version: 1,
                required: true,
            }],
            subscriptions: vec![subscription],
            registered_syntax_handlers: Vec::new(),
            catalog_annotations: Vec::new(),
            state_namespaces: Vec::new(),
        };

        validate_manifest(&manifest(subscription.clone()), &host_capabilities())
            .expect("an Effect category subscription must be accepted");
        let mut exact = subscription.clone();
        exact.target = HookTarget::Registration("effect:test#0".to_owned());
        exact.mode = HookMode::Override;
        validate_manifest(&manifest(exact), &host_capabilities())
            .expect("an exact Effect override must be accepted");

        let mut invalid_target = subscription.clone();
        invalid_target.target = HookTarget::ParseStage;
        let mut invalid_kind = subscription.clone();
        invalid_kind.target = HookTarget::SyntaxKind(SyntaxKind::Expression);
        let mut invalid_phase = subscription;
        invalid_phase.phase = HookPhase::Candidate;
        for invalid in [invalid_target, invalid_kind, invalid_phase] {
            assert!(matches!(
                validate_manifest(&manifest(invalid), &host_capabilities()),
                Err(HostError::InvalidManifest { .. })
            ));
        }
    }

    #[test]
    fn section_subscriptions_use_section_targets_and_phase() {
        use crate::bindings::nlaocs::skript_parser_addon::types::{
            AbiVersion as WitAbiVersion, CapabilityRequirement as WitCapabilityRequirement,
            CaptureParserBinding, RegisteredSyntaxHandler,
        };

        let subscription = HookSubscription {
            id: "section.parse".to_owned(),
            target: HookTarget::SyntaxKind(SyntaxKind::Section),
            phase: HookPhase::Section,
            priority: 0,
            mode: HookMode::Transform,
            capability_id: CAPABILITY_SECTION_PARSER.to_owned(),
            selector: empty_selector(),
        };
        let manifest = |subscription| ComponentManifest {
            component_id: "test.section-contract".to_owned(),
            component_version: "1.0.0".to_owned(),
            abi: WitAbiVersion {
                major: ABI_VERSION.major,
                minor: ABI_VERSION.minor,
            },
            capabilities: vec![WitCapabilityRequirement {
                id: CAPABILITY_SECTION_PARSER.to_owned(),
                minimum_version: 1,
                required: true,
            }],
            subscriptions: vec![subscription],
            registered_syntax_handlers: vec![RegisteredSyntaxHandler {
                handler_id: "test.sec-conditional".to_owned(),
                kind: SyntaxKind::Section,
                targets: vec![RegisteredSyntaxHandlerTarget::ClassSuffix(
                    ".SecConditional".to_owned(),
                )],
                pattern_indices: Vec::new(),
                pattern_sources: Vec::new(),
                required_tags: Vec::new(),
                forbidden_tags: Vec::new(),
                marks: Vec::new(),
                capture_parsers: vec![CaptureParserBinding {
                    capture_index: 0,
                    parser_id: "host.condition".to_owned(),
                    required: true,
                    options: Vec::new(),
                }],
                context_requirements: Vec::new(),
            }],
            catalog_annotations: Vec::new(),
            state_namespaces: Vec::new(),
        };

        validate_manifest(&manifest(subscription.clone()), &host_capabilities())
            .expect("a Section handler may claim Condition captures");

        let mut invalid_target = subscription.clone();
        invalid_target.target = HookTarget::ParseStage;
        let mut invalid_phase = subscription.clone();
        invalid_phase.phase = HookPhase::Candidate;
        for invalid in [invalid_target, invalid_phase] {
            assert!(matches!(
                validate_manifest(&manifest(invalid), &host_capabilities()),
                Err(HostError::InvalidManifest { .. })
            ));
        }

        let mut invalid_capture = manifest(subscription);
        invalid_capture.registered_syntax_handlers[0].capture_parsers[0].parser_id = " ".to_owned();
        assert!(matches!(
            validate_manifest(&invalid_capture, &host_capabilities()),
            Err(HostError::InvalidManifest { .. })
        ));
    }

    #[test]
    fn transform_hooks_compose_replacements() {
        let first = apply_hook_output(
            HookMode::Transform,
            output(HookDecision::ContinueProcessing, Some(document("first"))),
            document("original"),
        )
        .expect("first transform must apply");
        let second = apply_hook_output(
            HookMode::Transform,
            output(HookDecision::ContinueProcessing, Some(document("second"))),
            first.payload,
        )
        .expect("second transform must apply");
        let HookPayload::Document(value) = second.payload else {
            panic!("payload kind must be preserved");
        };
        assert_eq!(value.text, "second");
        assert!(!second.terminal);
    }

    #[test]
    fn override_stops_at_the_first_handled_result() {
        let applied = apply_hook_output(
            HookMode::Override,
            output(HookDecision::Handled, Some(document("handled"))),
            document("original"),
        )
        .expect("handled override must apply");
        assert!(applied.terminal);
        assert!(matches!(applied.decision, Some(HookDecision::Handled)));
    }

    #[test]
    fn observe_hooks_cannot_modify_or_control_flow() {
        let replacement = apply_hook_output(
            HookMode::Observe,
            output(HookDecision::ContinueProcessing, Some(document("changed"))),
            document("original"),
        );
        assert_eq!(
            replacement.unwrap_err(),
            "observe hooks cannot replace payloads"
        );

        let handled = apply_hook_output(
            HookMode::Observe,
            output(HookDecision::Handled, None),
            document("original"),
        );
        assert_eq!(
            handled.unwrap_err(),
            "observe hooks cannot control parser flow"
        );
    }

    fn test_engine() -> Engine {
        let mut config = Config::new();
        config.consume_fuel(true);
        config.epoch_interruption(true);
        Engine::new(&config).expect("test engine must initialize")
    }

    fn unbounded_module(engine: &Engine) -> WasmtimeModule {
        let mut types = TypeSection::new();
        types.ty().function([], []);
        let mut functions = FunctionSection::new();
        functions.function(0);
        let mut exports = ExportSection::new();
        exports.export("run", ExportKind::Func, 0);
        let mut body = Function::new([]);
        body.instruction(&Instruction::Loop(BlockType::Empty));
        body.instruction(&Instruction::Br(0));
        body.instruction(&Instruction::End);
        body.instruction(&Instruction::End);
        let mut code = CodeSection::new();
        code.function(&body);
        let mut module = EncodedModule::new();
        module.section(&types);
        module.section(&functions);
        module.section(&exports);
        module.section(&code);
        WasmtimeModule::new(engine, module.finish()).expect("module must compile")
    }

    #[test]
    fn fuel_interrupts_unbounded_wasm_execution() {
        let engine = test_engine();
        let module = unbounded_module(&engine);
        let mut store = Store::new(&engine, ());
        store.set_fuel(100).expect("fuel must be enabled");
        store.set_epoch_deadline(u64::MAX);
        let instance = Instance::new(&mut store, &module, &[]).expect("module must instantiate");
        let run = instance
            .get_typed_func::<(), ()>(&mut store, "run")
            .expect("run export must exist");
        let error = run.call(&mut store, ()).expect_err("fuel must interrupt");
        let classified = classify_wasmtime_error("test".to_owned(), "hook", error);
        assert!(matches!(classified, HostError::FuelExhausted { .. }));
    }

    #[test]
    fn store_memory_limit_traps_growth() {
        let engine = test_engine();
        let mut types = TypeSection::new();
        types.ty().function([], [ValType::I32]);
        let mut functions = FunctionSection::new();
        functions.function(0);
        let mut memories = MemorySection::new();
        memories.memory(MemoryType {
            minimum: 1,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        let mut exports = ExportSection::new();
        exports.export("grow", ExportKind::Func, 0);
        let mut body = Function::new([]);
        body.instruction(&Instruction::I32Const(1));
        body.instruction(&Instruction::MemoryGrow(0));
        body.instruction(&Instruction::End);
        let mut code = CodeSection::new();
        code.function(&body);
        let mut encoded = EncodedModule::new();
        encoded.section(&types);
        encoded.section(&functions);
        encoded.section(&memories);
        encoded.section(&exports);
        encoded.section(&code);
        let module = WasmtimeModule::new(&engine, encoded.finish()).expect("module must compile");
        let config = HostConfig {
            max_memory_bytes: 64 * 1024,
            ..HostConfig::default()
        };
        let mut store = create_store(
            &engine,
            &config,
            build_type_user_input_matchers(config.syntax_catalog.as_deref()),
        );
        prepare_store(&mut store, 10_000, u64::MAX, "test", "hook")
            .expect("store budget must initialize");
        let instance = Instance::new(&mut store, &module, &[]).expect("module must instantiate");
        let grow = instance
            .get_typed_func::<(), i32>(&mut store, "grow")
            .expect("grow export must exist");
        let error = grow
            .call(&mut store, ())
            .expect_err("memory growth beyond the quota must trap");
        let classified = classify_wasmtime_error("test".to_owned(), "hook", error);
        assert!(matches!(classified, HostError::ResourceLimit { .. }));
    }

    #[test]
    fn epoch_deadline_interrupts_execution() {
        let engine = test_engine();
        let ticker = EpochTicker::start(engine.clone(), Duration::from_millis(1))
            .expect("ticker must start");
        let module = unbounded_module(&engine);
        let mut store = Store::new(&engine, ());
        store.set_fuel(u64::MAX).expect("fuel must be enabled");
        store.set_epoch_deadline(1);
        let instance = Instance::new(&mut store, &module, &[]).expect("module must instantiate");
        let run = instance
            .get_typed_func::<(), ()>(&mut store, "run")
            .expect("run export must exist");
        let error = run
            .call(&mut store, ())
            .expect_err("epoch deadline must interrupt");
        drop(ticker);
        let classified = classify_wasmtime_error("test".to_owned(), "hook", error);
        assert!(matches!(classified, HostError::Timeout { .. }));
    }

    #[test]
    fn a_trap_does_not_poison_the_wasmtime_store() {
        let engine = test_engine();
        let mut types = TypeSection::new();
        types.ty().function([], []);
        let mut functions = FunctionSection::new();
        functions.function(0);
        functions.function(0);
        let mut exports = ExportSection::new();
        exports.export("trap", ExportKind::Func, 0);
        exports.export("healthy", ExportKind::Func, 1);
        let mut trap_body = Function::new([]);
        trap_body.instruction(&Instruction::Unreachable);
        trap_body.instruction(&Instruction::End);
        let mut healthy_body = Function::new([]);
        healthy_body.instruction(&Instruction::End);
        let mut code = CodeSection::new();
        code.function(&trap_body);
        code.function(&healthy_body);
        let mut encoded = EncodedModule::new();
        encoded.section(&types);
        encoded.section(&functions);
        encoded.section(&exports);
        encoded.section(&code);
        let module = WasmtimeModule::new(&engine, encoded.finish()).expect("module must compile");
        let mut store = Store::new(&engine, ());
        store.set_fuel(10_000).expect("fuel must be enabled");
        store.set_epoch_deadline(u64::MAX);
        let instance = Instance::new(&mut store, &module, &[]).expect("module must instantiate");
        let trap = instance
            .get_typed_func::<(), ()>(&mut store, "trap")
            .expect("trap export must exist");
        let healthy = instance
            .get_typed_func::<(), ()>(&mut store, "healthy")
            .expect("healthy export must exist");
        assert!(trap.call(&mut store, ()).is_err());
        healthy
            .call(&mut store, ())
            .expect("a trapped call must not poison later calls");
    }

    #[test]
    fn prefixed_literal_suffixes_follow_item_type_prefixes() {
        assert_eq!(prefixed_literal_suffix_offset("2 stone"), Some(2));
        assert_eq!(prefixed_literal_suffix_offset("2 of stone"), Some(5));
        assert_eq!(prefixed_literal_suffix_offset("2 of every stone"), Some(11));
        assert_eq!(prefixed_literal_suffix_offset("all stone"), Some(4));
        assert_eq!(prefixed_literal_suffix_offset("stone"), None);
        assert_eq!(prefixed_literal_suffix_offset("of stone"), None);
    }

    #[test]
    fn class_suffix_handlers_resolve_to_fixture_ids() {
        use crate::bindings::nlaocs::skript_parser_addon::types::RegisteredSyntaxHandler;

        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4");
        let snapshot = ssg::load(path).expect("multi-addon fixture must load");
        let catalog = snapshot.catalog();
        let (syntax, element_class) = catalog
            .syntaxes()
            .iter()
            .find_map(|syntax| syntax_element_class(syntax).map(|class| (syntax, class)))
            .expect("fixture must contain a syntax element class");
        let class_name = element_class.as_str().to_owned();
        let handler = RegisteredSyntaxHandler {
            handler_id: "fixture.class-suffix".to_owned(),
            kind: match syntax.kind() {
                CatalogSyntaxKind::Event => SyntaxKind::Event,
                CatalogSyntaxKind::Condition => SyntaxKind::Condition,
                CatalogSyntaxKind::Effect => SyntaxKind::Effect,
                CatalogSyntaxKind::Expression => SyntaxKind::Expression,
                CatalogSyntaxKind::Type => SyntaxKind::Type,
                CatalogSyntaxKind::Function => SyntaxKind::Function,
                CatalogSyntaxKind::Section => SyntaxKind::Section,
                CatalogSyntaxKind::Structure => SyntaxKind::Structure,
            },
            targets: vec![RegisteredSyntaxHandlerTarget::ClassSuffix(
                class_name.clone(),
            )],
            pattern_indices: Vec::new(),
            pattern_sources: Vec::new(),
            required_tags: Vec::new(),
            forbidden_tags: Vec::new(),
            marks: Vec::new(),
            capture_parsers: Vec::new(),
            context_requirements: Vec::new(),
        };

        let resolved = resolve_registered_handler_target(&handler, Some(catalog), false);
        assert!(
            resolved
                .0
                .iter()
                .any(|id| id == syntax.definition_id().as_str())
        );
        assert!(
            resolved
                .1
                .iter()
                .any(|id| id == syntax.registration_id().as_str())
        );
        assert!(!resolved.0.iter().any(|id| id == &class_name));
        assert!(!resolved.1.iter().any(|id| id == &class_name));

        let addon_syntax = catalog
            .syntaxes()
            .iter()
            .find(|syntax| {
                syntax
                    .common()
                    .is_some_and(|common| !common.addon.name.eq_ignore_ascii_case("Skript"))
                    && syntax_element_class(syntax).is_some()
            })
            .expect("multi-addon fixture must contain addon-owned syntax");
        let addon_class = syntax_element_class(addon_syntax).unwrap();
        let addon_handler = RegisteredSyntaxHandler {
            handler_id: "fixture.addon-suffix".to_owned(),
            kind: match addon_syntax.kind() {
                CatalogSyntaxKind::Event => SyntaxKind::Event,
                CatalogSyntaxKind::Condition => SyntaxKind::Condition,
                CatalogSyntaxKind::Effect => SyntaxKind::Effect,
                CatalogSyntaxKind::Expression => SyntaxKind::Expression,
                CatalogSyntaxKind::Type => SyntaxKind::Type,
                CatalogSyntaxKind::Function => SyntaxKind::Function,
                CatalogSyntaxKind::Section => SyntaxKind::Section,
                CatalogSyntaxKind::Structure => SyntaxKind::Structure,
            },
            targets: vec![RegisteredSyntaxHandlerTarget::ClassSuffix(
                addon_class.as_str().to_owned(),
            )],
            pattern_indices: Vec::new(),
            pattern_sources: Vec::new(),
            required_tags: Vec::new(),
            forbidden_tags: Vec::new(),
            marks: Vec::new(),
            capture_parsers: Vec::new(),
            context_requirements: Vec::new(),
        };
        assert!(
            !resolve_registered_handler_target(&addon_handler, Some(catalog), false)
                .1
                .is_empty()
        );
        assert!(
            resolve_registered_handler_target(&addon_handler, Some(catalog), true)
                .1
                .is_empty(),
            "CoreLibrary suffix bindings must not claim addon-owned registrations"
        );
    }

    #[test]
    fn parser_class_handlers_resolve_type_registrations() {
        use crate::bindings::nlaocs::skript_parser_addon::types::RegisteredSyntaxHandler;

        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4");
        let snapshot = ssg::load(path).expect("multi-addon fixture must load");
        let source = snapshot.catalog();
        let mut syntaxes = source.syntaxes().to_vec();
        let type_info = syntaxes
            .iter_mut()
            .find_map(|syntax| match syntax {
                Syntax::Type(value) => Some(value),
                _ => None,
            })
            .expect("fixture must contain a Type");
        type_info.parser_class = Some(ClassName("fixture.NumberParser".to_owned()));
        let catalog = Catalog::new(syntaxes::CatalogParts {
            syntaxes,
            converters: source.converters().to_vec(),
            comparators: source.comparators().to_vec(),
            event_values: source.event_values().to_vec(),
            properties: source.properties().to_vec(),
            operators: source.operators().to_vec(),
            operations: source.operations().clone(),
            differences: source.differences().to_vec(),
            classes: source.classes().to_vec(),
            aliases: source.aliases().clone(),
            plural_rules: source.plural_rules().clone(),
            language: source
                .language_entries()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect(),
        });
        let type_info = catalog
            .types()
            .find(|value| value.parser_class.is_some())
            .expect("fixture must contain a Type parser class");
        let parser_class = type_info.parser_class.as_ref().unwrap().as_str().to_owned();
        let handler = RegisteredSyntaxHandler {
            handler_id: "fixture.parser-class".to_owned(),
            kind: SyntaxKind::Type,
            targets: vec![RegisteredSyntaxHandlerTarget::ParserClass(
                parser_class.clone(),
            )],
            pattern_indices: Vec::new(),
            pattern_sources: Vec::new(),
            required_tags: Vec::new(),
            forbidden_tags: Vec::new(),
            marks: Vec::new(),
            capture_parsers: Vec::new(),
            context_requirements: Vec::new(),
        };

        let (definition_ids, registration_ids) =
            resolve_registered_handler_target(&handler, Some(&catalog), false);
        assert!(
            definition_ids
                .iter()
                .any(|id| id == type_info.definition_id.as_str())
        );
        assert!(
            registration_ids
                .iter()
                .any(|id| id == type_info.registration_id.as_str())
        );

        let option = expression_type_option(Some(&catalog), type_info);
        assert_eq!(option.addon_name, type_info.addon.name);
        assert_eq!(option.addon_version, type_info.addon.version);
        assert_eq!(option.parser_class.as_deref(), Some(parser_class.as_str()));
        assert_eq!(
            option.before,
            type_info
                .before
                .iter()
                .map(|value| value.as_str().to_owned())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            option.after,
            type_info
                .after
                .iter()
                .map(|value| value.as_str().to_owned())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn superclass_handlers_resolve_every_matching_fixture_registration() {
        use crate::bindings::nlaocs::skript_parser_addon::types::RegisteredSyntaxHandler;

        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4");
        let snapshot = ssg::load(path).expect("multi-addon fixture must load");
        let catalog = snapshot.catalog();
        let (syntax, super_class) = catalog
            .syntaxes()
            .iter()
            .find_map(|syntax| syntax_super_class(syntax).map(|class| (syntax, class)))
            .expect("fixture must contain a syntax superclass");
        let handler = RegisteredSyntaxHandler {
            handler_id: "fixture.superclass".to_owned(),
            kind: match syntax.kind() {
                CatalogSyntaxKind::Event => SyntaxKind::Event,
                CatalogSyntaxKind::Condition => SyntaxKind::Condition,
                CatalogSyntaxKind::Effect => SyntaxKind::Effect,
                CatalogSyntaxKind::Expression => SyntaxKind::Expression,
                CatalogSyntaxKind::Type => SyntaxKind::Type,
                CatalogSyntaxKind::Function => SyntaxKind::Function,
                CatalogSyntaxKind::Section => SyntaxKind::Section,
                CatalogSyntaxKind::Structure => SyntaxKind::Structure,
            },
            targets: vec![RegisteredSyntaxHandlerTarget::SuperClass(
                super_class.as_str().to_owned(),
            )],
            pattern_indices: Vec::new(),
            pattern_sources: Vec::new(),
            required_tags: Vec::new(),
            forbidden_tags: Vec::new(),
            marks: Vec::new(),
            capture_parsers: Vec::new(),
            context_requirements: Vec::new(),
        };

        let resolved = resolve_registered_handler_target(&handler, Some(catalog), false);
        let expected = catalog
            .syntaxes()
            .iter()
            .filter(|candidate| candidate.kind() == syntax.kind())
            .filter(|candidate| syntax_super_class(candidate) == Some(super_class))
            .count();
        assert_eq!(resolved.1.len(), expected);
        assert!(
            resolved
                .1
                .iter()
                .any(|id| id == syntax.registration_id().as_str())
        );
    }

    #[test]
    fn catalog_annotation_targets_match_only_their_dispatch_scope() {
        let pattern = PatternRef {
            registration_id: "effect:test:registration".to_owned(),
            pattern_index: 3,
        };
        let requested_pattern = DispatchTarget::Pattern {
            definition_id: "effect:test:definition".to_owned(),
            registration_id: pattern.registration_id.clone(),
            pattern_index: pattern.pattern_index,
            syntax_kind: SyntaxKind::Effect,
        };
        let requested_definition = DispatchTarget::Definition {
            definition_id: "effect:test:definition".to_owned(),
            syntax_kind: SyntaxKind::Effect,
        };
        let requested_registration = DispatchTarget::Registration {
            definition_id: "effect:test:definition".to_owned(),
            registration_id: pattern.registration_id.clone(),
            syntax_kind: SyntaxKind::Effect,
        };

        let definition = CatalogAnnotationTarget::Definition("effect:test:definition".to_owned());
        let registration =
            CatalogAnnotationTarget::Registration("effect:test:registration".to_owned());
        let pattern_target = CatalogAnnotationTarget::Pattern(pattern);

        assert!(annotation_matches(&definition, &requested_pattern));
        assert!(annotation_matches(&definition, &requested_definition));
        assert!(!annotation_matches(
            &definition,
            &DispatchTarget::SyntaxKind(SyntaxKind::Effect)
        ));
        assert!(annotation_matches(&registration, &requested_pattern));
        assert!(annotation_matches(&registration, &requested_registration));
        assert!(!annotation_matches(&registration, &requested_definition));
        assert!(annotation_matches(&pattern_target, &requested_pattern));
        assert!(!annotation_matches(
            &pattern_target,
            &requested_registration
        ));
        assert!(!annotation_matches(&pattern_target, &requested_definition));
        assert!(
            catalog_annotation_specificity(&definition)
                < catalog_annotation_specificity(&registration)
        );
        assert!(
            catalog_annotation_specificity(&registration)
                < catalog_annotation_specificity(&pattern_target)
        );
    }

    #[test]
    fn catalog_annotation_ownership_is_host_stamped() {
        use crate::bindings::nlaocs::skript_parser_addon::types::{
            AbiVersion as WitAbiVersion, CatalogAnnotation, MetadataEntry,
        };

        let annotation = || CatalogAnnotation {
            target: CatalogAnnotationTarget::Definition("effect:test:definition".to_owned()),
            metadata: vec![MetadataEntry {
                owner_component_id: None,
                key: "semantic-mode".to_owned(),
                value: "test".to_owned(),
            }],
        };
        let manifest = |annotations| ComponentManifest {
            component_id: "fixture.annotation-addon".to_owned(),
            component_version: "1.0.0".to_owned(),
            abi: WitAbiVersion {
                major: ABI_VERSION.major,
                minor: ABI_VERSION.minor,
            },
            capabilities: Vec::new(),
            subscriptions: Vec::new(),
            registered_syntax_handlers: Vec::new(),
            catalog_annotations: annotations,
            state_namespaces: Vec::new(),
        };

        let mut guest_owned = manifest(vec![annotation()]);
        guest_owned.catalog_annotations[0].metadata[0].owner_component_id =
            Some("spoofed.component".to_owned());
        assert!(matches!(
            validate_manifest(&guest_owned, &host_capabilities()),
            Err(HostError::InvalidManifest { .. })
        ));

        let mut unstamped = manifest(vec![annotation()]);
        validate_manifest(&unstamped, &host_capabilities())
            .expect("guest metadata without an owner is valid");
        stamp_catalog_annotation_owners(&mut unstamped);
        assert_eq!(
            unstamped.catalog_annotations[0].metadata[0]
                .owner_component_id
                .as_deref(),
            Some("fixture.annotation-addon")
        );
    }

    #[test]
    fn registered_handler_ids_must_be_nonempty_and_unique() {
        use crate::bindings::nlaocs::skript_parser_addon::types::{
            AbiVersion as WitAbiVersion, CapabilityRequirement, RegisteredSyntaxHandler,
        };

        let subscription = HookSubscription {
            id: "expression.parse".to_owned(),
            target: HookTarget::SyntaxKind(SyntaxKind::Expression),
            phase: HookPhase::Expression,
            priority: 0,
            mode: HookMode::Transform,
            capability_id: CAPABILITY_EXPRESSION_PARSER.to_owned(),
            selector: empty_selector(),
        };
        let handler = |handler_id: &str| RegisteredSyntaxHandler {
            handler_id: handler_id.to_owned(),
            kind: SyntaxKind::Expression,
            targets: vec![RegisteredSyntaxHandlerTarget::Definition(
                "expression:test:definition".to_owned(),
            )],
            pattern_indices: Vec::new(),
            pattern_sources: Vec::new(),
            required_tags: Vec::new(),
            forbidden_tags: Vec::new(),
            marks: Vec::new(),
            capture_parsers: Vec::new(),
            context_requirements: Vec::new(),
        };
        let manifest = |handlers| ComponentManifest {
            component_id: "fixture.handler-addon".to_owned(),
            component_version: "1.0.0".to_owned(),
            abi: WitAbiVersion {
                major: ABI_VERSION.major,
                minor: ABI_VERSION.minor,
            },
            capabilities: vec![CapabilityRequirement {
                id: CAPABILITY_EXPRESSION_PARSER.to_owned(),
                minimum_version: 1,
                required: true,
            }],
            subscriptions: vec![subscription.clone()],
            registered_syntax_handlers: handlers,
            catalog_annotations: Vec::new(),
            state_namespaces: Vec::new(),
        };

        let blank_error = validate_manifest(&manifest(vec![handler(" ")]), &host_capabilities())
            .expect_err("blank handler IDs must be rejected");
        assert!(
            blank_error
                .to_string()
                .contains("blank registered syntax handler ID")
        );

        let duplicate_error = validate_manifest(
            &manifest(vec![handler("fixture.handler"), handler("fixture.handler")]),
            &host_capabilities(),
        )
        .expect_err("duplicate handler IDs must be rejected");
        assert!(
            duplicate_error
                .to_string()
                .contains("registered syntax handler ID fixture.handler more than once")
        );

        let mut empty_targets = manifest(vec![handler("fixture.no-target")]);
        empty_targets.registered_syntax_handlers[0].targets.clear();
        let empty_target_error = validate_manifest(&empty_targets, &host_capabilities())
            .expect_err("registered handlers must declare at least one target");
        assert!(empty_target_error.to_string().contains("without a target"));

        let mut duplicate_targets = manifest(vec![handler("fixture.duplicate-target")]);
        duplicate_targets.registered_syntax_handlers[0].targets = vec![
            RegisteredSyntaxHandlerTarget::Definition("expression:test:definition".to_owned()),
            RegisteredSyntaxHandlerTarget::Definition("expression:test:definition".to_owned()),
        ];
        let duplicate_target_error = validate_manifest(&duplicate_targets, &host_capabilities())
            .expect_err("a handler must not repeat one target");
        assert!(
            duplicate_target_error
                .to_string()
                .contains("repeats registered syntax definition")
        );

        let mut dynamic_target = manifest(vec![handler("fixture.dynamic-target")]);
        dynamic_target.registered_syntax_handlers[0].targets =
            vec![RegisteredSyntaxHandlerTarget::DynamicHandler(
                "fixture.dynamic-expression".to_owned(),
            )];
        validate_manifest(&dynamic_target, &host_capabilities())
            .expect("dynamic handler targets use the normal parser subscription");

        let mut parser_class_target = manifest(vec![handler("fixture.parser-class")]);
        parser_class_target.registered_syntax_handlers[0].targets =
            vec![RegisteredSyntaxHandlerTarget::ParserClass(
                "fixture.Parser".to_owned(),
            )];
        let parser_class_error = validate_manifest(&parser_class_target, &host_capabilities())
            .expect_err("parser-class targets are Type-specific");
        assert!(parser_class_error.to_string().contains("non-Type syntax"));
    }

    #[test]
    fn native_metadata_round_trips_component_ownership() {
        let native = BTreeMap::from([
            ("host-key".to_owned(), "host-value".to_owned()),
            (
                "fixture.component/semantic-mode".to_owned(),
                "fixture".to_owned(),
            ),
        ]);

        let wire = metadata_to_wit(&native);
        assert!(wire.iter().any(|entry| {
            entry.owner_component_id.as_deref() == Some("fixture.component")
                && entry.key == "semantic-mode"
        }));
        assert_eq!(
            metadata_entries(wire).expect("qualified metadata must round trip"),
            native
        );
    }

    #[test]
    fn nested_parse_requests_can_override_event_and_addon_context() {
        let request = ParseRequest {
            request_id: 1,
            parser_id: skript_parser::HOST_EXPRESSION_PARSER_ID.to_owned(),
            input: "event-player".to_owned(),
            expected_types: Vec::new(),
            span: MappedSpan {
                virtual_range: WitTextRange { start: 0, end: 12 },
                origins: Vec::new(),
            },
            options: vec![
                WitMetadataEntry {
                    key: PARSE_CONTEXT_EVENT_CLASSES.to_owned(),
                    value: "fixture.FirstEvent; fixture.SecondEvent".to_owned(),
                    owner_component_id: None,
                },
                WitMetadataEntry {
                    key: format!("{PARSE_CONTEXT_VALUE_PREFIX}fixture.mode"),
                    value: "strict".to_owned(),
                    owner_component_id: None,
                },
            ],
        };
        let invocation = InvocationContext {
            invocation_id: 1,
            subscription_id: "fixture".to_owned(),
            document_id: "file:///fixture.sk".to_owned(),
            document_revision: 1,
            expansion: None,
            syntax_context: 9,
        };

        let context = parse_request_context(&request, &invocation).unwrap();
        assert_eq!(context.syntax_context, 9);
        assert_eq!(context.event_classes.len(), 2);
        assert_eq!(
            context.values.get("fixture.mode").map(String::as_str),
            Some("strict")
        );
    }

    #[test]
    fn nested_parse_requests_inherit_context_before_applying_overrides() {
        let parent = WitParseContext {
            syntax_context: 9,
            event_classes: vec!["fixture.ParentEvent".to_owned()],
            values: vec![WitParseContextValue {
                key: "fixture.mode".to_owned(),
                value: "parent".to_owned(),
            }],
        };
        let request = ParseRequest {
            request_id: 1,
            parser_id: skript_parser::HOST_EXPRESSION_PARSER_ID.to_owned(),
            input: "input".to_owned(),
            expected_types: Vec::new(),
            span: MappedSpan {
                virtual_range: WitTextRange { start: 0, end: 5 },
                origins: Vec::new(),
            },
            options: vec![WitMetadataEntry {
                key: format!("{PARSE_CONTEXT_VALUE_PREFIX}fixture.mode"),
                value: "child".to_owned(),
                owner_component_id: None,
            }],
        };
        let inherited = inherit_parse_request_context(request, Some(&parent));
        let invocation = InvocationContext {
            invocation_id: 1,
            subscription_id: "fixture".to_owned(),
            document_id: "file:///fixture.sk".to_owned(),
            document_revision: 1,
            expansion: None,
            syntax_context: 9,
        };

        let context = parse_request_context(&inherited, &invocation).unwrap();
        assert_eq!(context.event_classes[0].as_str(), "fixture.ParentEvent");
        assert_eq!(
            context.values.get("fixture.mode").map(String::as_str),
            Some("child")
        );
    }

    #[test]
    fn nested_parse_requests_select_a_root_expression_mode() {
        let mut request = ParseRequest {
            request_id: 1,
            parser_id: skript_parser::HOST_EXPRESSION_PARSER_ID.to_owned(),
            input: "value".to_owned(),
            expected_types: Vec::new(),
            span: MappedSpan {
                virtual_range: WitTextRange { start: 0, end: 5 },
                origins: Vec::new(),
            },
            options: vec![WitMetadataEntry {
                key: PARSE_MODE.to_owned(),
                value: "literals-only".to_owned(),
                owner_component_id: None,
            }],
        };
        assert_eq!(
            parse_request_root_mode(&request).unwrap(),
            ExpressionRootMode::LiteralsOnly
        );

        request.options[0].value = "unknown".to_owned();
        assert!(parse_request_root_mode(&request).is_err());
    }

    #[test]
    fn parse_result_attachments_are_stamped_with_the_provider_component() {
        let request = ParseRequest {
            request_id: 7,
            parser_id: "fixture.parser".to_owned(),
            input: "x".to_owned(),
            expected_types: Vec::new(),
            span: MappedSpan {
                virtual_range: WitTextRange { start: 0, end: 1 },
                origins: Vec::new(),
            },
            options: Vec::new(),
        };
        let result = ParseResult {
            host_token: 0,
            request_id: request.request_id,
            parser_id: request.parser_id.clone(),
            status: WitParseResultStatus::Partial,
            roots: vec![0],
            nodes: vec![ParseResultNode {
                node_id: 0,
                parser_id: request.parser_id.clone(),
                kind: "fixture".to_owned(),
                status: WitParseResultStatus::Partial,
                text: "x".to_owned(),
                span: request.span.clone(),
                expected_types: Vec::new(),
                summary: None,
                children: Vec::new(),
                attachments: vec![WitAddonAttachment {
                    owner_component_id: "guest-spoof".to_owned(),
                    schema_id: "fixture.schema".to_owned(),
                    schema_version: 1,
                    encoding: "raw".to_owned(),
                    bytes: vec![1, 2, 3],
                }],
                diagnostics: Vec::new(),
                metadata: Vec::new(),
            }],
            diagnostics: Vec::new(),
        };

        let mut effects = empty_effects();
        effects.parse_results.push(result);
        stamp_parse_result_attachments(&mut effects, "fixture.provider");
        let result = effects.parse_results.pop().expect("one parse result");
        validate_parse_result(&request, &result)
            .expect("the valid fixture result must pass validation");
        assert_eq!(
            result.nodes[0].attachments[0].owner_component_id,
            "fixture.provider"
        );
    }

    #[test]
    fn registered_capture_bindings_deduplicate_equal_bindings_and_reject_conflicts() {
        use crate::bindings::nlaocs::skript_parser_addon::types::{
            CaptureParserBinding, RegisteredSyntaxHandler,
        };

        const DEFINITION_ID: &str = "expression:fixture:definition";
        const REGISTRATION_ID: &str = "expression:fixture:registration";
        let syntax = RegisteredSyntaxIdentity {
            kind: CatalogSyntaxKind::Expression,
            definition_id: DEFINITION_ID,
            registration_id: REGISTRATION_ID,
            pattern_index: Some(0),
            pattern_source: Some("%objects%"),
            tags: None,
            mark: Some(0),
            dynamic_handler: None,
        };
        let binding = |parser_id: &str| CaptureParserBinding {
            capture_index: 0,
            parser_id: parser_id.to_owned(),
            required: true,
            options: Vec::new(),
        };
        let handler = |handler_id: &str, parser_id: &str| RegisteredSyntaxHandler {
            handler_id: handler_id.to_owned(),
            kind: SyntaxKind::Expression,
            targets: vec![RegisteredSyntaxHandlerTarget::Definition(
                DEFINITION_ID.to_owned(),
            )],
            pattern_indices: Vec::new(),
            pattern_sources: Vec::new(),
            required_tags: Vec::new(),
            forbidden_tags: Vec::new(),
            marks: Vec::new(),
            capture_parsers: vec![binding(parser_id)],
            context_requirements: Vec::new(),
        };
        let binding_record = |handler_id: &str| WitRegisteredHandlerBinding {
            handler_id: handler_id.to_owned(),
            definition_ids: vec![DEFINITION_ID.to_owned()],
            registration_ids: vec![REGISTRATION_ID.to_owned()],
        };

        let mut host = ParserHost::new(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../artifacts/core-library.wasm"
            )),
            core_test_config(),
        )
        .expect("core fixture must initialize");
        {
            let component = &mut host.components[0];
            component.manifest.registered_syntax_handlers = vec![
                handler("fixture.capture.first", "fixture.parser"),
                handler("fixture.capture.second", "fixture.parser"),
            ];
            component.registered_handler_bindings = vec![
                binding_record("fixture.capture.first"),
                binding_record("fixture.capture.second"),
            ];
        }

        let deduplicated = registered_capture_bindings(&host.components, syntax)
            .expect("equal capture bindings must compose");
        assert_eq!(deduplicated.len(), 1);
        assert_eq!(deduplicated[0].capture_index, 0);
        assert_eq!(deduplicated[0].parser_id, "fixture.parser");

        host.components[0].manifest.registered_syntax_handlers[1].capture_parsers[0] =
            binding("fixture.other-parser");
        let error = registered_capture_bindings(&host.components, syntax)
            .expect_err("different parsers for one capture must conflict");
        assert!(error.contains("conflicting capture parsers at index 0"));

        let dynamic_handler = handler("fixture.capture.dynamic", "fixture.dynamic-parser");
        let mut dynamic_syntax = syntax;
        dynamic_syntax.dynamic_handler = Some("fixture.dynamic-expression");
        let mut dynamic_handler = dynamic_handler;
        dynamic_handler.context_requirements = vec![REGISTERED_CONTEXT_ALL_TYPE_OPTIONS.to_owned()];
        host.components[0].manifest.registered_syntax_handlers = vec![RegisteredSyntaxHandler {
            targets: vec![RegisteredSyntaxHandlerTarget::DynamicHandler(
                "fixture.dynamic-expression".to_owned(),
            )],
            ..dynamic_handler
        }];
        host.components[0].registered_handler_bindings.clear();
        let dynamic_bindings = registered_capture_bindings(&host.components, dynamic_syntax)
            .expect("dynamic handlers do not need static catalog bindings");
        assert_eq!(dynamic_bindings.len(), 1);
        assert_eq!(dynamic_bindings[0].parser_id, "fixture.dynamic-parser");
        assert!(registered_handler_requires_context(
            &host.components,
            dynamic_syntax,
            REGISTERED_CONTEXT_ALL_TYPE_OPTIONS,
        ));

        dynamic_syntax.dynamic_handler = Some("fixture.other-expression");
        assert!(
            registered_capture_bindings(&host.components, dynamic_syntax)
                .expect("unmatched dynamic handlers are ignored")
                .is_empty()
        );
        assert!(!registered_handler_requires_context(
            &host.components,
            dynamic_syntax,
            REGISTERED_CONTEXT_ALL_TYPE_OPTIONS,
        ));
    }

    #[test]
    fn registration_handler_does_not_match_a_sibling_registration() {
        use crate::bindings::nlaocs::skript_parser_addon::types::RegisteredSyntaxHandler;

        let handler = RegisteredSyntaxHandler {
            handler_id: "fixture.registration".to_owned(),
            kind: SyntaxKind::Expression,
            targets: vec![RegisteredSyntaxHandlerTarget::Registration(
                "expression:fixture:first".to_owned(),
            )],
            pattern_indices: Vec::new(),
            pattern_sources: Vec::new(),
            required_tags: Vec::new(),
            forbidden_tags: Vec::new(),
            marks: Vec::new(),
            capture_parsers: Vec::new(),
            context_requirements: Vec::new(),
        };
        let mut host = ParserHost::new(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../artifacts/core-library.wasm"
            )),
            core_test_config(),
        )
        .expect("core fixture must initialize");
        let component = &mut host.components[0];
        component.manifest.registered_syntax_handlers = vec![handler.clone()];
        component.registered_handler_bindings = vec![WitRegisteredHandlerBinding {
            handler_id: handler.handler_id.clone(),
            definition_ids: vec!["expression:fixture:definition".to_owned()],
            registration_ids: vec!["expression:fixture:first".to_owned()],
        }];

        assert!(!registered_handler_matches(
            component,
            &handler,
            RegisteredSyntaxIdentity {
                kind: CatalogSyntaxKind::Expression,
                definition_id: "expression:fixture:definition",
                registration_id: "expression:fixture:second",
                pattern_index: Some(0),
                pattern_source: Some("%objects%"),
                tags: None,
                mark: Some(0),
                dynamic_handler: None,
            },
        ));

        let pattern_handler = RegisteredSyntaxHandler {
            pattern_indices: vec![3],
            pattern_sources: vec!["matched pattern".to_owned()],
            marks: vec![7],
            ..handler.clone()
        };
        let pattern_identity = RegisteredSyntaxIdentity {
            kind: CatalogSyntaxKind::Expression,
            definition_id: "expression:fixture:definition",
            registration_id: "expression:fixture:first",
            pattern_index: Some(3),
            pattern_source: Some("matched pattern"),
            tags: None,
            mark: Some(7),
            dynamic_handler: None,
        };
        assert!(registered_handler_matches(
            component,
            &pattern_handler,
            pattern_identity,
        ));
        assert!(!registered_handler_matches(
            component,
            &pattern_handler,
            RegisteredSyntaxIdentity {
                pattern_index: Some(4),
                ..pattern_identity
            },
        ));
        assert!(!registered_handler_matches(
            component,
            &pattern_handler,
            RegisteredSyntaxIdentity {
                pattern_source: Some("another pattern"),
                ..pattern_identity
            },
        ));
        assert!(!registered_handler_matches(
            component,
            &pattern_handler,
            RegisteredSyntaxIdentity {
                mark: Some(0),
                ..pattern_identity
            },
        ));

        let source = MappedSource::identity("");
        let tags = [skript_parser::ParseTagCapture {
            value: "parse".to_owned(),
            pattern_span: syntax_pattern_parser::syntax::Span::new(0, 0),
            input_span: MatchSpan {
                local_range: ParserTextRange::new(0, 0),
                mapped: source.map_range(ParserTextRange::new(0, 0)).unwrap(),
            },
            implicit: false,
        }];
        let tag_handler = RegisteredSyntaxHandler {
            required_tags: vec!["parse".to_owned()],
            ..handler.clone()
        };
        assert!(registered_handler_matches(
            component,
            &tag_handler,
            RegisteredSyntaxIdentity {
                tags: Some(&tags),
                ..pattern_identity
            },
        ));
        assert!(!registered_handler_matches(
            component,
            &tag_handler,
            RegisteredSyntaxIdentity {
                tags: None,
                ..pattern_identity
            },
        ));
        let forbidden_handler = RegisteredSyntaxHandler {
            forbidden_tags: vec!["parse".to_owned()],
            ..handler.clone()
        };
        assert!(!registered_handler_matches(
            component,
            &forbidden_handler,
            RegisteredSyntaxIdentity {
                tags: Some(&tags),
                ..pattern_identity
            },
        ));

        let dynamic = RegisteredSyntaxHandler {
            targets: vec![RegisteredSyntaxHandlerTarget::DynamicHandler(
                "fixture.dynamic-expression".to_owned(),
            )],
            ..handler
        };
        component.registered_handler_bindings.clear();
        assert!(registered_handler_matches(
            component,
            &dynamic,
            RegisteredSyntaxIdentity {
                kind: CatalogSyntaxKind::Expression,
                definition_id: "dynamic:test/expression",
                registration_id: "dynamic:test/expression",
                pattern_index: Some(0),
                pattern_source: Some("%objects%"),
                tags: None,
                mark: Some(0),
                dynamic_handler: Some("fixture.dynamic-expression"),
            },
        ));
        assert!(!registered_handler_matches(
            component,
            &dynamic,
            RegisteredSyntaxIdentity {
                kind: CatalogSyntaxKind::Expression,
                definition_id: "dynamic:test/expression",
                registration_id: "dynamic:test/expression",
                pattern_index: Some(0),
                pattern_source: Some("%objects%"),
                tags: None,
                mark: Some(0),
                dynamic_handler: Some("fixture.other-expression"),
            },
        ));
    }
}

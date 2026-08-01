//! Native Wasmtime host for CoreLibrary and parser addon Components.
//!
//! The host negotiates capabilities, orders hooks, enforces resource limits,
//! coordinates macros and dynamic syntax, and commits only accepted side effects.
#![allow(missing_docs)] // WIT transport fields are documented as aggregate contracts.

use std::{
    collections::{BTreeMap, BTreeSet},
    mem,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use skript_parser::{
    CandidateMatches, MatchInput, MatchSyntaxKind, PatternCandidate, PatternHookControl,
    PatternHookEvent, PatternHookOutcome, PatternHookScope, PatternHookTiming, PatternMatchError,
    PatternMatchHooks, PatternMatcherConfig, PatternPathSegment, TypeExpressionResolver,
    match_pattern_candidates as run_pattern_matcher,
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
    Catalog, DefinitionId, DynamicMultiplicity, DynamicRegistryError, DynamicSyntaxId,
    DynamicSyntaxInput, DynamicSyntaxOverrideInput, DynamicSyntaxRegistry, DynamicSyntaxSnapshot,
    DynamicSyntaxUpdate, RegistrationId, SyntaxKind as CatalogSyntaxKind, SyntaxOverrideTarget,
    SyntaxReference,
};
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Engine, ResourceLimiter, Store, Trap};

use crate::bindings::ParserAddon;
use crate::bindings::nlaocs::skript_parser_addon::dynamic_syntax_registry as wit_dynamic_registry;
use crate::bindings::nlaocs::skript_parser_addon::state_store as wit_state_store;
use crate::bindings::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity as WitDynamicMultiplicity, DynamicRegistryError as WitDynamicRegistryError,
    DynamicRegistryErrorKind as WitDynamicRegistryErrorKind,
    DynamicSyntaxDefinition as WitDynamicSyntaxDefinition,
    DynamicSyntaxOverride as WitDynamicSyntaxOverride,
    DynamicSyntaxOverrideTarget as WitDynamicSyntaxOverrideTarget,
    DynamicSyntaxReference as WitDynamicSyntaxReference,
    GeneratedRawNodeKind as WitGeneratedRawNodeKind, IndentKind as WitIndentKind,
    LineEnding as WitLineEnding, OriginKind as WitOriginKind,
    RawDiagnosticCode as WitRawDiagnosticCode, RawDiagnosticSeverity as WitRawDiagnosticSeverity,
    RawInvalidReason as WitRawInvalidReason, RawNodeKind as WitRawNodeKind,
    RawTriviaKind as WitRawTriviaKind, RetainedChildrenPlacement as WitRetainedChildrenPlacement,
    SourceOrigin as WitSourceOrigin, StateEncoding as WitStateEncoding,
    StateEntry as WitStateEntry, StateError as WitStateError, StateErrorKind as WitStateErrorKind,
    StateNamespaceVisibility as WitNamespaceVisibility, StateScope as WitStateScope,
    StateValue as WitStateValue, TextEdit as WitTextEdit, TextRange as WitTextRange,
    TreeEdit as WitTreeEdit,
};
use crate::state::{
    InvocationTransaction, NamespaceDeclaration, NamespaceVisibility, ParseTransaction,
    StateEncoding, StateError, StateReadWriteSet, StateSavepoint, StateScope, StateStore,
    StateStoreConfig, StateValue,
};
use crate::{
    ABI_VERSION, AbiVersion, CAPABILITY_ADDITIONAL_PARSE, CAPABILITY_CONTEXT_UPDATES,
    CAPABILITY_DYNAMIC_SYNTAX, CAPABILITY_HOOKS, CAPABILITY_STATE_STORE, CAPABILITY_TEXT_MACRO,
    CAPABILITY_TREE_MACRO, Capability, CapabilityRequirement, CompatibilityError,
    validate_compatibility,
};

pub use crate::bindings::nlaocs::skript_parser_addon::types::{
    AstNode, AstTree, Capture, CaptureValue, ComponentManifest, ContextUpdate, Diagnostic,
    DiagnosticSeverity, HookDecision, HookEffects, HookMode, HookOutput, HookPayload, HookPhase,
    HookSubscription, HookTarget, InvocationContext, MappedSpan, MatchingPathSegment,
    MatchingPayload, MatchingScope, MatchingStatus, MatchingTiming, ParseRequest, RawTree,
    RawTreeNode, Rejection, RelatedSpan, SyntaxKind, TextMacroInput, TextMacroOutput,
    TreeMacroInput, TreeMacroOutput,
};

/// Reserved component ID required for the first host component.
pub const CORE_LIBRARY_COMPONENT_ID: &str = "nlaocs.core-library";

#[derive(Debug, Clone)]
/// Execution, memory, pipeline, StateStore, and catalog configuration.
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
    pub max_text_macro_expansions: usize,
    pub max_text_macro_generated_bytes: usize,
    pub max_virtual_source_bytes: usize,
    pub max_raw_tree_depth: usize,
    pub max_tree_macro_expansion_depth: usize,
    pub max_tree_macro_nodes: usize,
    pub max_tree_macro_calls: usize,
    pub state_store: StateStoreConfig,
    pub syntax_catalog: Option<Arc<Catalog>>,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            fuel_per_call: 10_000_000,
            call_timeout: Duration::from_millis(100),
            epoch_tick: Duration::from_millis(10),
            max_memory_bytes: 64 * 1024 * 1024,
            max_table_elements: 100_000,
            max_instances_per_component: 32,
            max_tables_per_component: 32,
            max_memories_per_component: 32,
            max_calls_per_dispatch: 1_024,
            max_generated_output_bytes: 8 * 1024 * 1024,
            max_text_macro_expansions: 256,
            max_text_macro_generated_bytes: 8 * 1024 * 1024,
            max_virtual_source_bytes: 16 * 1024 * 1024,
            max_raw_tree_depth: 256,
            max_tree_macro_expansion_depth: 64,
            max_tree_macro_nodes: 100_000,
            max_tree_macro_calls: 4_096,
            state_store: StateStoreConfig::default(),
            syntax_catalog: None,
        }
    }
}

impl HostConfig {
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
            || self.max_text_macro_expansions == 0
            || self.max_text_macro_generated_bytes == 0
            || self.max_virtual_source_bytes == 0
            || self.max_raw_tree_depth == 0
            || self.max_tree_macro_expansion_depth == 0
            || self.max_tree_macro_nodes == 0
            || self.max_tree_macro_calls == 0;
        if invalid {
            Err(HostError::InvalidConfiguration)
        } else {
            Ok(())
        }
    }

    fn deadline_ticks(&self) -> u64 {
        let timeout = self.call_timeout.as_nanos();
        let tick = self.epoch_tick.as_nanos();
        timeout.div_ceil(tick).clamp(1, u64::MAX as u128) as u64
    }
}

#[derive(Debug, thiserror::Error)]
/// Host setup, component execution, output validation, or quota failure.
pub enum HostError {
    #[error("CoreLibrary component is missing")]
    CoreLibraryMissing,
    #[error("invalid parser host configuration: every quota and duration must be non-zero")]
    InvalidConfiguration,
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
    SyntaxDefinition(SyntaxKind),
    ExactRegistration {
        registration_id: String,
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

#[derive(Debug)]
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
}

#[derive(Debug)]
/// Native candidate results plus accepted matching-hook side effects.
pub struct WasmPatternMatchResult {
    pub matches: CandidateMatches,
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
}

impl crate::bindings::nlaocs::skript_parser_addon::types::Host for StoreData {}

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

struct ComponentEntry {
    manifest: ComponentManifest,
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
}

fn target_specificity(subscription: &HookTarget, requested: &DispatchTarget) -> Option<u8> {
    match (subscription, requested) {
        (HookTarget::ParseStage, DispatchTarget::ParseStage) => Some(0),
        (
            HookTarget::SyntaxDefinition(subscription_kind),
            DispatchTarget::SyntaxDefinition(requested_kind),
        ) if subscription_kind == requested_kind => Some(1),
        (
            HookTarget::ExactRegistration(subscription_id),
            DispatchTarget::ExactRegistration {
                registration_id, ..
            },
        ) if subscription_id == registration_id => Some(2),
        (
            HookTarget::SyntaxDefinition(subscription_kind),
            DispatchTarget::ExactRegistration { syntax_kind, .. },
        ) if subscription_kind == syntax_kind => Some(1),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
struct HookEffectsCheckpoint {
    diagnostics: usize,
    context_updates: usize,
    parse_requests: usize,
}

impl HookEffectsCheckpoint {
    const EMPTY: Self = Self {
        diagnostics: 0,
        context_updates: 0,
        parse_requests: 0,
    };

    fn capture(effects: &HookEffects) -> Self {
        Self {
            diagnostics: effects.diagnostics.len(),
            context_updates: effects.context_updates.len(),
            parse_requests: effects.parse_requests.len(),
        }
    }

    fn restore(self, effects: &mut HookEffects) {
        effects.diagnostics.truncate(self.diagnostics);
        effects.context_updates.truncate(self.context_updates);
        effects.parse_requests.truncate(self.parse_requests);
    }
}

struct WasmPatternHooks<'a> {
    host: &'a mut ParserHost,
    transaction: &'a ParseTransaction,
    context: InvocationContext,
    input: String,
    base: StateSavepoint,
    selected: Option<StateSavepoint>,
    selected_effects: Option<HookEffectsCheckpoint>,
    candidate_range: Option<ParserTextRange>,
    effects: HookEffects,
    calls: Vec<HookCall>,
    failures: Vec<ComponentFailure>,
}

impl WasmPatternHooks<'_> {
    fn restore_candidate_state(
        &mut self,
        scope: PatternHookScope,
        timing: PatternHookTiming,
        outcome: &PatternHookOutcome,
        control: &PatternHookControl,
    ) -> Result<(), String> {
        if scope != PatternHookScope::Definition {
            return Ok(());
        }
        if timing == PatternHookTiming::Before {
            return Ok(());
        }

        let accepted = match control {
            PatternHookControl::Continue => {
                matches!(outcome, PatternHookOutcome::Matched { .. })
            }
            PatternHookControl::Match(range) => Some(*range) == self.candidate_range,
            PatternHookControl::Fail(_) => false,
        };
        if accepted && self.selected.is_none() {
            self.selected = Some(
                self.transaction
                    .savepoint()
                    .map_err(|error| error.to_string())?,
            );
            self.selected_effects = Some(HookEffectsCheckpoint::capture(&self.effects));
            return Ok(());
        }

        self.transaction
            .rollback_to(self.selected.as_ref().unwrap_or(&self.base))
            .map_err(|error| error.to_string())?;
        self.selected_effects
            .unwrap_or(HookEffectsCheckpoint::EMPTY)
            .restore(&mut self.effects);
        Ok(())
    }

    fn into_parts(self) -> (HookEffects, Vec<HookCall>, Vec<ComponentFailure>) {
        (self.effects, self.calls, self.failures)
    }
}

impl PatternMatchHooks for WasmPatternHooks<'_> {
    fn dispatch(&mut self, event: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
        if event.scope == PatternHookScope::Definition && event.timing == PatternHookTiming::Before
        {
            self.transaction
                .rollback_to(&self.base)
                .map_err(|error| error.to_string())?;
            self.selected_effects
                .unwrap_or(HookEffectsCheckpoint::EMPTY)
                .restore(&mut self.effects);
            self.candidate_range = Some(event.input_range);
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
        };
        let target = if event.scope == PatternHookScope::Definition {
            DispatchTarget::SyntaxDefinition(wit_syntax_kind(event.kind))
        } else {
            DispatchTarget::ExactRegistration {
                registration_id: event.registration_id.to_owned(),
                syntax_kind: wit_syntax_kind(event.kind),
            }
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
            HookDecision::ContinueProcessing if changed => {
                matching_control(output.status, range, output.failure_reason)
            }
            HookDecision::ContinueProcessing => PatternHookControl::Continue,
        };
        self.restore_candidate_state(event.scope, event.timing, &event.outcome, &control)?;
        Ok(control)
    }
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

/// Wasmtime component registry and orchestrator for all parser extension stages.
pub struct ParserHost {
    engine: Engine,
    linker: Linker<StoreData>,
    config: HostConfig,
    state_store: StateStore,
    dynamic_syntax_registry: Option<DynamicSyntaxRegistry>,
    capabilities: Vec<Capability>,
    components: Vec<ComponentEntry>,
    registry: SubscriptionRegistry,
    _epoch_ticker: EpochTicker,
}

impl ParserHost {
    /// Creates a host and synchronously loads the mandatory CoreLibrary component.
    pub fn new(core_library: &[u8], config: HostConfig) -> Result<Self, HostError> {
        if core_library.is_empty() {
            return Err(HostError::CoreLibraryMissing);
        }
        config.validate()?;
        let state_store = StateStore::new(config.state_store.clone())?;
        let dynamic_syntax_registry = config
            .syntax_catalog
            .clone()
            .map(DynamicSyntaxRegistry::new);

        let mut wasmtime_config = Config::new();
        wasmtime_config.wasm_component_model(true);
        wasmtime_config.consume_fuel(true);
        wasmtime_config.epoch_interruption(true);
        let engine = Engine::new(&wasmtime_config).map_err(|error| HostError::Engine {
            message: error.to_string(),
        })?;
        let ticker = EpochTicker::start(engine.clone(), config.epoch_tick)?;
        let mut linker = Linker::new(&engine);
        ParserAddon::add_to_linker::<_, HasSelf<_>>(&mut linker, |data: &mut StoreData| data)
            .map_err(|error| HostError::Engine {
                message: format!("failed to register parser addon host imports: {error}"),
            })?;
        let capabilities = configured_host_capabilities(dynamic_syntax_registry.is_some());
        let mut host = Self {
            engine,
            linker,
            config,
            state_store,
            dynamic_syntax_registry,
            capabilities,
            components: Vec::new(),
            registry: SubscriptionRegistry::default(),
            _epoch_ticker: ticker,
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
        let mut hooks = WasmPatternHooks {
            host: self,
            transaction,
            context,
            input: input_text,
            base: base.clone(),
            selected: None,
            selected_effects: None,
            candidate_range: None,
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
    pub fn dispatch(&mut self, request: DispatchRequest) -> Result<DispatchResult, HostError> {
        let project_uri = request.context.document_id.clone();
        let transaction = self.begin_parse(
            &project_uri,
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
    pub fn expand_text(&mut self, request: TextMacroRequest) -> Result<TextMacroResult, HostError> {
        let project_uri = request.context.document_id.clone();
        let transaction = self.begin_parse(
            &project_uri,
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
                        self.config.deadline_ticks(),
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
                effects: macro_effects,
            } = output;
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
    pub fn expand_tree(&mut self, request: TreeMacroRequest) -> Result<TreeMacroResult, HostError> {
        let project_uri = request.context.document_id.clone();
        let transaction = self.begin_parse(
            &project_uri,
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
                        self.config.deadline_ticks(),
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
                effects,
            } = output;
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
    fn dispatch_with_transaction(
        &mut self,
        transaction: &ParseTransaction,
        request: DispatchRequest,
    ) -> Result<DispatchResult, HostError> {
        let candidates =
            self.registry
                .matching_capability(&request.target, request.phase, CAPABILITY_HOOKS);
        let document_id = transaction.document_id()?;
        let document_revision = transaction.document_revision()?;
        let mut payload = request.payload;
        let mut effects = empty_effects();
        let mut calls = Vec::new();
        let mut failures = Vec::new();
        let mut generated_output = 0usize;
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
            let mut context = request.context.clone();
            context.subscription_id = subscription_id.clone();
            let invocation = crate::bindings::nlaocs::skript_parser_addon::types::HookInvocation {
                context,
                target: candidate.subscription.target.clone(),
                phase: request.phase,
                payload: payload.clone(),
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
                    let prepared = prepare_store(
                        &mut entry.store,
                        self.config.fuel_per_call,
                        self.config.deadline_ticks(),
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
            let output = match call {
                Ok(Ok(output)) => output,
                Ok(Err(addon_error)) => {
                    state_invocation.rollback();
                    drop(dynamic_update);
                    effects.diagnostics.extend(addon_error.diagnostics);
                    failures.push(ComponentFailure {
                        component_id: component_id.clone(),
                        subscription_id: subscription_id.clone(),
                        error: HostError::AddonFailure {
                            component_id,
                            message: addon_error.message,
                        },
                    });
                    continue;
                }
                Err(error) => {
                    state_invocation.rollback();
                    drop(dynamic_update);
                    let error = classify_wasmtime_error(component_id.clone(), "hook", error);
                    if error.disables_component() {
                        self.components[candidate.component_index].disabled = true;
                        if let Some(registry) = &self.dynamic_syntax_registry {
                            registry.remove_component(&component_id)?;
                        }
                    }
                    failures.push(ComponentFailure {
                        component_id,
                        subscription_id,
                        error,
                    });
                    continue;
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

            match apply_hook_output(candidate.subscription.mode, output, payload.clone()) {
                Ok(applied) => {
                    if matches!(applied.decision, Some(HookDecision::Reject(_))) {
                        state_invocation.rollback();
                        drop(dynamic_update);
                    } else {
                        state_invocation.commit()?;
                        if let Some(update) = dynamic_update {
                            update.commit()?;
                        }
                    }
                    payload = applied.payload;
                    merge_effects(&mut effects, applied.effects);
                    if let Some(final_decision) = applied.decision {
                        decision = final_decision;
                    }
                    if applied.terminal {
                        break;
                    }
                }
                Err(message) => {
                    state_invocation.rollback();
                    drop(dynamic_update);
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
        let component = Component::new(&self.engine, bytes)
            .map_err(|error| classify_component_error(loading_id.to_owned(), "compile", error))?;
        let mut store = create_store(&self.engine, &self.config);
        prepare_store(
            &mut store,
            self.config.fuel_per_call,
            self.config.deadline_ticks(),
            loading_id,
            "instantiate",
        )?;
        let bindings =
            ParserAddon::instantiate(&mut store, &component, &self.linker).map_err(|error| {
                classify_component_error(loading_id.to_owned(), "instantiate", error)
            })?;

        prepare_store(
            &mut store,
            self.config.fuel_per_call,
            self.config.deadline_ticks(),
            loading_id,
            "manifest",
        )?;
        let manifest = bindings
            .nlaocs_skript_parser_addon_addon()
            .call_manifest(&mut store)
            .map_err(|error| classify_wasmtime_error(loading_id.to_owned(), "manifest", error))?;
        validate_manifest(&manifest, &self.capabilities)?;

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
            self.config.deadline_ticks(),
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
        let profile = host_profile(&self.capabilities);
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
    ]
    .map(|id| Capability::new(id, 1))
    .to_vec()
}

fn configured_host_capabilities(dynamic_syntax_available: bool) -> Vec<Capability> {
    let mut capabilities = host_capabilities();
    if dynamic_syntax_available {
        capabilities.push(Capability::new(CAPABILITY_DYNAMIC_SYNTAX, 1));
    }
    capabilities
}

fn host_profile(
    capabilities: &[Capability],
) -> crate::bindings::nlaocs::skript_parser_addon::types::HostProfile {
    use crate::bindings::nlaocs::skript_parser_addon::types::{
        AbiVersion as WitAbiVersion, Capability as WitCapability, HostProfile,
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
    }
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
                || !matches!(subscription.mode, HookMode::Transform))
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
                || !matches!(subscription.mode, HookMode::Transform))
        {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "tree macro subscription {} must target parse-stage in the tree phase with transform mode",
                    subscription.id
                ),
            });
        }
        if let HookTarget::ExactRegistration(registration_id) = &subscription.target
            && registration_id.trim().is_empty()
        {
            return Err(HostError::InvalidManifest {
                message: format!(
                    "subscription {} has a blank exact registration ID",
                    subscription.id
                ),
            });
        }
    }
    Ok(())
}

fn create_store(engine: &Engine, config: &HostConfig) -> Store<StoreData> {
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
            message: error.to_string(),
        }
    } else {
        HostError::ComponentInstantiation {
            component_id,
            message: error.to_string(),
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
            HookDecision::ContinueProcessing | HookDecision::Handled => 0,
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

fn empty_effects() -> HookEffects {
    HookEffects {
        diagnostics: Vec::new(),
        context_updates: Vec::new(),
        parse_requests: Vec::new(),
    }
}

fn merge_effects(target: &mut HookEffects, source: HookEffects) {
    target.diagnostics.extend(source.diagnostics);
    target.context_updates.extend(source.context_updates);
    target.parse_requests.extend(source.parse_requests);
}

fn hook_output_size(output: &HookOutput) -> usize {
    output
        .replacement
        .as_ref()
        .map_or(0, hook_payload_size)
        .saturating_add(hook_effects_size(&output.effects))
        .saturating_add(match &output.decision {
            HookDecision::Reject(rejection) => rejection_size(rejection),
            HookDecision::ContinueProcessing | HookDecision::Handled => 0,
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
            HookDecision::ContinueProcessing | HookDecision::Handled => 0,
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
            .saturating_add(value.pattern.as_ref().map_or(0, String::len)),
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
    }
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
    request.input.len().saturating_add(
        request
            .expected_types
            .iter()
            .map(String::len)
            .fold(0usize, usize::saturating_add),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::nlaocs::skript_parser_addon::types::DocumentPayload;
    use wasm_encoder::{
        BlockType, CodeSection, ExportKind, ExportSection, Function, FunctionSection, Instruction,
        MemorySection, MemoryType, Module as EncodedModule, TypeSection, ValType,
    };
    use wasmtime::{Instance, Module as WasmtimeModule};

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
        }
    }

    fn document(text: &str) -> HookPayload {
        HookPayload::Document(DocumentPayload {
            text: text.to_owned(),
        })
    }

    fn output(decision: HookDecision, replacement: Option<HookPayload>) -> HookOutput {
        HookOutput {
            decision,
            replacement,
            effects: empty_effects(),
        }
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
                    HookTarget::SyntaxDefinition(SyntaxKind::Expression),
                    -100,
                    HookMode::Observe,
                ),
                subscription(
                    "exact-first-component",
                    HookTarget::ExactRegistration("expr.test".to_owned()),
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
                    HookTarget::ExactRegistration("expr.test".to_owned()),
                    -10,
                    HookMode::Observe,
                ),
                subscription(
                    "exact-same-priority-later-load",
                    HookTarget::ExactRegistration("expr.test".to_owned()),
                    0,
                    HookMode::Observe,
                ),
            ],
        );

        let matched = registry.matching(
            &DispatchTarget::ExactRegistration {
                registration_id: "expr.test".to_owned(),
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
                "exact-higher-priority",
                "exact-first-component",
                "exact-same-priority-later-load",
                "syntax-first-component",
            ]
        );
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
            ]
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
            state_namespaces: Vec::new(),
        };

        validate_manifest(&manifest(subscription.clone()), &host_capabilities())
            .expect("the dedicated Text macro pipeline shape must be accepted");

        let mut invalid_target = subscription.clone();
        invalid_target.target = HookTarget::SyntaxDefinition(SyntaxKind::Expression);
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
            state_namespaces: Vec::new(),
        };

        validate_manifest(&manifest(subscription.clone()), &host_capabilities())
            .expect("the dedicated Tree macro pipeline shape must be accepted");

        let mut invalid_target = subscription.clone();
        invalid_target.target = HookTarget::SyntaxDefinition(SyntaxKind::Section);
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
        let mut store = create_store(&engine, &config);
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
}

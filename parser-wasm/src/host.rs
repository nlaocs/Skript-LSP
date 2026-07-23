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

use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, ResourceLimiter, Store, Trap};

use crate::bindings::ParserAddon;
use crate::{
    ABI_VERSION, AbiVersion, CAPABILITY_ADDITIONAL_PARSE, CAPABILITY_CONTEXT_UPDATES,
    CAPABILITY_HOOKS, Capability, CapabilityRequirement, CompatibilityError,
    validate_compatibility,
};

pub use crate::bindings::nlaocs::skript_parser_addon::types::{
    AstNode, AstTree, Capture, CaptureValue, ComponentManifest, ContextUpdate, Diagnostic,
    HookDecision, HookEffects, HookMode, HookOutput, HookPayload, HookPhase, HookSubscription,
    HookTarget, InvocationContext, MappedSpan, ParseRequest, RawTree, RawTreeNode, Rejection,
    SyntaxKind,
};

pub const CORE_LIBRARY_COMPONENT_ID: &str = "nlaocs.core-library";

#[derive(Debug, Clone)]
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
            || self.max_generated_output_bytes == 0;
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
pub struct ComponentInfo {
    pub component_id: String,
    pub component_version: String,
    pub load_order: usize,
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchTarget {
    ParseStage,
    SyntaxDefinition(SyntaxKind),
    ExactRegistration {
        registration_id: String,
        syntax_kind: SyntaxKind,
    },
}

pub struct DispatchRequest {
    pub context: InvocationContext,
    pub target: DispatchTarget,
    pub phase: HookPhase,
    pub payload: HookPayload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookCall {
    pub component_id: String,
    pub subscription_id: String,
}

#[derive(Debug)]
pub struct ComponentFailure {
    pub component_id: String,
    pub subscription_id: String,
    pub error: HostError,
}

#[derive(Debug)]
pub struct DispatchResult {
    pub decision: HookDecision,
    pub payload: HookPayload,
    pub effects: HookEffects,
    pub calls: Vec<HookCall>,
    pub failures: Vec<ComponentFailure>,
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
}

struct ComponentEntry {
    manifest: ComponentManifest,
    store: Store<StoreData>,
    bindings: ParserAddon,
    load_order: usize,
    disabled: bool,
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

pub struct ParserHost {
    engine: Engine,
    linker: Linker<StoreData>,
    config: HostConfig,
    capabilities: Vec<Capability>,
    components: Vec<ComponentEntry>,
    registry: SubscriptionRegistry,
    _epoch_ticker: EpochTicker,
}

impl ParserHost {
    pub fn new(core_library: &[u8], config: HostConfig) -> Result<Self, HostError> {
        if core_library.is_empty() {
            return Err(HostError::CoreLibraryMissing);
        }
        config.validate()?;

        let mut wasmtime_config = Config::new();
        wasmtime_config.wasm_component_model(true);
        wasmtime_config.consume_fuel(true);
        wasmtime_config.epoch_interruption(true);
        let engine = Engine::new(&wasmtime_config).map_err(|error| HostError::Engine {
            message: error.to_string(),
        })?;
        let ticker = EpochTicker::start(engine.clone(), config.epoch_tick)?;
        let linker = Linker::new(&engine);
        let capabilities = host_capabilities();
        let mut host = Self {
            engine,
            linker,
            config,
            capabilities,
            components: Vec::new(),
            registry: SubscriptionRegistry::default(),
            _epoch_ticker: ticker,
        };
        host.load_component(core_library, true)?;
        Ok(host)
    }

    pub fn load_addon(&mut self, component: &[u8]) -> Result<ComponentInfo, HostError> {
        self.load_component(component, false)
    }

    pub fn components(&self) -> Vec<ComponentInfo> {
        self.components
            .iter()
            .map(|entry| ComponentInfo {
                component_id: entry.manifest.component_id.clone(),
                component_version: entry.manifest.component_version.clone(),
                load_order: entry.load_order,
                disabled: entry.disabled,
            })
            .collect()
    }

    pub fn dispatch(&mut self, request: DispatchRequest) -> Result<DispatchResult, HostError> {
        let candidates = self.registry.matching(&request.target, request.phase);
        let mut payload = request.payload;
        let mut effects = empty_effects();
        let mut calls = Vec::new();
        let mut failures = Vec::new();
        let mut generated_output = 0usize;
        let mut decision = HookDecision::ContinueProcessing;

        for candidate in candidates {
            if self.components[candidate.component_index].disabled {
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
            let call = {
                let entry = &mut self.components[candidate.component_index];
                prepare_store(
                    &mut entry.store,
                    self.config.fuel_per_call,
                    self.config.deadline_ticks(),
                    &component_id,
                    "hook",
                )?;
                entry
                    .bindings
                    .nlaocs_skript_parser_addon_hooks()
                    .call_invoke(&mut entry.store, &invocation)
            };

            let output = match call {
                Ok(Ok(output)) => output,
                Ok(Err(addon_error)) => {
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
                    let error = classify_wasmtime_error(component_id.clone(), "hook", error);
                    if error.disables_component() {
                        self.components[candidate.component_index].disabled = true;
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
                return Err(HostError::GeneratedOutputQuotaExceeded {
                    limit: self.config.max_generated_output_bytes,
                });
            }

            match apply_hook_output(candidate.subscription.mode, output, payload.clone()) {
                Ok(applied) => {
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
        let profile = host_profile(&self.capabilities);
        match bindings
            .nlaocs_skript_parser_addon_addon()
            .call_initialize(&mut store, &profile)
            .map_err(|error| {
                classify_wasmtime_error(manifest.component_id.clone(), "initialize", error)
            })? {
            Ok(()) => {}
            Err(error) => {
                return Err(HostError::InitializationRejected {
                    component_id: manifest.component_id,
                    message: error.message,
                });
            }
        }

        let component_index = self.components.len();
        let load_order = component_index;
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
        });
        Ok(info)
    }
}

pub fn host_capabilities() -> Vec<Capability> {
    [
        CAPABILITY_HOOKS,
        CAPABILITY_CONTEXT_UPDATES,
        CAPABILITY_ADDITIONAL_PARSE,
    ]
    .map(|id| Capability::new(id, 1))
    .to_vec()
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

fn hook_payload_size(payload: &HookPayload) -> usize {
    match payload {
        HookPayload::Document(value) => value.text.len(),
        HookPayload::Preprocess(value) => value.text.len(),
        HookPayload::Line(value) => value.text.len(),
        HookPayload::Tree(value) => raw_tree_size(value),
        HookPayload::Node(value) => raw_node_size(&value.node),
        HookPayload::Matching(value) => value.input.len().saturating_add(value.pattern.len()),
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
                CAPABILITY_CONTEXT_UPDATES,
                CAPABILITY_ADDITIONAL_PARSE,
            ]
        );
        assert!(!ids.contains(&crate::CAPABILITY_TEXT_MACRO.to_owned()));
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

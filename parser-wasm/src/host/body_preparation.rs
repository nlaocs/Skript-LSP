//! Isolated, optional preparation of semantic invocations. Workers never publish
//! an AST, declarations, state deltas, receipts, or completion notifications.
//! The authoritative parser still performs the complete ordered pipeline. A
//! finished worker capsule only lends opt-in read-only invocation responses to
//! its ordinary current-payload/read-certificate/output validation path.
use super::*;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;
mod trace;
use trace::TraceToken;
pub(super) use trace::{BodyPhaseClock, EntryUse};
pub use trace::{BodyPhaseMetrics, BodyPreparationTrace, BodyWorkerTrace};

/// Counts preparation work separately from authoritative document analysis.
#[derive(Clone, Debug, Default)]
pub struct BodyPreparationMetrics {
    pub spawned: usize,
    pub completed: usize,
    pub failed: usize,
    pub entries: usize,
    pub bytes: usize,
    pub join_ms: f64,
    pub reason: &'static str,
    pub trace: Option<BodyPreparationTrace>,
}

// This private, non-serializable seed is the cross-host seal. It owns the actual
// Catalog/Engine and exact compiled Component objects, effective configuration,
// source/revision/imports and ordered manifest/binding identities. Capsules can
// only be produced from this seed and are accepted by pointer identity in this
// single attempt. No assumed manifest-version or reused numeric revision seal.
#[derive(Clone)]
struct Seed {
    runtime: Arc<SharedHostRuntime>,
    config: HostConfig,
    matchers: Arc<[TypeUserInputMatcher]>,
    components: Vec<(
        Component,
        ComponentManifest,
        Vec<WitRegisteredHandlerBinding>,
    )>,
    request: DocumentParseRequest,
    document: DocumentParserConfig,
    trace_origin: Option<Instant>,
}

impl Seed {
    fn instantiate(
        &self,
        cancellation: DocumentCancellationToken,
    ) -> Result<ParserHost, HostError> {
        let state_store = StateStore::new(self.config.state_store.clone())?;
        state_store.restrict_to_parse_scopes()?;
        let mut host = ParserHost {
            completion_types: CompletionTypeCertificate(Arc::new(AtomicBool::new(true))),
            body_preparation: None,
            body_partition: None,
            isolated_preparation: true,
            body_preparation_metrics: BodyPreparationMetrics::default(),
            body_phase_clock: BodyPhaseClock::default(),
            runtime: Arc::clone(&self.runtime),
            config: self.config.clone(),
            native_cancellation: Some(cancellation.clone()),
            native_function_namespace_complete: true,
            native_dependency_unavailable: None,
            semantic_receipts: None,
            expression_preparation: Default::default(),
            incremental: Default::default(),
            state_store,
            dynamic_syntax_registry: self
                .config
                .syntax_catalog
                .clone()
                .map(DynamicSyntaxRegistry::new),
            capabilities: configured_host_capabilities(
                self.config.syntax_catalog.is_some(),
                self.config
                    .syntax_catalog
                    .as_ref()
                    .is_some_and(|c| c.source().is_some()),
            ),
            components: Vec::new(),
            registry: Default::default(),
            type_user_input_matchers: Arc::clone(&self.matchers),
            active_parse_requests: Vec::new(),
            next_parse_result_token: 1,
        };
        for (index, (component, manifest, bindings)) in self.components.iter().enumerate() {
            if cancellation.is_cancelled() {
                return Err(skript_parser::ParseCancelled.into());
            }
            host.load_compiled_component(component.clone(), index == 0)?;
            let actual = &host.components[index];
            validate_restart_identity(
                manifest,
                &actual.manifest,
                bindings,
                &actual.registered_handler_bindings,
            )?;
        }
        Ok(host)
    }
}

struct Capsule {
    seed: Arc<Seed>,
    partition: usize,
    cache: Option<incremental::DocumentCache>,
    trace: Option<BodyWorkerTrace>,
}

pub(super) struct RunningPreparation {
    seed: Arc<Seed>,
    receiver: Receiver<Capsule>,
    cancellation: Vec<DocumentCancellationToken>,
    handles: Vec<JoinHandle<()>>,
    caches: BTreeMap<usize, PreparedCache>,
    received: usize,
    metrics: BodyPreparationMetrics,
}

struct PreparedCache {
    cache: incremental::DocumentCache,
    uses: Option<Vec<EntryUse>>,
}

impl RunningPreparation {
    fn accept(&mut self, mut capsule: Capsule) {
        if !Arc::ptr_eq(&capsule.seed, &self.seed) {
            if let Some(trace) = &mut self.metrics.trace {
                trace.foreign_capsules += 1;
            }
            return;
        }
        self.received += 1;
        let received_at_ms = self.offset_ms();
        if self.seed.document.cancellation.is_cancelled() {
            if let Some(trace) = &mut self.metrics.trace {
                trace.cancelled_capsules += 1;
                if let Some(mut worker) = capsule.trace {
                    worker.outcome = "discarded-main-cancelled";
                    worker.received_at_ms = received_at_ms;
                    trace.workers.push(worker);
                }
            }
            return;
        }
        if let Some(worker) = &mut capsule.trace {
            worker.received_at_ms = received_at_ms;
        }
        if let Some(mut cache) = capsule.cache {
            self.metrics.completed += 1;
            self.metrics.entries += cache.entries.len();
            self.metrics.bytes += cache
                .entries
                .values()
                .map(|entry| entry.bytes)
                .sum::<usize>();
            let uses = self.metrics.trace.as_ref().map(|_| {
                for (slot, entry) in cache.entries.values_mut().enumerate() {
                    entry.trace_slot = slot;
                    if let (Some(worker), Some(at)) = (&mut capsule.trace, entry.recorded_at_ms) {
                        worker.first_recorded_at_ms =
                            Some(worker.first_recorded_at_ms.map_or(at, |old| old.min(at)));
                        worker.last_recorded_at_ms =
                            Some(worker.last_recorded_at_ms.map_or(at, |old| old.max(at)));
                    }
                }
                vec![EntryUse::Unqueried; cache.entries.len()]
            });
            self.caches
                .insert(capsule.partition, PreparedCache { cache, uses });
        } else {
            self.metrics.failed += 1;
        }
        if let (Some(trace), Some(worker)) = (&mut self.metrics.trace, capsule.trace) {
            trace.workers.push(worker);
        }
    }

    fn offset_ms(&self) -> Option<f64> {
        self.seed
            .trace_origin
            .map(|origin| origin.elapsed().as_secs_f64() * 1000.0)
    }
    fn poll(&mut self) {
        while let Ok(capsule) = self.receiver.try_recv() {
            self.accept(capsule);
        }
    }
    pub(super) fn lookup(
        &mut self,
        world: &str,
        key: &str,
    ) -> Option<(&incremental::CachedInvocation, Option<TraceToken>)> {
        if let Some(trace) = &mut self.metrics.trace {
            trace.lookup_calls += 1;
            if self.received < self.metrics.spawned {
                trace.lookups_while_pending += 1;
            }
        }
        if self.seed.document.cancellation.is_cancelled() {
            if let Some(trace) = &mut self.metrics.trace {
                trace.cancelled_lookups += 1;
            }
            return None;
        }
        let mut matching_world = false;
        for (partition, prepared) in &self.caches {
            if prepared.cache.world != world {
                continue;
            }
            matching_world = true;
            if let Some(entry) = prepared.cache.entries.get(key) {
                let token = prepared.uses.as_ref().map(|_| TraceToken {
                    partition: *partition,
                    slot: entry.trace_slot,
                    recorded_at_ms: entry.recorded_at_ms,
                });
                return Some((entry, token));
            }
        }
        if let Some(trace) = &mut self.metrics.trace {
            if self.caches.is_empty() {
                trace.no_ready_cache += 1;
            } else if !matching_world {
                trace.no_matching_world += 1;
            } else {
                trace.key_not_in_ready_caches += 1;
            }
        }
        None
    }
    pub(super) fn record_use(&mut self, token: Option<TraceToken>, outcome: EntryUse) {
        let Some(token) = token else {
            return;
        };
        if let Some(state) = self
            .caches
            .get_mut(&token.partition)
            .and_then(|cache| cache.uses.as_mut())
            .and_then(|uses| uses.get_mut(token.slot))
        {
            if *state != EntryUse::Consumed {
                *state = outcome;
            }
        }
        let now = self.offset_ms();
        let Some(worker) = self.metrics.trace.as_mut().and_then(|trace| {
            trace
                .workers
                .iter_mut()
                .find(|worker| worker.partition == token.partition)
        }) else {
            return;
        };
        match outcome {
            EntryUse::Consumed => {
                worker.hit_calls += 1;
                if let Some(now) = now {
                    worker.first_consumed_at_ms.get_or_insert(now);
                    worker.last_consumed_at_ms = Some(now);
                    if let Some(at) = token.recorded_at_ms {
                        let age = (now - at).max(0.0);
                        worker.max_recorded_to_consumed_ms = Some(
                            worker
                                .max_recorded_to_consumed_ms
                                .map_or(age, |old| old.max(age)),
                        );
                    }
                    if let Some(at) = worker.finished_at_ms {
                        let age = (now - at).max(0.0);
                        worker.max_ready_to_consumed_ms = Some(
                            worker
                                .max_ready_to_consumed_ms
                                .map_or(age, |old| old.max(age)),
                        );
                    }
                }
            }
            EntryUse::ReceiptRejected => worker.receipt_rejections += 1,
            EntryUse::HeaderRejected => worker.header_rejections += 1,
            EntryUse::StateRejected => worker.state_rejections += 1,
            EntryUse::StateError => worker.state_errors += 1,
            EntryUse::Unqueried => {}
        }
    }
    fn summarize_uses(&mut self) {
        let Some(trace) = &mut self.metrics.trace else {
            return;
        };
        for (partition, prepared) in &self.caches {
            let Some(uses) = &prepared.uses else {
                continue;
            };
            let Some(worker) = trace
                .workers
                .iter_mut()
                .find(|worker| worker.partition == *partition)
            else {
                continue;
            };
            for state in uses {
                match state {
                    EntryUse::Unqueried => worker.unqueried_after_receipt += 1,
                    EntryUse::Consumed => worker.unique_consumed_entries += 1,
                    EntryUse::ReceiptRejected => worker.unused_receipt_rejected += 1,
                    EntryUse::HeaderRejected => worker.unused_header_rejected += 1,
                    EntryUse::StateRejected => worker.unused_state_rejected += 1,
                    EntryUse::StateError => worker.unused_state_error += 1,
                }
            }
        }
        trace.workers.sort_by_key(|worker| worker.partition);
    }
    fn finish(mut self) -> BodyPreparationMetrics {
        let started = Instant::now();
        self.poll();
        for token in &self.cancellation {
            token.cancel();
        }
        let wait = self.metrics.trace.as_ref().map(|_| Instant::now());
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
        if let (Some(trace), Some(wait)) = (&mut self.metrics.trace, wait) {
            trace.helper_join_wait_ms = wait.elapsed().as_secs_f64() * 1000.0;
        }
        self.poll();
        self.metrics.join_ms = started.elapsed().as_secs_f64() * 1000.0;
        self.summarize_uses();
        self.metrics.clone()
    }
}

impl Drop for RunningPreparation {
    fn drop(&mut self) {
        for token in &self.cancellation {
            token.cancel();
        }
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
    }
}

fn prepare(
    seed: Arc<Seed>,
    partition: usize,
    partitions: usize,
    token: DocumentCancellationToken,
    sender: Sender<Capsule>,
) {
    let mut trace = seed.trace_origin.map(|origin| BodyWorkerTrace {
        partition,
        outcome: "initializing",
        started_at_ms: Some(origin.elapsed().as_secs_f64() * 1000.0),
        ..Default::default()
    });
    let cache = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let initialized = trace.as_ref().map(|_| Instant::now());
        let host = seed.instantiate(token.clone());
        if let (Some(trace), Some(started)) = (&mut trace, initialized) {
            trace.initialization_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
        }
        let mut host = match host {
            Ok(host) => host,
            Err(_) => {
                if let Some(trace) = &mut trace {
                    trace.outcome = if token.is_cancelled() {
                        "cancelled-initialization"
                    } else {
                        "initialization-error"
                    };
                }
                return None;
            }
        };
        host.body_partition = Some((partition, partitions));
        host.body_phase_clock.align_next_document(seed.trace_origin);
        let mut config = seed.document.clone();
        config.body_preparation_workers = 1;
        config.cancellation = token.clone();
        let result = host.parse_document(seed.request.clone(), config);
        if let Some(trace) = &mut trace {
            trace.phase = host
                .body_preparation_metrics
                .trace
                .as_ref()
                .map(|trace| trace.main.clone());
        }
        let result = match result {
            Ok(result) => result,
            Err(_) => {
                if let Some(trace) = &mut trace {
                    trace.outcome = if token.is_cancelled() {
                        "cancelled-parse"
                    } else {
                        "parse-error"
                    };
                }
                return None;
            }
        };
        let outcome = if token.is_cancelled() {
            "cancelled-parse"
        } else if !result.component_failures.is_empty() || !result.fault_regions.is_empty() {
            "component-fault"
        } else if !result.dependency_unavailable.is_empty() {
            "dependency-unavailable"
        } else if result.execution_status != DocumentExecutionStatus::Complete {
            "incomplete"
        } else {
            "ready"
        };
        if let Some(trace) = &mut trace {
            trace.outcome = outcome;
        }
        if outcome != "ready" {
            return None;
        }
        // A recorded response is not available to the main host until all the
        // ordinary validation and private worker commit above have completed.
        let cache = host
            .incremental
            .documents
            .remove(&seed.request.context.document_id);
        if cache.is_none()
            && let Some(trace) = &mut trace
        {
            trace.outcome = "no-cache";
        }
        cache
    }));
    let cache = match cache {
        Ok(cache) => cache,
        Err(_) => {
            if let Some(trace) = &mut trace {
                trace.outcome = "panic";
            }
            None
        }
    };
    if let (Some(trace), Some(origin)) = (&mut trace, seed.trace_origin) {
        trace.finished_at_ms = Some(origin.elapsed().as_secs_f64() * 1000.0);
    }
    // Timing travels on the existing one-capsule-per-worker channel. No shared
    // lock, per-invocation channel traffic, source string or error text is added.
    let _ = sender.send(Capsule {
        seed,
        partition,
        cache,
        trace,
    });
}

impl ParserHost {
    pub fn body_preparation_metrics(&self) -> &BodyPreparationMetrics {
        &self.body_preparation_metrics
    }

    pub(super) fn start_body_preparation(
        &mut self,
        request: &DocumentParseRequest,
        config: &DocumentParserConfig,
    ) {
        if self.body_partition.is_some() || self.body_preparation.is_some() {
            return;
        }
        self.body_preparation_metrics = BodyPreparationMetrics::default();
        let budget = config.body_preparation_workers;
        if !(2..=4).contains(&budget) {
            self.body_preparation_metrics.reason = "serial worker budget";
            return;
        }
        if config.structure.headers_only
            || !self.native_function_namespace_complete
            || !config.structure.expression.function_namespace_complete
            || self.incremental.is_active()
            || !self.active_parse_requests.is_empty()
            || self
                .incremental
                .documents
                .contains_key(&request.context.document_id)
        {
            self.body_preparation_metrics.reason = "headers-only or an existing document cache";
            return;
        }
        if !self.project_semantics_are_stable()
            || self.components.iter().any(|component| {
                component.disabled
                    || component.unloaded
                    || component.restart_pending
                    || !component_promises(
                        &component.manifest.capabilities,
                        crate::CAPABILITY_ISOLATED_PREPARATION_SAFE,
                    )
            })
        {
            self.body_preparation_metrics.reason = "provider isolation is not certified";
            return;
        }
        let tree = skript_parser::parse_raw_tree(
            &request.source,
            config.raw_tree.unwrap_or_else(|| self.raw_tree_options()),
        );
        let roots = tree
            .roots
            .iter()
            .filter(|id| {
                tree.get(**id).is_some_and(|node| {
                    matches!(
                        node.kind,
                        ParserRawNodeKind::Section | ParserRawNodeKind::Simple
                    )
                })
            })
            .count();
        if roots < 2 {
            self.body_preparation_metrics.reason = "fewer than two root candidates";
            return;
        }
        let helpers = budget.min(roots) - 1;
        let seed = Arc::new(Seed {
            runtime: Arc::clone(&self.runtime),
            config: self.config.clone(),
            matchers: Arc::clone(&self.type_user_input_matchers),
            components: self
                .components
                .iter()
                .map(|component| {
                    (
                        component.component.clone(),
                        component.manifest.clone(),
                        component.registered_handler_bindings.clone(),
                    )
                })
                .collect(),
            request: request.clone(),
            document: config.clone(),
            trace_origin: self.body_phase_clock.origin(),
        });
        let (sender, receiver) = mpsc::channel();
        let mut running = RunningPreparation {
            seed: Arc::clone(&seed),
            receiver,
            cancellation: Vec::new(),
            handles: Vec::new(),
            caches: BTreeMap::new(),
            received: 0,
            metrics: BodyPreparationMetrics {
                reason: "isolated read-only invocation preparation",
                trace: config
                    .trace_body_preparation
                    .then(BodyPreparationTrace::default),
                ..Default::default()
            },
        };
        for partition in 0..helpers {
            let seed = Arc::clone(&seed);
            let token = config.cancellation.child();
            let stop = token.clone();
            let sender = sender.clone();
            match thread::Builder::new()
                .name(format!("skript-body-prepare-{partition}"))
                .stack_size(32 * 1024 * 1024)
                .spawn(move || prepare(seed, partition, helpers, token, sender))
            {
                Ok(handle) => {
                    running.handles.push(handle);
                    running.cancellation.push(stop);
                    running.metrics.spawned += 1;
                }
                Err(_) => {
                    running.metrics.failed += 1;
                    if let Some(trace) = &mut running.metrics.trace {
                        trace.workers.push(BodyWorkerTrace {
                            partition,
                            outcome: "spawn-error",
                            ..Default::default()
                        });
                    }
                }
            }
        }
        self.body_preparation = Some(running);
    }

    pub(super) fn poll_body_preparation(&mut self) {
        if let Some(running) = &mut self.body_preparation {
            running.poll();
        }
    }

    pub(super) fn finish_body_preparation(&mut self) {
        if let Some(running) = self.body_preparation.take() {
            self.body_preparation_metrics = running.finish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn host() -> ParserHost {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/type-parser-versions/skript-2.15.4-type-runtime");
        let catalog = Arc::new(ssg::load(path).unwrap().into_catalog());
        ParserHost::new(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../artifacts/core-library.wasm"
            )),
            HostConfig {
                syntax_catalog: Some(catalog),
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn ready() -> (
        ParserHost,
        DocumentParseRequest,
        DocumentParserConfig,
        Capsule,
    ) {
        ready_with_trace(false)
    }

    fn ready_with_trace(
        trace: bool,
    ) -> (
        ParserHost,
        DocumentParseRequest,
        DocumentParserConfig,
        Capsule,
    ) {
        let mut host = host();
        let request = DocumentParseRequest::new(
            "file:///public-capsule",
            "same.sk",
            1,
            "on load:\n    send 1\non load:\n    send 2\non load:\n    send 3\n",
        );
        let config = DocumentParserConfig {
            body_preparation_workers: 2,
            trace_body_preparation: trace,
            ..Default::default()
        };
        host.body_phase_clock.begin_document(trace);
        host.start_body_preparation(&request, &config);
        let running = host
            .body_preparation
            .as_ref()
            .expect("certified Core must spawn an isolated helper");
        // An actual completion channel barrier, not a timing-dependent sleep.
        let capsule = running
            .receiver
            .recv_timeout(Duration::from_secs(20))
            .expect("helper completion");
        assert!(
            capsule
                .cache
                .as_ref()
                .is_some_and(|cache| !cache.entries.is_empty()),
            "a successfully validated cross-host response is required"
        );
        (host, request, config, capsule)
    }

    #[test]
    fn completed_same_attempt_capsule_is_revalidated_and_actually_hits() {
        let (mut host, request, config, capsule) = ready();
        host.body_preparation.as_mut().unwrap().accept(capsule);
        assert_eq!(host.body_preparation.as_ref().unwrap().metrics.completed, 1);
        let result = host.parse_document(request.clone(), config).unwrap();
        assert!(result.component_failures.is_empty());
        assert!(
            host.incremental_metrics().hits > 0,
            "the authoritative parser must actually consume a cross-host capsule"
        );
        let mut serial = self::host();
        let baseline = serial
            .parse_document(request, DocumentParserConfig::default())
            .unwrap();
        assert_eq!(
            format!("{:?}", result.syntax),
            format!("{:?}", baseline.syntax)
        );
        assert_eq!(format!("{:?}", result.ast), format!("{:?}", baseline.ast));
        assert_eq!(
            format!("{:?}", result.effects),
            format!("{:?}", baseline.effects)
        );
        assert_eq!(result.state, baseline.state);
        assert_eq!(result.functions.lookups(), baseline.functions.lookups());
        let witnesses = result.semantic_witnesses();
        let expected = baseline.semantic_witnesses();
        for (actual, original) in result.ast.nodes.iter().zip(&baseline.ast.nodes) {
            assert_eq!(
                witnesses.native_valid(actual),
                expected.native_valid(original)
            );
            assert_eq!(
                witnesses.provider_valid(actual, CORE_LIBRARY_COMPONENT_ID),
                expected.provider_valid(original, CORE_LIBRARY_COMPONENT_ID)
            );
        }
    }

    #[test]
    fn foreign_seed_and_cancelled_seed_never_enter_the_current_bank() {
        let (mut host, _request, config, mut capsule) = ready();
        capsule.seed = Arc::new((*capsule.seed).clone());
        host.body_preparation.as_mut().unwrap().accept(capsule);
        assert!(host.body_preparation.as_ref().unwrap().caches.is_empty());
        host.finish_body_preparation();
        let (mut host, _request, _config, capsule) = ready();
        host.body_preparation
            .as_ref()
            .unwrap()
            .seed
            .document
            .cancellation
            .cancel();
        host.body_preparation.as_mut().unwrap().accept(capsule);
        assert!(host.body_preparation.as_ref().unwrap().caches.is_empty());
        host.finish_body_preparation();
        assert!(!config.cancellation.is_cancelled());
    }

    #[test]
    fn isolated_store_rejects_durable_reads_scans_and_writes() {
        let (mut owner, _request, _config, capsule) = ready();
        let worker = capsule
            .seed
            .instantiate(DocumentCancellationToken::new())
            .unwrap();
        let parse = worker
            .begin_parse("file:///public-capsule", "durable.sk", 1)
            .unwrap();
        let mut invocation = parse.begin_invocation(CORE_LIBRARY_COMPONENT_ID).unwrap();
        for scope in [
            crate::state::StateScope::Document,
            crate::state::StateScope::Project,
            crate::state::StateScope::PersistentProject,
        ] {
            assert!(matches!(
                invocation.get(
                    scope,
                    crate::state::NamespaceVisibility::Private,
                    "aliases",
                    "absent"
                ),
                Err(StateError::AccessDenied { .. })
            ));
            assert!(matches!(
                invocation.scan_prefix(
                    scope,
                    crate::state::NamespaceVisibility::Private,
                    "aliases",
                    "",
                    1
                ),
                Err(StateError::AccessDenied { .. })
            ));
            assert!(matches!(
                invocation.delete(
                    scope,
                    crate::state::NamespaceVisibility::Private,
                    "aliases",
                    "absent"
                ),
                Err(StateError::AccessDenied { .. })
            ));
        }
        invocation.rollback();
        parse.cancel().unwrap();
        owner.finish_body_preparation();
    }
    #[test]
    fn cross_host_bank_rechecks_absent_reads_and_empty_scans() {
        use crate::bindings::nlaocs::skript_parser_addon::types::{HookInvocation, SourceOrigin};
        use crate::state::{NamespaceVisibility, StateEncoding, StateScope, StateValue};
        for scan in [false, true] {
            let (mut main, request, _config, mut capsule) = ready_with_trace(true);
            let worker = capsule
                .seed
                .instantiate(DocumentCancellationToken::new())
                .unwrap();
            let old = worker
                .begin_parse("file:///public-capsule", "same.sk", 1)
                .unwrap();
            old.enable_read_observations().unwrap();
            let checkpoint = old.observation_checkpoint().unwrap();
            let mut observed = old.begin_invocation(CORE_LIBRARY_COMPONENT_ID).unwrap();
            observed.observe_reads();
            if scan {
                assert!(
                    observed
                        .scan_prefix(
                            StateScope::Parse,
                            NamespaceVisibility::Private,
                            "aliases",
                            "missing",
                            1
                        )
                        .unwrap()
                        .is_empty()
                );
            } else {
                assert!(
                    observed
                        .get(
                            StateScope::Parse,
                            NamespaceVisibility::Private,
                            "aliases",
                            "missing"
                        )
                        .unwrap()
                        .is_none()
                );
            }
            let reads = observed.read_certificate(&checkpoint).unwrap().unwrap();
            observed.rollback();
            old.cancel().unwrap();
            let current = main
                .begin_parse("file:///public-capsule", "same.sk", 1)
                .unwrap();
            main.begin_incremental_document(&current, Arc::from(request.source.virtual_source()))
                .unwrap();
            let start = request.source.virtual_source().find("send").unwrap() as u64;
            let range = WitTextRange {
                start,
                end: start + 6,
            };
            let mut invocation = HookInvocation {
                context: InvocationContext {
                    invocation_id: 1,
                    subscription_id: "core.effect-semantics".into(),
                    document_id: "same.sk".into(),
                    document_revision: 1,
                    expansion: None,
                    syntax_context: 9,
                },
                target: HookTarget::SyntaxKind(SyntaxKind::Effect),
                phase: HookPhase::Effect,
                payload: HookPayload::Effect(WitEffectPayload {
                    input: request.source.virtual_source().to_owned(),
                    context: WitParseContext {
                        syntax_context: 9,
                        event_classes: vec!["test.Event".into()],
                        section_stack: vec![],
                        values: vec![],
                    },
                    raw_node_id: 2,
                    span: MappedSpan {
                        virtual_range: range,
                        origins: vec![SourceOrigin {
                            original_range: range,
                            kind: WitOriginKind::Exact,
                            expansion: None,
                        }],
                    },
                    timing: WitEffectTiming::Before,
                    candidate: None,
                    alternatives: vec![],
                    failure: None,
                    near_match: None,
                }),
                parse_results: vec![],
            };
            let key = main.incremental_key(0, &mut invocation).unwrap().unwrap();
            let cache = capsule.cache.as_mut().unwrap();
            cache.entries.clear();
            // This is a bridge/read-certificate unit: the real completed capsule
            // authenticates the seed, while the entry deliberately isolates the
            // State dependency from guest-specific syntax behavior.
            cache.entries.insert(
                key.clone(),
                incremental::CachedInvocation {
                    output: HookOutput {
                        decision: HookDecision::ContinueProcessing,
                        replacement: None,
                        effects: empty_effects(),
                    },
                    reads,
                    bytes: 1,
                    trace_slot: usize::MAX,
                    recorded_at_ms: None,
                },
            );
            main.body_preparation.as_mut().unwrap().accept(capsule);
            let mut replay = current.begin_invocation(CORE_LIBRARY_COMPONENT_ID).unwrap();
            replay.observe_reads();
            assert!(
                main.incremental_lookup(&key, &mut replay, &invocation.payload)
                    .unwrap()
                    .is_some(),
                "unchanged namespace must hit across separate stores"
            );
            replay.rollback();
            assert_eq!(main.incremental_metrics().hits, 1);
            let mut writer = current.begin_invocation(CORE_LIBRARY_COMPONENT_ID).unwrap();
            writer
                .put(
                    StateScope::Parse,
                    NamespaceVisibility::Private,
                    "aliases",
                    if scan { "missing.child" } else { "missing" },
                    StateValue::new(
                        "nlaocs.core-library.aliases",
                        StateEncoding::Raw,
                        b"now-present".to_vec(),
                    ),
                )
                .unwrap();
            writer.commit().unwrap();
            let prior = current.read_write_set().unwrap();
            let mut replay = current.begin_invocation(CORE_LIBRARY_COMPONENT_ID).unwrap();
            replay.observe_reads();
            assert!(
                main.incremental_lookup(&key, &mut replay, &invocation.payload)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(main.incremental_metrics().dependency_misses, 1);
            assert!(replay.read_write_set().reads.is_empty());
            replay.rollback();
            assert_eq!(current.read_write_set().unwrap(), prior);
            current.cancel().unwrap();
            main.finish_incremental_document("same.sk", 1, false);
            main.finish_body_preparation();
            let trace = main.body_preparation_metrics().trace.as_ref().unwrap();
            assert_eq!(trace.workers.len(), 1);
            assert_eq!(trace.workers[0].hit_calls, 1);
            assert_eq!(trace.workers[0].state_rejections, 1);
            assert_eq!(trace.workers[0].unique_consumed_entries, 1);
            assert_eq!(
                trace.workers[0].unused_state_rejected, 0,
                "a consumed entry must not be reclassified as unused after a later miss"
            );
        }
    }
}

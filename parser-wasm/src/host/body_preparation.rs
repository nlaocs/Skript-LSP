//! Isolated, optional preparation of semantic invocations. Workers never publish
//! an AST, declarations, state deltas, receipts, or completion notifications.
//! The authoritative parser still performs the complete ordered pipeline. A
//! finished worker capsule only lends opt-in read-only invocation responses to
//! its ordinary current-payload/read-certificate/output validation path.
use super::*;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

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
}

pub(super) struct RunningPreparation {
    seed: Arc<Seed>,
    receiver: Receiver<Capsule>,
    cancellation: Vec<DocumentCancellationToken>,
    handles: Vec<JoinHandle<()>>,
    caches: BTreeMap<usize, incremental::DocumentCache>,
    metrics: BodyPreparationMetrics,
}

impl RunningPreparation {
    fn accept(&mut self, capsule: Capsule) {
        if !Arc::ptr_eq(&capsule.seed, &self.seed) || self.seed.document.cancellation.is_cancelled()
        {
            return;
        }
        if let Some(cache) = capsule.cache {
            self.metrics.completed += 1;
            self.metrics.entries += cache.entries.len();
            self.metrics.bytes += cache
                .entries
                .values()
                .map(|entry| entry.bytes)
                .sum::<usize>();
            self.caches.insert(capsule.partition, cache);
        } else {
            self.metrics.failed += 1;
        }
    }

    fn poll(&mut self) {
        while let Ok(capsule) = self.receiver.try_recv() {
            self.accept(capsule);
        }
    }

    pub(super) fn lookup(&self, world: &str, key: &str) -> Option<&incremental::CachedInvocation> {
        if self.seed.document.cancellation.is_cancelled() {
            return None;
        }
        self.caches
            .values()
            .filter(|cache| cache.world == world)
            .find_map(|cache| cache.entries.get(key))
    }

    fn finish(mut self) -> BodyPreparationMetrics {
        let started = Instant::now();
        self.poll();
        for token in &self.cancellation {
            token.cancel();
        }
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
        self.poll();
        self.metrics.join_ms = started.elapsed().as_secs_f64() * 1000.0;
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
    let cache = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut host = seed.instantiate(token.clone()).ok()?;
        host.body_partition = Some((partition, partitions));
        let mut config = seed.document.clone();
        config.body_preparation_workers = 1;
        config.cancellation = token.clone();
        let result = host.parse_document(seed.request.clone(), config).ok()?;
        if token.is_cancelled()
            || result.execution_status != DocumentExecutionStatus::Complete
            || !result.component_failures.is_empty()
            || !result.fault_regions.is_empty()
            || !result.dependency_unavailable.is_empty()
        {
            return None;
        }
        // Only after the ordinary pipeline, output checks and private commit
        // completed. Pending entries are never streamed out of an active parse.
        host.incremental
            .documents
            .remove(&seed.request.context.document_id)
    }))
    .ok()
    .flatten();
    // Completed successful capsules have independent lifetimes: an unrelated
    // worker's later failure does not revoke a response already revalidated by
    // the authoritative parser. Failed workers lend no capsule or semantic fact.
    let _ = sender.send(Capsule {
        seed,
        partition,
        cache,
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
        });
        let (sender, receiver) = mpsc::channel();
        let mut running = RunningPreparation {
            seed: Arc::clone(&seed),
            receiver,
            cancellation: Vec::new(),
            handles: Vec::new(),
            caches: BTreeMap::new(),
            metrics: BodyPreparationMetrics {
                reason: "isolated read-only invocation preparation",
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
        let mut host = host();
        let request = DocumentParseRequest::new(
            "file:///public-capsule",
            "same.sk",
            1,
            "on load:\n    send 1\non load:\n    send 2\non load:\n    send 3\n",
        );
        let config = DocumentParserConfig {
            body_preparation_workers: 2,
            ..Default::default()
        };
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
            let (mut main, request, _config, mut capsule) = ready();
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
        }
    }
}

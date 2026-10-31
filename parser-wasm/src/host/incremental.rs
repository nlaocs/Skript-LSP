//! Opt-in semantic invocation reuse. Native matching and transaction validation
//! always run against the current document; no AST or old State delta is reused.
use super::*;
use crate::bindings::nlaocs::skript_parser_addon::types::HookInvocation;
use crate::state::{StateObservationCheckpoint, StateReadCertificate};
mod header;
pub(super) mod body;
mod body_source;

/// Cache identity preserves the fully resolved candidate/override order and
/// every signature. Absolute registration counters vary on identical fresh
/// collection; retain only their relative order, without mutating the registry.
pub(super) fn dynamic_world(mut snapshot: DynamicSyntaxSnapshot) -> String {
    snapshot.document_revision=0;snapshot.registry_revision=0;
    let orders=snapshot.definitions.values().map(|d|d.declaration_order)
        .chain(snapshot.overrides.values().map(|d|d.declaration_order))
        .chain(snapshot.candidates.iter().flat_map(|c|c.overrides.iter().map(|h|h.declaration_order)))
        .collect::<BTreeSet<_>>();
    let ranks=orders.into_iter().enumerate().map(|(rank,order)|(order,rank as u64)).collect::<BTreeMap<_,_>>();
    for definition in snapshot.definitions.values_mut() {definition.declaration_order=ranks[&definition.declaration_order];}
    for replacement in snapshot.overrides.values_mut() {replacement.declaration_order=ranks[&replacement.declaration_order];}
    for candidate in &mut snapshot.candidates {for handler in &mut candidate.overrides {handler.declaration_order=ranks[&handler.declaration_order];}}
    format!("{snapshot:?}")
}

const MAX_ENTRIES: usize = 4_096;
const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_DOCUMENTS: usize = 4;

/// The guest receives one of these exact current-source input views. Keep the
/// shape in the key: statement-local transport is not a full-document echo.
#[derive(Debug, Clone, Copy)]
enum AuthenticatedInputView {
    Absent,
    FullSource,
    ExactSpan,
    PhysicalLine,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IncrementalMetrics {
    pub body_eligible: u64,
    pub body_hits: u64,
    pub body_stored: u64,
    pub body_discarded: u64,
    pub eligible: u64,
    pub hits: u64,
    /// Explicit Structure Enter invocations eligible for header-local reuse.
    pub header_eligible: u64,
    /// Header semantic calls skipped after current dependency validation.
    pub header_hits: u64,
    pub misses: u64,
    pub dependency_misses: u64,
    pub stored: u64,
    pub discarded: u64,
}

#[derive(Clone)]
struct CachedInvocation {
    output: HookOutput,
    reads: StateReadCertificate,
    bytes: usize,
}

struct DocumentCache {
    revision: u64,
    world: String,
    entries: HashMap<String, CachedInvocation>,
    bodies: HashMap<String, Arc<body::CachedBody>>,
}

struct ActiveDocument {
    document: String,
    revision: u64,
    source: Arc<str>,
    physical_lines: Vec<skript_parser::PhysicalLine>,
    world: Option<String>,
    pending: HashMap<String, CachedInvocation>,
    pending_bodies: HashMap<String, Arc<body::CachedBody>>,
    body_publish_allowed: bool,
    bytes: usize,
    metrics: IncrementalMetrics,
}

#[derive(Default)]
pub(super) struct IncrementalCache {
    documents: BTreeMap<String, DocumentCache>,
    active: Option<ActiveDocument>,
    last_metrics: IncrementalMetrics,
}

impl IncrementalCache {
    pub(super) fn release(&mut self, document: &str, through: u64) {
        if self
            .documents
            .get(document)
            .is_some_and(|cache| cache.revision <= through)
        {
            self.documents.remove(document);
        }
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.document == document && active.revision <= through)
        {
            self.active = None;
        }
    }
}

impl ParserHost {
    pub(super) fn allow_body_cache_publication(&mut self, allowed:bool) {
        if let Some(active)=self.incremental.active.as_mut() { active.body_publish_allowed=allowed; }
    }
    /// Begin a revision-bound, opt-in cache session. Call before all macros, and
    /// finish it on every exit. Transformed inputs conservatively miss this v1.
    pub fn begin_incremental_document(
        &mut self,
        transaction: &ParseTransaction,
        source: Arc<str>,
    ) -> Result<(), HostError> {
        self.incremental.active = None;
        transaction.enable_read_observations()?;
        let physical_lines = skript_parser::physical_lines(&source);
        self.incremental.active = Some(ActiveDocument {
            document: transaction.document_id()?,
            revision: transaction.document_revision()?,
            source,
            physical_lines,
            world: None,
            pending: HashMap::new(),
            pending_bodies: HashMap::new(),
            body_publish_allowed: false,
            bytes: 0,
            metrics: IncrementalMetrics::default(),
        });
        Ok(())
    }

    /// Publish only a fully committed, fault-free current parse. Candidate-local
    /// rejection is ordinary data; cancellation and extension faults are not.
    pub fn finish_incremental_document(
        &mut self,
        document: &str,
        revision: u64,
        committed: bool,
    ) -> IncrementalMetrics {
        if !self
            .incremental
            .active
            .as_ref()
            .is_some_and(|active| active.document == document && active.revision == revision)
        {
            return IncrementalMetrics::default();
        }
        let mut active = self
            .incremental
            .active
            .take()
            .expect("matching active document");
        if committed && let Some(world) = active.world {
            if self
                .incremental
                .documents
                .get(document)
                .is_none_or(|cache| cache.revision <= revision)
            {
                let bodies = if active.body_publish_allowed { active.pending_bodies } else {
                    active.metrics.body_discarded += active.pending_bodies.len() as u64;
                    HashMap::new()
                };
                // Replacement, rather than union, drops invocations no longer
                // reachable in this document and keeps the memory bound honest.
                self.incremental.documents.insert(
                    document.to_owned(),
                    DocumentCache {
                        revision,
                        world,
                        entries: active.pending,
                        bodies,
                    },
                );
                while self.incremental.documents.len() > MAX_DOCUMENTS {
                    let oldest = self
                        .incremental
                        .documents
                        .iter()
                        .min_by_key(|(_, cache)| cache.revision)
                        .map(|(key, _)| key.clone())
                        .unwrap();
                    self.incremental.documents.remove(&oldest);
                }
            }
        } else {
            active.metrics.body_discarded += active.pending_bodies.len() as u64;
            active.metrics.discarded = active
                .metrics
                .discarded
                .saturating_add(active.pending.len() as u64);
        }
        self.incremental.last_metrics = active.metrics;
        active.metrics
    }

    pub fn incremental_metrics(&self) -> IncrementalMetrics {
        self.incremental
            .active
            .as_ref()
            .map_or(self.incremental.last_metrics, |active| active.metrics)
    }

    /// Publication evidence for lifecycle tests; pending counters are not proof
    /// that a failed revision replaced the last successful document cache.
    #[cfg(test)]
    pub(in crate::host) fn cached_body_revision(&self, document:&str)->Option<(u64,usize)> {
        self.incremental.documents.get(document).map(|cache|(cache.revision,cache.bodies.len()))
    }

    pub(super) fn incremental_key(
        &mut self,
        component: usize,
        invocation: &mut HookInvocation,
    ) -> Result<Option<String>, HostError> {
        #[cfg(test)]
        let _measurement = self.measurement_scope("dispatch.incremental_key");
        // An incomplete Function namespace cannot borrow a semantic response
        // produced with complete project candidates. No key means no lookup
        // or store, including providers whose continuation parses a Function.
        if !self.native_function_namespace_complete {
            return Ok(None);
        }
        let Some(active) = self.incremental.active.as_ref() else {
            return Ok(None);
        };
        let entry = &self.components[component];
        let header = header::eligible(entry, invocation);
        if (!header
            && !matches!(
                invocation.phase,
                HookPhase::Expression
                    | HookPhase::DefaultExpression
                    | HookPhase::Effect
                    | HookPhase::Condition
            ))
            || invocation.context.document_id != active.document
            || invocation.context.document_revision != active.revision
            || invocation.context.expansion.is_some()
            || !invocation.parse_results.is_empty()
            || !self.active_parse_requests.is_empty()
            || (!header
                && (!entry
                    .manifest
                    .incremental_local_subscriptions
                    .iter()
                    .any(|id| id == &invocation.context.subscription_id)
                    || !entry.manifest.capabilities.iter().any(|capability| {
                        capability.id == "parser.incremental-local-syntax"
                            && capability.minimum_version == 1
                    })))
        {
            return Ok(None);
        }
        let Some((start, end)) = exact_local_span(&invocation.payload) else {
            return Ok(None);
        };
        let source = active.source.clone();
        if end > source.len() || !source.is_char_boundary(start) || !source.is_char_boundary(end) {
            return Ok(None);
        }
        let Some(line_index) = active
            .physical_lines
            .partition_point(|line| line.content_range.start <= start)
            .checked_sub(1)
        else {
            return Ok(None);
        };
        let line = active.physical_lines[line_index];
        // This rejects bare-CR crossings, CRLF interiors and synthetic EOF
        // lines. A zero-width point at a real content end belongs to that line.
        if start < line.content_range.start || end > line.content_range.end {
            return Ok(None);
        }
        let line_start = line.content_range.start;
        let line_end = line.content_range.end;
        let full_line_end = line.full_range.end;
        let input_view = match payload_input(&invocation.payload) {
            None => AuthenticatedInputView::Absent,
            Some(input) if input == source.as_ref() => AuthenticatedInputView::FullSource,
            Some(input) if input == &source[start..end] => AuthenticatedInputView::ExactSpan,
            Some(input) if input == &source[line_start..line_end] => {
                AuthenticatedInputView::PhysicalLine
            }
            _ => return Ok(None),
        };
        if (header && !matches!(input_view, AuthenticatedInputView::FullSource))
            || !payload_is_data_only(&invocation.payload)
        {
            return Ok(None);
        }
        if self.incremental.active.as_ref().unwrap().world.is_none() {
            // Freeze is already required before semantic syntax dispatch. Its
            // identity counters vary by revision; its actual ordered facts do not.
            let dynamic = self
                .dynamic_syntax_registry
                .as_ref()
                .map(|registry| {
                    registry.freeze(
                        &invocation.context.document_id,
                        invocation.context.document_revision,
                    )
                })
                .transpose()?;
            let dynamic = dynamic.map(dynamic_world);
            let components = self
                .components
                .iter()
                .map(|entry| {
                    format!(
                        "{:?}|{:?}|{}|{}",
                        entry.manifest, entry.registered_handler_bindings, entry.load_order, entry.preparation_generation
                    )
                })
                .collect::<Vec<_>>();
            self.incremental.active.as_mut().unwrap().world =
                Some(format!("{dynamic:?}|{components:?}|{}", self.project_declaration_body_key()));
        }
        // Availability may change on an earlier component fault in this parse.
        let availability = self
            .components
            .iter()
            .map(|entry| (entry.disabled, entry.unloaded))
            .collect::<Vec<_>>();
        // Only full-source transport can contain contract-independent bytes
        // outside the current line. Exact span/line input remains byte-for-byte.
        let old_input = if !header && matches!(input_view, AuthenticatedInputView::FullSource) {
            payload_input_mut(&mut invocation.payload)
                .map(|input| mem::replace(input, source[line_start..full_line_end].to_owned()))
        } else {
            None
        };
        let old_id = invocation.context.invocation_id;
        let old_revision = invocation.context.document_revision;
        invocation.context.invocation_id = 0;
        invocation.context.document_revision = 0;
        // Exhaustive transport destructuring makes a future invocation-field
        // addition require an explicit key decision. Receipt aliases are kept,
        // but per-parse host numbers are normalized and restored before use.
        let key = if header {
            let mut context = invocation.context.clone();
            context.syntax_context = 0;
            let target = invocation.target.clone();
            super::semantic_proof::with_payload_receipts_normalized(
                &mut invocation.payload,
                |payload| {
                    let HookPayload::Structure(value) = payload else {
                        unreachable!()
                    };
                    header::key_payload(value).map(|payload| {
                        format!(
                            "header|{component}|{availability:?}|{context:?}|{:?}|{}|{payload}",
                            target,
                            &source[line_start..full_line_end]
                        )
                    })
                },
            )
            .flatten()
        } else {
            let HookInvocation {
                context,
                target,
                phase,
                payload,
                parse_results,
            } = &mut *invocation;
            super::semantic_proof::with_payload_receipts_normalized(payload, |payload| {
                format!(
                    "{component}|{availability:?}|{input_view:?}|{line:?}|{:?}|{context:?}|{target:?}|{phase:?}|{payload:?}|{parse_results:?}",
                    &source[line_start..full_line_end]
                )
            })
        };
        invocation.context.invocation_id = old_id;
        invocation.context.document_revision = old_revision;
        if let Some(input) = old_input {
            *payload_input_mut(&mut invocation.payload).unwrap() = input;
        }
        let active = self.incremental.active.as_mut().unwrap();
        active.metrics.eligible = active.metrics.eligible.saturating_add(1);
        if header && key.is_some() {
            active.metrics.header_eligible += 1;
        }
        Ok(key.filter(|key| key.len() <= MAX_BYTES / 4))
    }

    pub(super) fn incremental_lookup(
        &mut self,
        key: &str,
        state: &mut InvocationTransaction,
        payload: &HookPayload,
    ) -> Result<Option<HookOutput>, HostError> {
        #[cfg(test)]
        let _measurement = self.measurement_scope("dispatch.incremental_lookup");
        let active = self
            .incremental
            .active
            .as_mut()
            .expect("eligible invocation has an active session");
        let cached = self
            .incremental
            .documents
            .get(&active.document)
            .filter(|cache| {
                cache.revision < active.revision && Some(&cache.world) == active.world.as_ref()
            })
            .and_then(|cache| cache.entries.get(key));
        let Some(cached) = cached else {
            active.metrics.misses += 1;
            return Ok(None);
        };
        let mut output = cached.output.clone();
        if !super::semantic_proof::restore_output_receipts(payload, &mut output) {
            active.metrics.misses += 1;
            return Ok(None);
        }
        if !header::restore_output(payload, &mut output) {
            active.metrics.misses += 1;
            return Ok(None);
        }
        if !state.replay_read_certificate(&cached.reads)? {
            active.metrics.misses += 1;
            active.metrics.dependency_misses += 1;
            return Ok(None);
        }
        if let Some(replacement) = output.replacement.as_mut()
            && let Some(input) = payload_input_mut(replacement)
        {
            *input = payload_input(payload)
                .expect("cached echo has matching input payload")
                .to_owned();
        }
        active.metrics.hits += 1;
        if matches!(payload, HookPayload::Structure(_)) {
            active.metrics.header_hits += 1;
        }
        insert_pending(active, key.to_owned(), cached.clone());
        Ok(Some(output))
    }

    pub(super) fn incremental_record(
        &mut self,
        key: String,
        checkpoint: &StateObservationCheckpoint,
        state: &InvocationTransaction,
        payload: &HookPayload,
        output: &HookOutput,
        impure: bool,
    ) -> Result<(), HostError> {
        #[cfg(test)]
        let _measurement = self.measurement_scope("dispatch.incremental_record");
        if impure
            || matches!(&output.decision, HookDecision::Reject(rejection) if !rejection.diagnostics.is_empty())
            || !output.effects.diagnostics.is_empty()
            || (!matches!(payload, HookPayload::Structure(_))
                && !output.effects.context_updates.is_empty())
            || !output.effects.parse_requests.is_empty()
            || !output.effects.parse_results.is_empty()
        {
            return Ok(());
        }
        let Some(reads) = state.read_certificate(checkpoint)? else {
            return Ok(());
        };
        let mut output = output.clone();
        if !header::normalize_output(payload, &mut output) {
            return Ok(());
        }
        if let Some(replacement) = output.replacement.as_mut() {
            if mem::discriminant(replacement) != mem::discriminant(payload)
                || !payload_is_data_only(replacement)
            {
                return Ok(());
            }
            if !matches!(payload, HookPayload::Structure(_))
                && let Some(input) = payload_input_mut(replacement)
            {
                if Some(input.as_str()) != payload_input(payload) {
                    return Ok(());
                }
                input.clear(); // explicit immutable-input echo; rebound on a hit
            }
        }
        if !super::semantic_proof::normalize_output_receipts(payload, &mut output) {
            return Ok(());
        }
        let bytes = key
            .len()
            .saturating_add(format!("{output:?}{reads:?}").len());
        let active = self
            .incremental
            .active
            .as_mut()
            .expect("eligible invocation has an active session");
        insert_pending(
            active,
            key,
            CachedInvocation {
                output,
                reads,
                bytes,
            },
        );
        Ok(())
    }
}

fn insert_pending(active: &mut ActiveDocument, key: String, value: CachedInvocation) {
    if active.pending.contains_key(&key) {
        return;
    }
    if active.pending.len()+active.pending_bodies.len() >= MAX_ENTRIES || active.bytes.saturating_add(value.bytes) > MAX_BYTES {
        active.metrics.discarded += 1;
        return;
    }
    active.bytes += value.bytes;
    active.pending.insert(key, value);
    active.metrics.stored += 1;
}

fn payload_input(payload: &HookPayload) -> Option<&str> {
    match payload {
        HookPayload::Expression(value) => Some(&value.input),
        HookPayload::RegisteredExpression(value) => Some(&value.input),
        HookPayload::Effect(value) => Some(&value.input),
        HookPayload::Condition(value) => Some(&value.input),
        HookPayload::Structure(value) => Some(&value.input),
        _ => None,
    }
}

fn payload_input_mut(payload: &mut HookPayload) -> Option<&mut String> {
    match payload {
        HookPayload::Expression(value) => Some(&mut value.input),
        HookPayload::RegisteredExpression(value) => Some(&mut value.input),
        HookPayload::Effect(value) => Some(&mut value.input),
        HookPayload::Condition(value) => Some(&mut value.input),
        HookPayload::Structure(value) => Some(&mut value.input),
        _ => None,
    }
}

fn exact_local_span(payload: &HookPayload) -> Option<(usize, usize)> {
    let span = match payload {
        HookPayload::Expression(value) => &value.span,
        HookPayload::RegisteredExpression(value) => &value.span,
        HookPayload::DefaultExpression(value) => &value.span,
        HookPayload::Effect(value) => &value.span,
        HookPayload::Condition(value) => &value.candidate.span,
        HookPayload::Structure(value) => &value.candidate.span,
        _ => return None,
    };
    let [origin] = span.origins.as_slice() else {
        return None;
    };
    if !matches!(origin.kind, WitOriginKind::Exact)
        || origin.expansion.is_some()
        || origin.original_range.start != span.virtual_range.start
        || origin.original_range.end != span.virtual_range.end
    {
        return None;
    }
    let start = usize::try_from(span.virtual_range.start).ok()?;
    let end = usize::try_from(span.virtual_range.end).ok()?;
    (start <= end).then_some((start, end))
}

fn payload_is_data_only(payload: &HookPayload) -> bool {
    match payload {
        HookPayload::Expression(value) => value
            .candidates
            .iter()
            .all(|candidate| candidate.children.is_empty() && candidate.function.is_none()),
        HookPayload::RegisteredExpression(value) => value
            .parsed_captures
            .iter()
            .all(|capture| capture.attachments.is_empty() && capture.diagnostics.is_empty()),
        HookPayload::Effect(value) => {
            value
                .candidate
                .iter()
                .chain(value.alternatives.iter())
                .all(|candidate| {
                    candidate.parsed_captures.iter().all(|capture| {
                        capture.attachments.is_empty() && capture.diagnostics.is_empty()
                    })
                })
        }
        HookPayload::Structure(value) => {
            value.candidate.parsed_captures.is_empty()
                && value.candidate.entries.is_empty()
                && value.candidate.child_node_ids.is_empty()
        }
        HookPayload::DefaultExpression(_) | HookPayload::Condition(_) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::nlaocs::skript_parser_addon::types::{
        CapabilityRequirement, SourceOrigin,
    };
    const DOCUMENT: &str = "file:///workspace/incremental.sk";
    const SUBSCRIPTION: &str = "fixture.incremental.local";

    fn host() -> ParserHost {
        let mut host = ParserHost::new(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../artifacts/core-library.wasm"
            )),
            HostConfig {
                runtime_profile: RuntimeProfile {
                    skript_version: Some("2.15.4".into()),
                    ..RuntimeProfile::default()
                },
                ..HostConfig::default()
            },
        )
        .unwrap();
        // These tests exercise the transport cache directly, never execute a
        // production subscription under an unaudited local promise.
        host.components[0].manifest.incremental_local_subscriptions = vec![SUBSCRIPTION.into()];
        host.components[0]
            .manifest
            .capabilities
            .push(CapabilityRequirement {
                id: "parser.incremental-local-syntax".into(),
                minimum_version: 1,
                required: false,
            });
        host
    }
    fn invocation(source: &str, revision: u64) -> HookInvocation {
        let start = source.find("send").unwrap() as u64;
        let span = MappedSpan {
            virtual_range: WitTextRange {
                start,
                end: start + 6,
            },
            origins: vec![SourceOrigin {
                original_range: WitTextRange {
                    start,
                    end: start + 6,
                },
                kind: WitOriginKind::Exact,
                expansion: None,
            }],
        };
        HookInvocation {
            context: InvocationContext {
                invocation_id: revision * 7,
                subscription_id: SUBSCRIPTION.into(),
                document_id: DOCUMENT.into(),
                document_revision: revision,
                expansion: None,
                syntax_context: 9,
            },
            target: HookTarget::SyntaxKind(SyntaxKind::Effect),
            phase: HookPhase::Effect,
            payload: HookPayload::Effect(WitEffectPayload {
                input: source.into(),
                context: WitParseContext {
                    syntax_context: 9,
                    event_classes: vec!["test.Event".into()],
                    section_stack: Vec::new(),
                    values: Vec::new(),
                },
                raw_node_id: 2,
                span,
                timing: WitEffectTiming::Before,
                candidate: None,
                alternatives: Vec::new(),
                failure: None,
                near_match: None,
            }),
            parse_results: Vec::new(),
        }
    }
    fn begin(host: &mut ParserHost, source: &str, revision: u64) -> ParseTransaction {
        let parse = host
            .begin_parse("file:///workspace", DOCUMENT, revision)
            .unwrap();
        host.begin_incremental_document(&parse, Arc::from(source))
            .unwrap();
        parse
    }
    fn record(
        host: &mut ParserHost,
        parse: &ParseTransaction,
        source: &str,
        revision: u64,
    ) -> String {
        let mut invocation = invocation(source, revision);
        let key = host.incremental_key(0, &mut invocation).unwrap().unwrap();
        let before = parse.observation_checkpoint().unwrap();
        let mut state = parse
            .begin_invocation(host.components[0].manifest.component_id.clone())
            .unwrap();
        state.observe_reads();
        let output = HookOutput {
            decision: HookDecision::ContinueProcessing,
            replacement: Some(invocation.payload.clone()),
            effects: empty_effects(),
        };
        host.incremental_record(
            key.clone(),
            &before,
            &state,
            &invocation.payload,
            &output,
            false,
        )
        .unwrap();
        state.rollback();
        key
    }
    #[test]
    fn dynamic_world_ignores_attempt_counters_but_keeps_actual_order_and_signatures() {
        use syntaxes::{AliasRegistry,CatalogParts};
        let plural_rules=syntax_pattern_parser::syntax::PluralRules::from_json(r#"{"algorithm":"singular-aware","pluralOverrideSupported":false,"rules":[{"ruleOrder":0,"singular":"","plural":"s","completeWord":false,"origin":"built-in","addon":{"name":"Skript","version":"fixture"}}]}"#).unwrap();
        let registry=DynamicSyntaxRegistry::new(Arc::new(Catalog::new(CatalogParts {
            syntaxes:vec![],converters:vec![],comparators:vec![],event_values:vec![],properties:vec![],operators:vec![],
            operations:BTreeMap::new(),differences:vec![],classes:vec![],aliases:AliasRegistry {aliases:BTreeMap::new(),targets:vec![]},
            plural_rules,language:BTreeMap::new(),
        })));
        let mut snapshots=Vec::new();
        for (revision,ids,handler) in [(1,["a","b"],"same"),(2,["a","b"],"same"),(3,["b","a"],"same"),(4,["a","b"],"different")] {
            registry.begin_document(DOCUMENT,revision).unwrap();
            let mut update=registry.begin_document_update("test.project",0,DOCUMENT,revision).unwrap();
            for id in ids {update.register(DynamicSyntaxInput {local_id:id.into(),kind:CatalogSyntaxKind::Effect,
                patterns:vec![format!("project {id}")],priority:0,before:vec![],after:vec![],return_type:None,
                return_multiplicity:None,structure_node_type:None,structure_body_mode:None,entry_validator:None,
                handler:handler.into(),metadata:BTreeMap::new()}).unwrap();}
            update.commit().unwrap();snapshots.push(registry.freeze(DOCUMENT,revision).unwrap());
        }
        assert_ne!(snapshots[0].definitions,snapshots[1].definitions,"real registration counters advance");
        assert_eq!(dynamic_world(snapshots[0].clone()),dynamic_world(snapshots[1].clone()));
        assert_ne!(dynamic_world(snapshots[1].clone()),dynamic_world(snapshots[2].clone()),"actual candidate order remains a dependency");
        assert_ne!(dynamic_world(snapshots[1].clone()),dynamic_world(snapshots[3].clone()),"handler/signature changes invalidate");
    }

    #[test]
    fn line_local_key_reuses_only_current_complete_context_and_rebinds_input_echo() {
        let mut host = host();
        let first = "on join:\n    send 1\n# old\n";
        let second = "on join:\n    send 1\n# new\n";
        let parse = begin(&mut host, first, 1);
        let old_key = record(&mut host, &parse, first, 1);
        parse.commit().unwrap();
        assert_eq!(
            host.finish_incremental_document(DOCUMENT, 1, true).stored,
            1
        );
        let parse = begin(&mut host, second, 2);
        let mut current = invocation(second, 2);
        let key = host.incremental_key(0, &mut current).unwrap().unwrap();
        assert_eq!(key, old_key);
        assert_eq!(payload_input(&current.payload), Some(second));
        let mut state = parse
            .begin_invocation(host.components[0].manifest.component_id.clone())
            .unwrap();
        state.observe_reads();
        let output = host
            .incremental_lookup(&key, &mut state, &current.payload)
            .unwrap()
            .unwrap();
        assert_eq!(
            payload_input(output.replacement.as_ref().unwrap()),
            Some(second)
        );
        assert_eq!(host.incremental_metrics().hits, 1);
        // Context, node identity, local prefix, and absolute positions are not
        // inferred from an unchanged word; each remains part of the actual key.
        let mut changed = current.clone();
        if let HookPayload::Effect(value) = &mut changed.payload {
            value.context.event_classes.push("other.Event".into());
        }
        assert_ne!(host.incremental_key(0, &mut changed).unwrap().unwrap(), key);
        let mut changed = current.clone();
        if let HookPayload::Effect(value) = &mut changed.payload {
            value.raw_node_id += 1;
        }
        assert_ne!(host.incremental_key(0, &mut changed).unwrap().unwrap(), key);
        state.rollback();
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 2, false);
        assert_eq!(host.incremental.documents[DOCUMENT].revision, 1);
    }
    #[test]
    fn source_preparation_generation_invalidates_previous_invocation_world() {
        let mut host=host();let source="on join:\n    send 1\n";
        let parse=begin(&mut host,source,1);
        let old=record(&mut host,&parse,source,1);
        parse.commit().unwrap();assert_eq!(host.finish_incremental_document(DOCUMENT,1,true).stored,1);
        host.components[0].preparation_generation+=1;
        let parse=begin(&mut host,source,2);let mut current=invocation(source,2);
        let new=host.incremental_key(0,&mut current).unwrap().unwrap();
        // Local input keys stay stable; the independent document world is the fence.
        assert_eq!(old,new);assert!(host.incremental.documents[DOCUMENT].entries.contains_key(&old));
        assert_ne!(Some(&host.incremental.documents[DOCUMENT].world),host.incremental.active.as_ref().unwrap().world.as_ref());
        let mut state=parse.begin_invocation(host.components[0].manifest.component_id.clone()).unwrap();
        state.observe_reads();assert!(host.incremental_lookup(&new,&mut state,&current.payload).unwrap().is_none());
        state.rollback();parse.cancel().unwrap();host.finish_incremental_document(DOCUMENT,2,false);
    }

    #[test]
    fn authenticated_local_input_views_keep_shape_context_and_current_echo() {
        let mut host = host();
        let first = "on join:\n    send 1 # old\n# first\n";
        let second = "on join:\n    send 1 # old\n# later\n";
        let parse = begin(&mut host, first, 1);
        let mut local = invocation(first, 1);
        *payload_input_mut(&mut local.payload).unwrap() = "send 1".into();
        let key = host.incremental_key(0, &mut local).unwrap().unwrap();
        let mut full = invocation(first, 1);
        assert_ne!(host.incremental_key(0, &mut full).unwrap().unwrap(), key);
        let mut line = invocation(first, 1);
        *payload_input_mut(&mut line.payload).unwrap() = "    send 1 # old".into();
        let line_key = host.incremental_key(0, &mut line).unwrap().unwrap();
        assert_ne!(line_key, key);
        let mut arbitrary = invocation(first, 1);
        *payload_input_mut(&mut arbitrary.payload).unwrap() = "unrelated send 1".into();
        assert!(host.incremental_key(0, &mut arbitrary).unwrap().is_none());
        let before = parse.observation_checkpoint().unwrap();
        let mut state = parse
            .begin_invocation(host.components[0].manifest.component_id.clone())
            .unwrap();
        state.observe_reads();
        let output = HookOutput {
            decision: HookDecision::ContinueProcessing,
            replacement: Some(local.payload.clone()),
            effects: empty_effects(),
        };
        host.incremental_record(key.clone(), &before, &state, &local.payload, &output, false)
            .unwrap();
        state.rollback();
        parse.commit().unwrap();
        host.finish_incremental_document(DOCUMENT, 1, true);
        let parse = begin(&mut host, second, 2);
        let mut current = invocation(second, 2);
        *payload_input_mut(&mut current.payload).unwrap() = "send 1".into();
        let current_key = host.incremental_key(0, &mut current).unwrap().unwrap();
        assert_eq!(current_key, key);
        let mut state = parse
            .begin_invocation(host.components[0].manifest.component_id.clone())
            .unwrap();
        state.observe_reads();
        let output = host
            .incremental_lookup(&current_key, &mut state, &current.payload)
            .unwrap()
            .unwrap();
        assert_eq!(
            payload_input(output.replacement.as_ref().unwrap()),
            Some("send 1")
        );
        assert_eq!(host.incremental_metrics().hits, 1);
        state.rollback();
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 2, false);
        let changed = "on join:\n    send 1 # new\n# later\n";
        let parse = begin(&mut host, changed, 3);
        let mut current = invocation(changed, 3);
        *payload_input_mut(&mut current.payload).unwrap() = "send 1".into();
        assert_ne!(
            host.incremental_key(0, &mut current).unwrap().unwrap(),
            key,
            "same span bytes cannot hide changed physical-line context"
        );
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 3, false);
    }
    #[test]
    fn physical_line_endings_and_boundaries_are_semantic_dependencies() {
        for (first_ending, second_ending) in
            [("\n", ""), ("", "\n"), ("\r\n", "\r"), ("\r", "\r\n")]
        {
            let mut host = host();
            let first = format!("on join:\n    send 1{first_ending}");
            let second = format!("on join:\n    send 1{second_ending}");
            let parse = begin(&mut host, &first, 1);
            let mut previous = invocation(&first, 1);
            let key = host.incremental_key(0, &mut previous).unwrap().unwrap();
            parse.cancel().unwrap();
            host.finish_incremental_document(DOCUMENT, 1, false);
            let parse = begin(&mut host, &second, 2);
            let mut current = invocation(&second, 2);
            assert_ne!(
                host.incremental_key(0, &mut current).unwrap().unwrap(),
                key,
                "ending {first_ending:?} -> {second_ending:?} must not reuse"
            );
            parse.cancel().unwrap();
            host.finish_incremental_document(DOCUMENT, 2, false);
        }
        let mut host = host();
        let source = "on join:\r    send 1\rmore";
        let parse = begin(&mut host, source, 1);
        let mut crossing = invocation(source, 1);
        if let HookPayload::Effect(value) = &mut crossing.payload {
            value.span.virtual_range.end = source.len() as u64;
            value.span.origins[0].original_range.end = source.len() as u64;
        }
        assert!(
            host.incremental_key(0, &mut crossing).unwrap().is_none(),
            "a bare-CR physical line boundary cannot be crossed"
        );
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 1, false);
        let source = "on join:\r\n    send 1\r\n";
        let parse = begin(&mut host, source, 2);
        for offset in [source.rfind("\r\n").unwrap() + 1, source.len()] {
            let mut point = invocation(source, 2);
            if let HookPayload::Effect(value) = &mut point.payload {
                value.span.virtual_range.start = offset as u64;
                value.span.virtual_range.end = offset as u64;
                value.span.origins[0].original_range.start = offset as u64;
                value.span.origins[0].original_range.end = offset as u64;
            }
            assert!(
                host.incremental_key(0, &mut point).unwrap().is_none(),
                "CRLF interior or synthetic trailing line is not content"
            );
        }
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 2, false);
    }
    #[test]
    fn transformed_nonexact_and_multiline_inputs_require_fresh_execution() {
        let mut host = host();
        let source = "on join:\n    send 1\n";
        let parse = begin(&mut host, source, 1);
        let mut call = invocation(source, 1);
        if let HookPayload::Effect(value) = &mut call.payload {
            value.input.push_str("# macro");
        }
        assert!(host.incremental_key(0, &mut call).unwrap().is_none());
        let mut call = invocation(source, 1);
        if let HookPayload::Effect(value) = &mut call.payload {
            value.span.origins[0].kind = WitOriginKind::Anchored;
        }
        assert!(host.incremental_key(0, &mut call).unwrap().is_none());
        let mut call = invocation(source, 1);
        if let HookPayload::Effect(value) = &mut call.payload {
            value.span.virtual_range.start = 0;
            value.span.origins[0].original_range.start = 0;
        }
        assert!(host.incremental_key(0, &mut call).unwrap().is_none());
        let mut call = invocation(source, 1);
        call.phase = HookPhase::Matching;
        assert!(host.incremental_key(0, &mut call).unwrap().is_none());
        host.components[0]
            .manifest
            .incremental_local_subscriptions
            .clear();
        assert!(
            host.incremental_key(0, &mut invocation(source, 1))
                .unwrap()
                .is_none()
        );
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 1, false);
    }
    #[test]
    fn current_line_prefix_and_shifted_absolute_positions_are_dependencies() {
        let mut host = host();
        let source = "on join:\n    send 1\n";
        let parse = begin(&mut host, source, 1);
        let key = host
            .incremental_key(0, &mut invocation(source, 1))
            .unwrap()
            .unwrap();
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 1, false);
        let changed_prefix = "on join:\n   \tsend 1\n";
        let parse = begin(&mut host, changed_prefix, 2);
        assert_ne!(
            host.incremental_key(0, &mut invocation(changed_prefix, 2))
                .unwrap()
                .unwrap(),
            key
        );
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 2, false);
        let shifted = "# inserted\non join:\n    send 1\n";
        let parse = begin(&mut host, shifted, 3);
        assert_ne!(
            host.incremental_key(0, &mut invocation(shifted, 3))
                .unwrap()
                .unwrap(),
            key
        );
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 3, false);
    }
    #[test]
    fn release_is_revision_fenced_and_failed_finish_never_publishes() {
        let mut host = host();
        let source = "on join:\n    send 1\n";
        let parse = begin(&mut host, source, 10);
        record(&mut host, &parse, source, 10);
        parse.cancel().unwrap();
        assert_eq!(
            host.finish_incremental_document(DOCUMENT, 10, false)
                .discarded,
            1
        );
        assert!(!host.incremental.documents.contains_key(DOCUMENT));
        let parse = begin(&mut host, source, 11);
        record(&mut host, &parse, source, 11);
        parse.commit().unwrap();
        host.finish_incremental_document(DOCUMENT, 11, true);
        host.incremental.release(DOCUMENT, 10);
        assert!(host.incremental.documents.contains_key(DOCUMENT));
        host.incremental.release(DOCUMENT, 11);
        assert!(!host.incremental.documents.contains_key(DOCUMENT));
    }
    #[test]
    fn diagnostics_impure_operations_and_changed_echo_are_not_stored() {
        let mut host = host();
        let source = "on join:\n    send 1\n";
        let parse = begin(&mut host, source, 1);
        let mut call = invocation(source, 1);
        let key = host.incremental_key(0, &mut call).unwrap().unwrap();
        let before = parse.observation_checkpoint().unwrap();
        let mut state = parse
            .begin_invocation(host.components[0].manifest.component_id.clone())
            .unwrap();
        state.observe_reads();
        let mut output = HookOutput {
            decision: HookDecision::ContinueProcessing,
            replacement: Some(call.payload.clone()),
            effects: empty_effects(),
        };
        host.incremental_record(key.clone(), &before, &state, &call.payload, &output, true)
            .unwrap();
        assert!(host.incremental.active.as_ref().unwrap().pending.is_empty());
        let span = if let HookPayload::Effect(value) = &call.payload {
            value.span.clone()
        } else {
            unreachable!()
        };
        output.effects.diagnostics.push(Diagnostic {
            code: "fixture.diagnostic".into(),
            message: "not cacheable".into(),
            severity: DiagnosticSeverity::Error,
            span,
            related: Vec::new(),
        });
        host.incremental_record(key.clone(), &before, &state, &call.payload, &output, false)
            .unwrap();
        assert!(host.incremental.active.as_ref().unwrap().pending.is_empty());
        output.decision = HookDecision::Reject(Rejection {
            reason: "fixture rejection".into(),
            diagnostics: mem::take(&mut output.effects.diagnostics),
        });
        host.incremental_record(key.clone(), &before, &state, &call.payload, &output, false)
            .unwrap();
        assert!(
            host.incremental.active.as_ref().unwrap().pending.is_empty(),
            "rejection diagnostics are not reusable"
        );
        output.decision = HookDecision::ContinueProcessing;
        if let Some(HookPayload::Effect(value)) = &mut output.replacement {
            value.input = "wrong source".into();
        }
        host.incremental_record(key, &before, &state, &call.payload, &output, false)
            .unwrap();
        assert!(host.incremental.active.as_ref().unwrap().pending.is_empty());
        state.rollback();
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 1, false);
    }
    fn receipt_payload(first: &str, second: &str) -> HookPayload {
        use crate::bindings::nlaocs::skript_parser_addon::types::{
            EffectCandidate, ParseSummary, ParsedCapture,
        };
        let mut payload = invocation("on join:\n    send 1\n", 1).payload;
        let HookPayload::Effect(value) = &mut payload else {
            unreachable!()
        };
        let receipt = |token: &str| WitMetadataEntry {
            key: "parser.semantic-writer-receipt.v1".into(),
            value: token.into(),
            owner_component_id: None,
        };
        let summary = ParseSummary {
            kind: "literal".into(),
            definition_id: None,
            registration_id: None,
            element_class: None,
            pattern_index: None,
            return_type: None,
            possible_return_types: Vec::new(),
            possible_return_types_state: WitPossibleReturnTypesState::Complete,
            multiplicity: None,
            default_expression: None,
            public_data: Vec::new(),
            metadata: vec![receipt(second)],
        };
        let candidate = EffectCandidate {
            raw_node_id: 2,
            definition_id: "fixture.definition".into(),
            registration_id: "fixture.registration".into(),
            element_class: None,
            priority: 0,
            registration_order: 0,
            pattern_index: 0,
            pattern: "send %object%".into(),
            span: value.span.clone(),
            captures: Vec::new(),
            tags: Vec::new(),
            mark: 0,
            marks: Vec::new(),
            handler: None,
            metadata: vec![
                receipt(first),
                WitMetadataEntry {
                    key: "parser.semantic-writer-receipt.v1".into(),
                    value: "addon semantic data".into(),
                    owner_component_id: Some("fixture.addon".into()),
                },
            ],
            parsed_captures: vec![ParsedCapture {
                capture_index: 0,
                parser_id: "host.expression".into(),
                status: WitParseResultStatus::Success,
                text: "1".into(),
                span: value.span.clone(),
                expected_types: Vec::new(),
                summary: Some(summary),
                attachments: Vec::new(),
                diagnostics: Vec::new(),
            }],
        };
        value.candidate = Some(candidate.clone());
        value.alternatives.push(candidate);
        payload
    }

    #[test]
    fn cache_receipts_preserve_nested_alias_topology_and_restore_current_ids() {
        let mut old = receipt_payload("11", "22");
        let old_before = format!("{old:?}");
        let old_key =
            super::super::semantic_proof::with_payload_receipts_normalized(&mut old, |payload| {
                format!("{payload:?}")
            })
            .unwrap();
        assert_eq!(
            format!("{old:?}"),
            old_before,
            "scoped key normalization restores the actual request"
        );
        let mut current = receipt_payload("101", "202");
        let current_key = super::super::semantic_proof::with_payload_receipts_normalized(
            &mut current,
            |payload| format!("{payload:?}"),
        )
        .unwrap();
        assert_eq!(old_key, current_key);
        let mut collapsed = receipt_payload("101", "101");
        assert_ne!(
            old_key,
            super::super::semantic_proof::with_payload_receipts_normalized(
                &mut collapsed,
                |payload| format!("{payload:?}")
            )
            .unwrap()
        );
        let mut output = HookOutput {
            decision: HookDecision::ContinueProcessing,
            replacement: Some(old.clone()),
            effects: empty_effects(),
        };
        assert!(super::super::semantic_proof::normalize_output_receipts(
            &old,
            &mut output
        ));
        assert!(super::super::semantic_proof::restore_output_receipts(
            &current,
            &mut output
        ));
        assert_eq!(
            format!("{:?}", output.replacement.unwrap()),
            format!("{current:?}"),
            "nested summaries and alternatives use current registry IDs; addon-owned data stays intact"
        );
    }

    #[test]
    fn unknown_or_invalid_host_receipts_are_not_cacheable() {
        let original = receipt_payload("11", "22");
        let mut output = HookOutput {
            decision: HookDecision::ContinueProcessing,
            replacement: Some(receipt_payload("999", "22")),
            effects: empty_effects(),
        };
        assert!(!super::super::semantic_proof::normalize_output_receipts(
            &original,
            &mut output
        ));
        let mut invalid = receipt_payload("invalid", "22");
        assert!(
            super::super::semantic_proof::with_payload_receipts_normalized(&mut invalid, |_| ())
                .is_none()
        );
        let mut output = HookOutput {
            decision: HookDecision::ContinueProcessing,
            replacement: Some(receipt_payload("1", "999")),
            effects: empty_effects(),
        };
        assert!(!super::super::semantic_proof::restore_output_receipts(
            &original,
            &mut output
        ));
    }
}

//! First native body reuse case: exact geometry, read-only State, no Function
//! lookups or AdditionalParse resources. Nested editing owners require real
//! enter-time preparation and current consumed-resource proofs. Unknown paths
//! stay cold; headers, ExitBody and the final proof are always current.
use super::*;
use crate::state::{BodyObservationCheckpoint, BodyReadCertificate};
use skript_parser::{PortableStructureBody, StructureBodyPlan, StructureBodyReplay, StructureBodyReuseRequest, StructureBodyWork};
mod preparation;
use preparation::BodyPreparations;

const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_BODY_CALLS: usize = 4096;

#[cfg(test)]
pub(in crate::host) fn shadow_original_range(source:&skript_parser::MappedSource, range:ParserTextRange) -> Option<ParserTextRange> {
    super::body_source::original_extent(source,range,|_,_|Ok(())).ok()
}

impl ParserHost {
    /// Optional diagnostic counters only; never part of State or reuse authority.
    pub(in crate::host) fn body_admission_count(&self, name: &'static str) {
        #[cfg(test)]
        if let Some(probe) = &self.body_admission_probe { probe.record(name); }
        let _ = name;
    }
}

/// Conditional declarations certify the entire invocation, not a single selected
/// resolver. Target matching is deliberately broader than parser selectors: a
/// guest can consult registration bindings before testing pattern/tag/mark.
pub(in crate::host) fn subscription_is_body_local(
    component: &ComponentEntry,
    invocation: &crate::bindings::nlaocs::skript_parser_addon::types::HookInvocation,
    catalog: Option<&Catalog>,
) -> bool {
    let id = &invocation.context.subscription_id;
    if component.manifest.incremental_body_subscriptions.contains(id) {
        return true;
    }
    let Some(scope) = component.manifest.incremental_body_handler_subscriptions.iter()
        .find(|scope| scope.subscription_id == *id) else { return false; };
    let HookPayload::RegisteredExpression(payload) = &invocation.payload else { return false; };
    if invocation.phase != HookPhase::Expression { return false; }
    let Some(identity) = catalog.and_then(|catalog|
        registered_syntax_identity(catalog, MatchSyntaxKind::Expression, &payload.registration_id))
        else { return false; };
    if identity.definition_id != payload.definition_id { return false; }
    // RegisteredExpression does not transport an authenticated dynamic handler
    // identity. Do not mistake an unknown DynamicHandler target for a nonmatch.
    if component.manifest.registered_syntax_handlers.iter().any(|handler|
        handler.kind == SyntaxKind::Expression && handler.phase == HookPhase::Expression
            && handler.targets.iter().any(|target| matches!(target, RegisteredSyntaxHandlerTarget::DynamicHandler(_)))) {
        return false;
    }
    let mut found = false;
    for handler in &component.manifest.registered_syntax_handlers {
        if handler.kind != SyntaxKind::Expression || handler.phase != HookPhase::Expression {
            continue;
        }
        if registered_handler_target_matches(component, handler,
            &payload.definition_id, &payload.registration_id, None) {
            found = true;
            if !scope.handler_ids.contains(&handler.handler_id) {
                return false;
            }
        }
    }
    found
}

pub(in crate::host) fn debug_reason(host:&ParserHost, reason:&str) {
    #[cfg(test)]
    if host.body_debug { eprintln!("body reuse bypass: {reason}"); }
    let _=(host,reason);
}

pub(in crate::host) struct BodyObservation {
    key: String,
    before: StructureBodyWork,
    state: BodyObservationCheckpoint,
    effects: HookEffectsCheckpoint,
    proposal: Option<Arc<CachedBody>>,
    pending: Option<Arc<CachedBody>>,
    invalid: bool,
    preparations: BodyPreparations,
    replay_applied: bool,
    #[cfg(test)]
    read_refused: bool,
    #[cfg(test)]
    shadow: Option<body_shadow::Lease>,
}

#[derive(Clone)]
pub(super) struct CachedBody {
    replay: StructureBodyReplay,
    effects: HookEffects,
    calls: Vec<HookCall>,
    reads: BodyReadCertificate,
    receipts: semantic_proof::BodyReceiptBundle,
    bytes: usize,
}

struct BoundedSize(usize);
impl std::fmt::Write for BoundedSize {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        self.0 = self.0.saturating_add(value.len());
        if self.0 > MAX_BODY_BYTES { Err(std::fmt::Error) } else { Ok(()) }
    }
}
impl WasmExpressionEnvironment<'_> {
    #[cfg(test)]
    pub(in crate::host) fn shadow_preparation(&self, event:&skript_parser::StructurePreparationEvent<'_>) {
        if let Some(observation)=&self.body
            && observation.shadow.is_some()
            && let Some(probe)=&self.hooks.host.body_shadow_probe {
            body_shadow::preparation(probe,&self.hooks,&observation.effects,event);
        }
    }

    #[cfg(test)]
    fn shadow_finish(&self, body:&skript_parser::StructureBody, context:&ExpressionParseContext, work:StructureBodyWork) {
        if let Some(observation)=&self.body && let Some(lease)=&observation.shadow {
            body_shadow::finish(lease,&self.hooks,body,context,work,observation.before,
                &observation.state,&observation.effects,self.pending_leaf.is_some()
                    || self.pending_registered.is_some() || !self.expression_candidates.is_empty());
        }
    }

    pub(in crate::host) fn body_reuse_enabled(&self) -> bool {
        !self.hooks.host.body_observing
            && self.hooks.host.incremental.active.is_some()
            && self.hooks.host.components.iter().filter(|entry| !entry.disabled && !entry.unloaded)
                .all(|entry| (!entry.manifest.incremental_body_subscriptions.is_empty()
                    || !entry.manifest.incremental_body_handler_subscriptions.is_empty())
                    && component_promises(&entry.manifest.capabilities, crate::CAPABILITY_INCREMENTAL_STRUCTURE_BODY))
    }

    pub(in crate::host) fn body_prepare(&mut self, request: StructureBodyReuseRequest<'_>) -> Result<StructureBodyPlan,String> {
        self.hooks.host.body_admission_count("body_admission.prepare.total");
        let document=self.hooks.transaction.document_id().map_err(|e|self.hooks.host_error(e))?;
        let revision=self.hooks.transaction.document_revision().map_err(|e|self.hooks.host_error(e))?;
        if self.hooks.host.incremental.active.as_ref().is_none_or(|active|
            active.document!=document || active.revision!=revision
                || active.document!=self.hooks.context.document_id
                || active.revision!=self.hooks.context.document_revision
                || active.source.as_ref()!=request.source.original()) {
            self.hooks.host.body_admission_count("body_admission.prepare.reject.document_context");
            return Ok(StructureBodyPlan::Parse);
        }
        let key = match self.body_key(&request) {
            Ok(key) => key,
            Err(reason) => {
                self.hooks.host.body_admission_count(reason);
                return Ok(StructureBodyPlan::Parse);
            }
        };
        self.hooks.host.body_admission_count("body_admission.prepare.admitted");
        let active = self.hooks.host.incremental.active.as_mut().unwrap();
        active.metrics.body_eligible += 1;
        let world = active.world.as_ref().unwrap();
        let proposal = self.hooks.host.incremental.documents.get(&active.document)
            .filter(|previous| previous.revision < active.revision && &previous.world == world)
            .and_then(|previous| previous.bodies.get(&key)).cloned();
        self.hooks.host.body_admission_count(if proposal.is_some() {
            "body_admission.prepare.reuse_proposed"
        } else {"body_admission.prepare.observe"});
        #[cfg(test)]
        let shadow = self.hooks.host.body_shadow_probe.as_ref()
            .and_then(|probe| probe.begin(&request, &key, revision,self.hooks.transaction,
                self.hooks.host.incremental.active.as_ref().unwrap().world.as_ref().unwrap()));
        self.body = Some(BodyObservation {
            key, before:request.work,
            state:self.hooks.transaction.body_observation_checkpoint().map_err(|e|self.hooks.host_error(e))?,
            effects:HookEffectsCheckpoint::capture(&self.hooks.effects,&self.hooks.calls,&self.hooks.failures),
            proposal:proposal.clone(),pending:None,invalid:false,
            preparations:BodyPreparations::new(request.candidate.raw_node_id,request.parent_attempt),
            replay_applied:false,
            #[cfg(test)]
            read_refused:false,
            #[cfg(test)]
            shadow,
        });
        self.hooks.host.body_observing=true;
        self.hooks.host.body_impure=false;
        Ok(proposal.map_or(StructureBodyPlan::Observe,|entry|StructureBodyPlan::Reuse(entry.replay.clone())))
    }

    fn body_key(&mut self, request: &StructureBodyReuseRequest<'_>) -> Result<String, &'static str> {
        use std::fmt::Write;
        // A new key generation includes the real ordered segment witness; the
        // flattened MappedSpan is retained as well, never rewritten/coalesced.
        struct Key(sha2::Sha256);
        impl std::fmt::Write for Key {
            fn write_str(&mut self,value:&str)->std::fmt::Result {
                use sha2::Digest;self.0.update(value.as_bytes());Ok(())
            }
        }
        use sha2::Digest;
        let mut key=Key(sha2::Sha256::new());
        write!(&mut key,"body-v2|").map_err(|_|"body_admission.prepare.reject.digest")?;
        let root=request.tree.get(request.candidate.raw_node_id).ok_or("body_admission.prepare.reject.owner_missing")?;
        let end=root.body_span.as_ref().ok_or("body_admission.prepare.reject.body_span_missing")?.virtual_range.end.max(root.span.virtual_range.end);
        let range=ParserTextRange::new(root.span.virtual_range.start,end);
        if range.len()>1024*1024 { return Err("body_admission.prepare.reject.source_budget"); }
        let span=request.source.map_range(range).map_err(|_|"body_admission.prepare.reject.mapping_error")?;
        let extent=super::body_source::original_extent(request.source,range,|virtual_range,origin|
            write!(&mut key,"segment:{virtual_range:?}|{origin:?}|"))?;
        let original=extent.slice(request.source.original()).ok_or("body_admission.prepare.reject.original_range")?;
        let current=range.slice(request.source.virtual_source()).ok_or("body_admission.prepare.reject.virtual_range")?;
        let mut candidate=request.candidate.clone();
        candidate.matched.extension_state=None;
        let authority=self.hooks.host.semantic_receipts.as_ref().ok_or("body_admission.prepare.reject.receipt_registry_missing")?
            .body_header_identity(&mut candidate.metadata).ok_or("body_admission.prepare.reject.header_authority")?;
        let mut nodes=Vec::new();
        let mut seen=BTreeSet::new();
        let mut pending=vec![root.id];
        while let Some(id)=pending.pop() {
            if !seen.insert(id) || nodes.len()>=4096 {return Err("body_admission.prepare.reject.tree_shape_or_budget");}
            let node=request.tree.get(id).ok_or("body_admission.prepare.reject.raw_node_missing")?;
            pending.extend(node.children.iter().rev().copied());
            nodes.push(node);
        }
        let active=self.hooks.host.incremental.active.as_mut().ok_or("body_admission.prepare.reject.active_document_missing")?;
        if active.world.is_none() {
            let dynamic=self.hooks.dynamic_snapshot.cloned().map(|mut value| {
                value.document_revision=0;value.registry_revision=0;format!("{value:?}")
            });
            let components=self.hooks.host.components.iter().map(|entry|
                format!("{:?}|{:?}|{}|{}",entry.manifest,entry.registered_handler_bindings,entry.load_order,entry.preparation_generation)).collect::<Vec<_>>();
            active.world=Some(format!("{dynamic:?}|{components:?}"));
        }
        let availability=self.hooks.host.components.iter().map(|e|(e.disabled,e.unloaded)).collect::<Vec<_>>();
        // Stream into the digest; never retain full source in keys or traces.
        write!(&mut key,"{extent:?}|{original:?}|{current:?}|{span:?}|{nodes:?}|{candidate:?}|{authority}|{:?}|{:?}|{:?}|{}|{availability:?}",
            request.context,request.config,request.work,request.parent_attempt).map_err(|_|"body_admission.prepare.reject.digest")?;
        Ok(format!("{:x}",key.0.finalize()))
    }

    pub(in crate::host) fn body_accept(&mut self) -> Result<bool,String> {
        let Some(proposal)=self.body.as_ref().and_then(|body|body.proposal.clone()) else {return Ok(false);};
        self.hooks.host.body_admission_count("body_admission.accept.total");
        let Some(registry)=self.hooks.host.semantic_receipts.as_ref() else {
            self.hooks.host.body_admission_count("body_admission.accept.reject.registry");return Ok(false);
        };
        if !registry.can_import_body(&proposal.receipts) {
            self.hooks.host.body_admission_count("body_admission.accept.reject.receipts");return Ok(false);
        }
        #[cfg(test)]
        let read_profile=self.hooks.host.body_admission_probe.as_ref().filter(|p|p.read_refusals_enabled()).map(|probe| {
            let detail=probe.take_read_detail();
            let profile=self.hooks.transaction.body_read_refusal_profile(&proposal.reads,detail)
                .unwrap_or_else(|_|serde_json::json!({"observationError":true,"detailRequested":detail}));
            (probe.clone(),profile)
        });
        let replayed=self.hooks.transaction.replay_body_read_certificate(&proposal.reads);
        #[cfg(test)]
        if let Some((probe,profile))=read_profile {probe.record_read(&self.body.as_ref().unwrap().key,replayed.as_ref().ok().copied(),profile);}
        if !replayed.map_err(|e|self.hooks.host_error(e))? {
            #[cfg(test)]
            {self.body.as_mut().unwrap().read_refused=true;}
            self.hooks.host.body_admission_count("body_admission.accept.reject.reads");return Ok(false);
        }
        if !self.hooks.host.semantic_receipts.as_mut().unwrap().import_body(&proposal.receipts) {
            return Err("body receipt admission changed during exclusive replay".into());
        }
        merge_shared_effects(&mut self.hooks.effects,proposal.effects.clone());
        let mut calls=proposal.calls.clone();
        for call in &mut calls {call.origin=HookCallOrigin::ReusedStructureBody;}
        self.hooks.calls.extend(calls);
        self.body.as_mut().unwrap().replay_applied=true;
        self.hooks.host.incremental.active.as_mut().unwrap().metrics.body_hits+=1;
        self.hooks.host.body_admission_count("body_admission.accept.replayed");
        Ok(true)
    }

    pub(in crate::host) fn body_preparation(&mut self, event:&skript_parser::StructurePreparationEvent<'_>) {
        if let Some(body)=self.body.as_mut() {
            if body.invalid {return;}
            if body.replay_applied {
                let expected=body.proposal.as_ref().and_then(|p|p.replay.preparations.get(body.preparations.entries().len()));
                if !matches!((expected,event),(Some(expected),skript_parser::StructurePreparationEvent::SectionHeader {
                    candidate,attempt_id,parent_attempt_id,..}) if expected.candidate==**candidate
                        && expected.attempt_id==*attempt_id && expected.parent_attempt_id==*parent_attempt_id) {
                    body.invalid=true;return;
                }
            }
            if !body.preparations.observe(event,&self.hooks,&body.effects) {body.invalid=true;}
        }
    }

    pub(in crate::host) fn body_finish(&mut self, body:&mut skript_parser::StructureBody, context:&ExpressionParseContext, work:StructureBodyWork) -> Result<bool,String> {
        #[cfg(test)]
        self.shadow_finish(body,context,work);
        use std::fmt::Write;
        self.hooks.host.body_observing=false;
        let Some(observation)=self.body.as_ref() else {return Ok(false);};
        if observation.replay_applied && observation.invalid {
            return Err("reused body preparation changed after admission".into());
        }
        self.hooks.host.body_admission_count("body_admission.finish.total");
        let before=&observation.effects;
        let mut reason="body_admission.finish.reject.invalid";
        if observation.invalid
            || {reason="body_admission.finish.reject.impure";self.hooks.host.body_impure}
            || {reason="body_admission.finish.reject.failures";self.hooks.failures.len()!=before.failures_len}
            || {reason="body_admission.finish.reject.diagnostics";self.hooks.effects.diagnostics.len()!=before.effects.diagnostics.len()}
            || {reason="body_admission.finish.reject.parse_requests";self.hooks.effects.parse_requests.len()!=before.effects.parse_requests.len()}
            || {reason="body_admission.finish.reject.parse_results";self.hooks.effects.parse_results.len()!=before.effects.parse_results.len()}
            || {reason="body_admission.finish.reject.pending_leaf";!self.pending_leaf.is_none()}
            || {reason="body_admission.finish.reject.pending_registered";!self.pending_registered.is_none()}
            || {reason="body_admission.finish.reject.expression_scopes";!self.expression_candidates.is_empty()}
            || {reason="body_admission.finish.reject.call_prefix";!before.calls.is_prefix_of(&self.hooks.calls)}
            || {reason="body_admission.finish.reject.call_budget";self.hooks.calls.len().saturating_sub(before.calls.len())>MAX_BODY_CALLS} {
            self.hooks.host.body_admission_count(reason);
            #[cfg(test)]
            debug_reason(self.hooks.host,&format!("scope/effect/resource guard invalid={} impure={} failures={} diagnostics={} requests={} results={} leaf={} registered={} expression-scopes={} prefix={}",
                observation.invalid,self.hooks.host.body_impure,self.hooks.failures.len()!=before.failures_len,
                self.hooks.effects.diagnostics.len()!=before.effects.diagnostics.len(),
                self.hooks.effects.parse_requests.len()!=before.effects.parse_requests.len(),
                self.hooks.effects.parse_results.len()!=before.effects.parse_results.len(),
                self.pending_leaf.is_some(),self.pending_registered.is_some(),self.expression_candidates.len(),before.calls.is_prefix_of(&self.hooks.calls)));
            return Ok(false);
        }
        let mut size=BoundedSize(0);
        if write!(&mut size,"{body:?}|{context:?}").is_err() {
            self.hooks.host.body_admission_count("body_admission.finish.reject.serialized_budget");return Ok(false);
        }
        let reads=self.hooks.transaction.body_read_certificate(&observation.state);
        let Some(reads)=reads.map_err(|e|self.hooks.host_error(e))? else {
            self.hooks.host.body_admission_count("body_admission.finish.reject.readonly");
            debug_reason(self.hooks.host,"State is not certifiably readonly");return Ok(false);
        };
        #[cfg(test)]
        if observation.read_refused && let Some(probe)=self.hooks.host.body_admission_probe.as_ref().filter(|p|p.read_refusals_enabled()) {
            probe.record_read_finish(&observation.key,observation.proposal.as_ref().unwrap().reads.compare_fresh_read_identity(&reads));
        }
        let calls=self.hooks.calls.suffix(before.calls.len());
        let updates=&self.hooks.effects.context_updates[before.effects.context_updates.len()..];
        let mut consumed=|kind,value:&skript_parser::ExpressionEffects|
            consumed_resource(&self.hooks,&calls,updates,kind,value);
        if !observation.preparations.validate_current(body,&mut consumed) {
            self.hooks.host.body_admission_count("body_admission.finish.reject.preparations");
            if observation.replay_applied {return Err("reused body lost current preparation coverage".into());}
            return Ok(false);
        }
        let Some(portable)=PortableStructureBody::freeze(body,&mut consumed) else {
            self.hooks.host.body_admission_count("body_admission.finish.reject.portable");
            debug_reason(self.hooks.host,"native resources are not portable");return Ok(false);
        };
        if !portable.matches_preparations(observation.preparations.entries()) {
            self.hooks.host.body_admission_count("body_admission.finish.reject.preparation_identity");
            if observation.replay_applied {return Err("reused body preparation identity changed".into());}
            return Ok(false);
        }
        let mut metadata=portable.metadata().to_vec();
        metadata.extend(context.values.iter().map(|(k,v)|(k.clone(),v.clone())));
        metadata.extend(observation.preparations.metadata().iter().cloned());
        for frame in &context.section_stack {
            metadata.extend(frame.metadata.iter().map(|(k,v)|(k.clone(),v.clone())));
            for capture in &frame.captures {metadata.extend(capture.metadata.iter().map(|(k,v)|(k.clone(),v.clone())));}
        }
        let Some(receipts)=self.hooks.host.semantic_receipts.as_ref().and_then(|r|r.export_body(&calls,&metadata)) else {
            self.hooks.host.body_admission_count("body_admission.finish.reject.receipts");
            debug_reason(self.hooks.host,"receipt graph is not closed");return Ok(false);
        };
        let effects=HookEffects {diagnostics:Vec::new(),context_updates:updates.to_vec(),parse_requests:Vec::new(),parse_results:Vec::new()};
        size.0=size.0.saturating_add(reads.retained_bytes());
        size.0=size.0.saturating_add(observation.preparations.retained_bytes());
        size.0=size.0.saturating_add(portable.metadata().len().saturating_mul(std::mem::size_of::<(String,String)>()));
        if write!(&mut size,"{:?}",portable.metadata()).is_err() {
            self.hooks.host.body_admission_count("body_admission.finish.reject.retained_budget");return Ok(false);
        }
        if size.0>MAX_BODY_BYTES || write!(&mut size,"{receipts:?}|{effects:?}|{calls:?}").is_err() {
            self.hooks.host.body_admission_count("body_admission.finish.reject.retained_budget");return Ok(false);
        }
        let entry=Arc::new(CachedBody {
            replay:StructureBodyReplay {before:observation.before,body:portable.clone(),post_context:context.clone(),work,
                preparations:observation.preparations.entries().to_vec()},
            effects,calls,reads,receipts,bytes:size.0,
        });
        *body=portable.into_body();
        self.body.as_mut().unwrap().pending=Some(entry);
        self.hooks.host.body_admission_count("body_admission.finish.ready");
        Ok(true)
    }

    pub(in crate::host) fn body_adopt(&mut self, accepted:bool) {
        self.hooks.host.body_observing=false;
        let Some(body)=self.body.take() else {return;};
        let Some(entry)=body.pending else {return;};
        let Some(active)=self.hooks.host.incremental.active.as_mut() else {return;};
        // Only an actual pending entry starts this stage. Earlier refusal must
        // not be counted again as an adoption refusal.
        #[cfg(test)]
        let probe=self.hooks.host.body_admission_probe.clone();
        #[cfg(test)]
        if let Some(probe)=&probe {probe.record("body_admission.adopt.total");}
        if accepted && active.pending.len()+active.pending_bodies.len()<MAX_ENTRIES
            && active.bytes.saturating_add(body.key.len()).saturating_add(entry.bytes)<=MAX_BYTES {
            active.bytes+=body.key.len()+entry.bytes;
            active.pending_bodies.insert(body.key,entry);
            active.metrics.body_stored+=1;
            #[cfg(test)]
            if let Some(probe)=&probe {probe.record("body_admission.adopt.stored");}
        } else {
            active.metrics.body_discarded+=1;
            #[cfg(test)]
            if let Some(probe)=&probe {probe.record(if !accepted {"body_admission.adopt.reject.not_accepted"}
                else {"body_admission.adopt.reject.capacity"});}
        }
    }
}

/// A known resource must have been consumed in this transaction. Used both at
/// the real enter event and at body end; identity alone never grants authority.
fn consumed_resource(hooks:&WasmPatternHooks<'_>, calls:&[HookCall], updates:&[ContextUpdate],
    kind:skript_parser::ConsumedBodyResource, value:&skript_parser::ExpressionEffects) -> bool {
    match kind {
        skript_parser::ConsumedBodyResource::Expression=>value.downcast_ref::<DeferredExpressionEffects>().is_some_and(|e|
            hooks.transaction.delta_is_currently_applied(&e.state).unwrap_or(false)
                && e.failures.is_empty() && e.effects.diagnostics.is_empty()
                && e.effects.parse_requests.is_empty() && e.effects.parse_results.is_empty()
                && (e.calls.is_empty() || calls.windows(e.calls.len()).any(|part|part==e.calls.as_slice()))
                && (e.effects.context_updates.is_empty() || updates.windows(e.effects.context_updates.len()).any(|part|
                    part.iter().zip(&e.effects.context_updates).all(|(a,b)|
                        a.syntax_context==b.syntax_context && a.key==b.key && a.value==b.value)))),
        skript_parser::ConsumedBodyResource::Pattern=>value.downcast_ref::<SavedPatternCandidate>().is_some_and(|e|
            hooks.transaction.owns_savepoint(&e.state).unwrap_or(false)
                && e.restored.as_ref().is_some_and(|flag|flag.load(Ordering::Relaxed))
                && e.effects.calls.is_prefix_of(&hooks.calls)),
    }
}

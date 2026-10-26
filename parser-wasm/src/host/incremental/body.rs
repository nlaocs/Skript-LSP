//! First native body reuse case: exact geometry, read-only State, no Function
//! lookups, nested editing owners or AdditionalParse resources. Unknown paths
//! stay cold; headers, ExitBody and the final proof are always current.
use super::*;
use crate::state::{BodyObservationCheckpoint, BodyReadCertificate};
use skript_parser::{PortableStructureBody, StructureBodyPlan, StructureBodyReplay, StructureBodyReuseRequest, StructureBodyWork};

const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_BODY_CALLS: usize = 4096;

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
    pub(in crate::host) fn body_reuse_enabled(&self) -> bool {
        !self.hooks.host.body_observing
            && self.hooks.host.incremental.active.is_some()
            && self.hooks.host.components.iter().filter(|entry| !entry.disabled && !entry.unloaded)
                .all(|entry| !entry.manifest.incremental_body_subscriptions.is_empty()
                    && component_promises(&entry.manifest.capabilities, crate::CAPABILITY_INCREMENTAL_STRUCTURE_BODY))
    }

    pub(in crate::host) fn body_prepare(&mut self, request: StructureBodyReuseRequest<'_>) -> Result<StructureBodyPlan,String> {
        let document=self.hooks.transaction.document_id().map_err(|e|self.hooks.host_error(e))?;
        let revision=self.hooks.transaction.document_revision().map_err(|e|self.hooks.host_error(e))?;
        if self.hooks.host.incremental.active.as_ref().is_none_or(|active|
            active.document!=document || active.revision!=revision
                || active.document!=self.hooks.context.document_id
                || active.revision!=self.hooks.context.document_revision
                || active.source.as_ref()!=request.source.original()) {
            return Ok(StructureBodyPlan::Parse);
        }
        let Some(key) = self.body_key(&request) else { return Ok(StructureBodyPlan::Parse); };
        let active = self.hooks.host.incremental.active.as_mut().unwrap();
        active.metrics.body_eligible += 1;
        let world = active.world.as_ref().unwrap();
        let proposal = self.hooks.host.incremental.documents.get(&active.document)
            .filter(|previous| previous.revision < active.revision && &previous.world == world)
            .and_then(|previous| previous.bodies.get(&key)).cloned();
        self.body = Some(BodyObservation {
            key, before:request.work,
            state:self.hooks.transaction.body_observation_checkpoint().map_err(|e|self.hooks.host_error(e))?,
            effects:HookEffectsCheckpoint::capture(&self.hooks.effects,&self.hooks.calls,&self.hooks.failures),
            proposal:proposal.clone(),pending:None,invalid:false,
        });
        self.hooks.host.body_observing=true;
        self.hooks.host.body_impure=false;
        Ok(proposal.map_or(StructureBodyPlan::Observe,|entry|StructureBodyPlan::Reuse(entry.replay.clone())))
    }

    fn body_key(&mut self, request: &StructureBodyReuseRequest<'_>) -> Option<String> {
        use std::fmt::Write;
        let root=request.tree.get(request.candidate.raw_node_id)?;
        let end=root.body_span.as_ref()?.virtual_range.end.max(root.span.virtual_range.end);
        let range=ParserTextRange::new(root.span.virtual_range.start,end);
        if range.len()>1024*1024 { return None; }
        let span=request.source.map_range(range).ok()?;
        let [origin]=span.origins.as_slice() else {return None;};
        if origin.kind!=skript_parser::OriginKind::Exact || origin.expansion.is_some()
            || origin.original_range.len()!=range.len() {return None;}
        let original=origin.original_range.slice(request.source.original())?;
        let current=range.slice(request.source.virtual_source())?;
        let mut candidate=request.candidate.clone();
        candidate.matched.extension_state=None;
        let authority=self.hooks.host.semantic_receipts.as_ref()?.body_header_identity(&mut candidate.metadata)?;
        let mut nodes=Vec::new();
        let mut seen=BTreeSet::new();
        let mut pending=vec![root.id];
        while let Some(id)=pending.pop() {
            if !seen.insert(id) || nodes.len()>=4096 {return None;}
            let node=request.tree.get(id)?;
            pending.extend(node.children.iter().rev().copied());
            nodes.push(node);
        }
        let active=self.hooks.host.incremental.active.as_mut()?;
        if active.world.is_none() {
            let dynamic=self.hooks.dynamic_snapshot.cloned().map(|mut value| {
                value.document_revision=0;value.registry_revision=0;format!("{value:?}")
            });
            let components=self.hooks.host.components.iter().map(|entry|
                format!("{:?}|{:?}|{}",entry.manifest,entry.registered_handler_bindings,entry.load_order)).collect::<Vec<_>>();
            active.world=Some(format!("{dynamic:?}|{components:?}"));
        }
        let availability=self.hooks.host.components.iter().map(|e|(e.disabled,e.unloaded)).collect::<Vec<_>>();
        // Stream into the digest; never retain full source in keys or traces.
        struct Key(sha2::Sha256);
        impl std::fmt::Write for Key {
            fn write_str(&mut self,value:&str)->std::fmt::Result {
                use sha2::Digest;self.0.update(value.as_bytes());Ok(())
            }
        }
        use sha2::Digest;
        let mut key=Key(sha2::Sha256::new());
        write!(&mut key,"body-v1|{original:?}|{current:?}|{span:?}|{nodes:?}|{candidate:?}|{authority}|{:?}|{:?}|{:?}|{}|{availability:?}",
            request.context,request.config,request.work,request.parent_attempt).ok()?;
        Some(format!("{:x}",key.0.finalize()))
    }

    pub(in crate::host) fn body_accept(&mut self) -> Result<bool,String> {
        let Some(proposal)=self.body.as_ref().and_then(|body|body.proposal.clone()) else {return Ok(false);};
        let Some(registry)=self.hooks.host.semantic_receipts.as_ref() else {return Ok(false);};
        if !registry.can_import_body(&proposal.receipts) {return Ok(false);}
        if !self.hooks.transaction.replay_body_read_certificate(&proposal.reads)
            .map_err(|e|self.hooks.host_error(e))? {return Ok(false);}
        if !self.hooks.host.semantic_receipts.as_mut().unwrap().import_body(&proposal.receipts) {
            return Err("body receipt admission changed during exclusive replay".into());
        }
        merge_shared_effects(&mut self.hooks.effects,proposal.effects.clone());
        let mut calls=proposal.calls.clone();
        for call in &mut calls {call.origin=HookCallOrigin::ReusedStructureBody;}
        self.hooks.calls.extend(calls);
        self.hooks.host.incremental.active.as_mut().unwrap().metrics.body_hits+=1;
        Ok(true)
    }

    pub(in crate::host) fn body_preparation(&mut self) {
        // No nested editing owner is portable in the first enabled host case.
        if let Some(body)=self.body.as_mut() {body.invalid=true;}
    }

    pub(in crate::host) fn body_finish(&mut self, body:&mut skript_parser::StructureBody, context:&ExpressionParseContext, work:StructureBodyWork) -> Result<bool,String> {
        use std::fmt::Write;
        self.hooks.host.body_observing=false;
        let Some(observation)=self.body.as_ref() else {return Ok(false);};
        let before=&observation.effects;
        if observation.invalid || self.hooks.host.body_impure
            || self.hooks.failures.len()!=before.failures_len
            || self.hooks.effects.diagnostics.len()!=before.effects.diagnostics.len()
            || self.hooks.effects.parse_requests.len()!=before.effects.parse_requests.len()
            || self.hooks.effects.parse_results.len()!=before.effects.parse_results.len()
            || !self.pending_leaf.is_none() || !self.pending_registered.is_none()
            || !self.expression_candidates.is_empty()
            || !before.calls.is_prefix_of(&self.hooks.calls)
            || self.hooks.calls.len().saturating_sub(before.calls.len())>MAX_BODY_CALLS {
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
        if write!(&mut size,"{body:?}|{context:?}").is_err() {return Ok(false);}
        let reads=self.hooks.transaction.body_read_certificate(&observation.state);
        let Some(reads)=reads.map_err(|e|self.hooks.host_error(e))? else {debug_reason(self.hooks.host,"State is not certifiably readonly");return Ok(false);};
        let calls=self.hooks.calls.suffix(before.calls.len());
        let updates=&self.hooks.effects.context_updates[before.effects.context_updates.len()..];
        let tx=self.hooks.transaction;
        let mut consumed=|kind,value:&skript_parser::ExpressionEffects| {
            match kind {
                skript_parser::ConsumedBodyResource::Expression => value.downcast_ref::<DeferredExpressionEffects>()
                    .is_some_and(|e| tx.delta_is_currently_applied(&e.state).unwrap_or(false)
                        && e.failures.is_empty() && e.effects.diagnostics.is_empty()
                        && e.effects.parse_requests.is_empty() && e.effects.parse_results.is_empty()
                        && (e.calls.is_empty() || calls.windows(e.calls.len()).any(|part|part==e.calls.as_slice()))
                        && (e.effects.context_updates.is_empty() || updates.windows(e.effects.context_updates.len()).any(|part|
                            part.iter().zip(&e.effects.context_updates).all(|(a,b)|
                                a.syntax_context==b.syntax_context && a.key==b.key && a.value==b.value)))),
                skript_parser::ConsumedBodyResource::Pattern => value.downcast_ref::<SavedPatternCandidate>()
                    .is_some_and(|e| tx.owns_savepoint(&e.state).unwrap_or(false)
                        && e.restored.as_ref().is_some_and(|flag|flag.load(Ordering::Relaxed))
                        && e.effects.calls.is_prefix_of(&self.hooks.calls)),
            }
        };
        let Some(portable)=PortableStructureBody::freeze(body,&mut consumed) else {debug_reason(self.hooks.host,"native resources are not portable");return Ok(false);};
        let mut metadata=portable.metadata().to_vec();
        metadata.extend(context.values.iter().map(|(k,v)|(k.clone(),v.clone())));
        for frame in &context.section_stack {
            metadata.extend(frame.metadata.iter().map(|(k,v)|(k.clone(),v.clone())));
            for capture in &frame.captures {metadata.extend(capture.metadata.iter().map(|(k,v)|(k.clone(),v.clone())));}
        }
        let Some(receipts)=self.hooks.host.semantic_receipts.as_ref().and_then(|r|r.export_body(&calls,&metadata)) else {debug_reason(self.hooks.host,"receipt graph is not closed");return Ok(false);};
        let effects=HookEffects {diagnostics:Vec::new(),context_updates:updates.to_vec(),parse_requests:Vec::new(),parse_results:Vec::new()};
        size.0=size.0.saturating_add(reads.retained_bytes());
        size.0=size.0.saturating_add(portable.metadata().len().saturating_mul(std::mem::size_of::<(String,String)>()));
        if write!(&mut size,"{:?}",portable.metadata()).is_err() {return Ok(false);}
        if size.0>MAX_BODY_BYTES || write!(&mut size,"{receipts:?}|{effects:?}|{calls:?}").is_err() {return Ok(false);}
        let entry=Arc::new(CachedBody {
            replay:StructureBodyReplay {before:observation.before,body:portable.clone(),post_context:context.clone(),work,preparations:Vec::new()},
            effects,calls,reads,receipts,bytes:size.0,
        });
        *body=portable.into_body();
        self.body.as_mut().unwrap().pending=Some(entry);
        Ok(true)
    }

    pub(in crate::host) fn body_adopt(&mut self, accepted:bool) {
        self.hooks.host.body_observing=false;
        let Some(body)=self.body.take() else {return;};
        let Some(entry)=body.pending else {return;};
        let Some(active)=self.hooks.host.incremental.active.as_mut() else {return;};
        if accepted && active.pending.len()+active.pending_bodies.len()<MAX_ENTRIES
            && active.bytes.saturating_add(body.key.len()).saturating_add(entry.bytes)<=MAX_BYTES {
            active.bytes+=body.key.len()+entry.bytes;
            active.pending_bodies.insert(body.key,entry);
            active.metrics.body_stored+=1;
        } else {active.metrics.body_discarded+=1;}
    }
}

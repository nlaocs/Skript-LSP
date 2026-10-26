//! Real enter-time proposals, frozen only after their resources are consumed.
//! A weak identity is a coverage witness, never authority or an old State delta.
use super::*;
use skript_parser::{ConsumedBodyResource, ExpressionEffects, ExpressionEffectsIdentity,
    RawNodeId, ReusedSectionPreparation, SectionBodyNode, StructurePreparationEvent};
use std::fmt::Write;

const MAX_PREPARATIONS:usize=128;
const MAX_RESOURCES:usize=4096;

struct Resource {owner:RawNodeId,kind:ConsumedBodyResource,identity:ExpressionEffectsIdentity}

pub(super) struct BodyPreparations {
    root:RawNodeId,
    parent:u64,
    entries:Vec<ReusedSectionPreparation>,
    resources:Vec<Resource>,
    metadata:Vec<(String,String)>,
    bytes:usize,
}

impl BodyPreparations {
    pub(super) fn new(root:RawNodeId,parent:u64)->Self {
        Self {root,parent,entries:Vec::new(),resources:Vec::new(),metadata:Vec::new(),bytes:0}
    }
    pub(super) fn entries(&self)->&[ReusedSectionPreparation] {&self.entries}
    pub(super) fn metadata(&self)->&[(String,String)] {&self.metadata}
    pub(super) fn retained_bytes(&self)->usize {self.bytes}

    pub(super) fn observe(&mut self,event:&StructurePreparationEvent<'_>,hooks:&WasmPatternHooks<'_>,
        before:&HookEffectsCheckpoint)->bool {
        let StructurePreparationEvent::SectionHeader {source,tree,candidate,attempt_id,parent_attempt_id}=event else {
            // Invalidations, new Structure headers and unknown owner lifecycles
            // cannot be silently treated as adopted nested preparations.
            return false;
        };
        if self.entries.len()>=MAX_PREPARATIONS || !candidate.body.is_empty()
            || self.entries.iter().any(|p|p.attempt_id==*attempt_id || p.candidate.raw_node_id==candidate.raw_node_id)
            || hooks.host.incremental.active.as_ref().is_none_or(|a|a.source.as_ref()!=source.original()) {
            return false;
        }
        let Some(node)=tree.get(candidate.raw_node_id) else {return false;};
        let expected_parent=if node.parent==Some(self.root) {Some(self.parent)} else {
            self.entries.iter().find(|p|Some(p.candidate.raw_node_id)==node.parent).map(|p|p.attempt_id)
        };
        let span=&candidate.matched.matched.span.mapped;
        if expected_parent.is_none() || expected_parent!=*parent_attempt_id
            || super::super::body_source::original_extent(source,span.virtual_range,|_,_|Ok(())).is_err()
            || source.map_range(span.virtual_range).ok().as_ref()!=Some(span)
            || !before.calls.is_prefix_of(&hooks.calls)
            || hooks.calls.len().saturating_sub(before.calls.len())>MAX_BODY_CALLS {return false;}
        // Bound before cloning candidates or retaining any witness. All owned
        // proposals remain resource-free; failed attempts are dropped on cold.
        let mut size=BoundedSize(self.bytes);
        if write!(&mut size,"{candidate:?}").is_err() {return false;}
        let calls=hooks.calls.suffix(before.calls.len());
        let Some(updates)=hooks.effects.context_updates.get(before.effects.context_updates.len()..) else {return false;};
        let mut resources=Vec::new();
        let Some((candidate,metadata))=PortableStructureBody::freeze_preparation(candidate,&mut |kind,value| {
            if self.resources.len()+resources.len()>=MAX_RESOURCES
                || !consumed_resource(hooks,&calls,updates,kind,value) {return false;}
            resources.push(Resource {owner:candidate.raw_node_id,kind,identity:value.identity()});true
        }) else {return false;};
        if write!(&mut size,"{metadata:?}").is_err() {return false;}
        size.0=size.0.saturating_add(resources.len()*std::mem::size_of::<Resource>());
        if size.0>MAX_BODY_BYTES {return false;}
        self.bytes=size.0;
        self.entries.push(ReusedSectionPreparation {candidate,attempt_id:*attempt_id,parent_attempt_id:*parent_attempt_id});
        self.metadata.extend(metadata);self.resources.extend(resources);
        true
    }

    pub(super) fn validate_current(&self,body:&skript_parser::StructureBody,
        consumed:&mut impl FnMut(ConsumedBodyResource,&ExpressionEffects)->bool)->bool {
        let skript_parser::StructureBody::Trigger(nodes)=body else {return false;};
        let mut pending=nodes.iter().rev().collect::<Vec<_>>();let mut index=0;
        let mut covered=vec![false;self.resources.len()];
        while let Some(node)=pending.pop() {
            if let SectionBodyNode::Section(matches)=node {
                let Some(candidate)=matches.selected.as_ref() else {return false;};
                let Some(preparation)=self.entries.get(index) else {return false;};index+=1;
                if preparation.candidate.raw_node_id!=candidate.raw_node_id {return false;}
                pending.extend(candidate.body.iter().rev());
                // Compare only the current header's resources. Nested bodies
                // are certified independently by the normal portable freeze.
                let mut header=candidate.clone();header.body.clear();
                if PortableStructureBody::freeze_preparation(&header,&mut |kind,value| {
                    let found=self.resources.iter().enumerate().find(|(i,r)|!covered[*i]
                        && r.owner==candidate.raw_node_id && r.kind==kind && r.identity.matches(value));
                    let Some((i,_))=found else {return false;};
                    if !consumed(kind,value) {return false;}
                    covered[i]=true;true
                }).is_none() {return false;}
            }
        }
        index==self.entries.len() && covered.into_iter().all(|value|value)
    }
}

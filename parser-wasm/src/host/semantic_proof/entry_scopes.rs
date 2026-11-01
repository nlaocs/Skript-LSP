//! Native entry-scope inputs for explicitly selected composed facts.
//! Public refs are never permissions without their live private native frame.
use super::*;
use std::sync::atomic::AtomicU64;
use crate::host::{ExpressionParseContext,ParserHost};
use crate::host::project_declarations::ProviderStamp;
use crate::bindings::nlaocs::skript_parser_addon::types::{EffectPayload,StructurePayload,StructureBodyMode,StructureTiming};

pub(in crate::host) const INPUT:&str="parser.native-entry-scope-input.v1";
pub(in crate::host) const CAPABILITY:&str=crate::CAPABILITY_SEMANTIC_ENTRY_SCOPES;
const MAX_HEADERS:usize=256;
const MAX_FRAMES:usize=256;
const MAX_INPUTS:usize=1024;
const MAX_CHAIN:usize=32;
const MAX_BYTES:usize=2*1024*1024;
static NEXT:AtomicU64=AtomicU64::new(1);
fn next()->Option<u64>{NEXT.fetch_update(Ordering::Relaxed,Ordering::Relaxed,|n|n.checked_add(1)).ok()}

#[derive(Clone,Debug)]
pub(in crate::host) struct Operation {pub source:[u8;32],pub frames:Vec<u64>}
#[derive(Debug)]
struct Header {
    id:u64,source:[u8;32],root:u64,owner:String,span:crate::host::MappedSpan,
    definition:String,registration:String,pattern:u64,plans:skript_parser::StructureEntryPlans,
    context:ExpressionParseContext,provider:ProviderStamp,enter_receipts:BTreeSet<u64>,exit_receipts:Option<BTreeSet<u64>>,
}
#[derive(Debug)]
struct Frame {
    header:u64,root:u64,lease:String,entry:u64,index:usize,entry_span:crate::host::MappedSpan,
    body_span:crate::host::MappedSpan,context:[u8;32],ended:bool,completed:bool,bound:Option<Bound>,
}
#[derive(Clone,Debug)]
struct Bound {root_span:crate::host::MappedSpan,root_graph:[u8;32],entry_span:crate::host::MappedSpan}
#[derive(Debug)]
struct Input {consumer:String,source:[u8;32],frames:Vec<u64>,operation:[u8;32],span:crate::host::MappedSpan,definition:String,registration:String,pattern:u64}
#[derive(Debug,Default)]
pub(in crate::host) struct Registry {
    headers:BTreeMap<(u64,String),Header>,frames:BTreeMap<u64,Frame>,inputs:BTreeMap<String,Input>,
    bytes:usize,sealed:bool,exhausted:bool,
}
fn effect_signature(value:&EffectPayload)->Option<[u8;32]> {
    let c=value.candidate.as_ref()?;
    Some(debug_digest("entry-scope-operation-v1",&(&value.context,&c.span,&c.definition_id,&c.registration_id,c.pattern_index)))
}
fn exact(span:&crate::host::MappedSpan)->bool {
    span.virtual_range.start<span.virtual_range.end&&span.origins.len()==1
        &&span.origins[0].kind==crate::host::WitOriginKind::Exact&&span.origins[0].expansion.is_none()
}
fn within(inner:&crate::host::MappedSpan,outer:&crate::host::MappedSpan)->bool {
    exact(inner)&&exact(outer)&&inner.virtual_range.start>=outer.virtual_range.start
        &&inner.virtual_range.end<=outer.virtual_range.end
        &&inner.origins[0].original_range.start>=outer.origins[0].original_range.start
        &&inner.origins[0].original_range.end<=outer.origins[0].original_range.end
}
fn receipt_roots(metadata:&[WitMetadataEntry])->Option<BTreeSet<u64>> {
    let mut roots=BTreeSet::new();metadata_receipts(metadata,&mut roots).then_some(())?;
    (!roots.is_empty()).then_some(roots)
}
struct BudgetSize(usize);
impl fmt::Write for BudgetSize {fn write_str(&mut self,s:&str)->fmt::Result {
    self.0=self.0.saturating_add(s.len());if self.0>MAX_BYTES{Err(fmt::Error)}else{Ok(())}
}}
fn size(value:&impl fmt::Debug)->Option<usize>{let mut n=BudgetSize(0);write!(&mut n,"{value:?}").ok()?;Some(n.0)}
impl Registry {
    fn charge(&mut self,bytes:usize)->bool {
        self.bytes=self.bytes.saturating_add(bytes);
        if self.bytes>MAX_BYTES {self.exhausted=true;}
        !self.exhausted&&!self.sealed
    }
    pub(in crate::host) fn header_current(&self,root:u64,lease:&str,host:&ParserHost)->bool {
        self.headers.get(&(root,lease.into())).is_some_and(|h|h.provider.is_current(host))
    }
    pub(in crate::host) fn operation_current(&self,operation:&Operation,host:&ParserHost)->bool {
        !operation.frames.is_empty()&&operation.frames.len()<=MAX_CHAIN&&operation.frames.iter().all(|id|
            self.frames.get(id).is_some_and(|f|!f.ended&&self.headers.get(&(f.root,f.lease.clone()))
                .is_some_and(|h|h.id==f.header&&h.source==operation.source&&h.provider.is_current(host))))
    }
    pub(in crate::host) fn register_header(&mut self,source:[u8;32],payload:&StructurePayload,
        context:ExpressionParseContext,provider:ProviderStamp,opted_owners:&BTreeSet<String>) {
        let root=payload.candidate.raw_node_id;
        // Header alternatives have separate fresh native leases. A later
        // discarded alternative must not erase the selected candidate's proof.
        if self.sealed||self.exhausted||payload.timing!=StructureTiming::EnterBody
            ||payload.candidate.body_mode!=StructureBodyMode::Entries||!payload.candidate.declarations.is_empty()
            ||self.headers.len()>=MAX_HEADERS||!exact(&payload.candidate.span){return;}
        let Ok(metadata)=crate::host::metadata_entries(payload.candidate.metadata.clone()) else{return;};
        let Ok(plans)=skript_parser::decode_structure_entry_plans(&metadata,skript_parser::RawNodeId::new(root)) else{return;};
        let Some(owner)=plans.owner.clone() else{return;};
        if !opted_owners.contains(&owner){return;}
        let Some(lease)=plans.native_lease.clone()else{return;};
        let Some(enter_receipts)=receipt_roots(&payload.candidate.metadata)else{return;};
        if plans.entries.is_empty()||self.headers.contains_key(&(root,lease.clone())){return;}
        let Some(id)=next()else{self.exhausted=true;return;};
        let Some(bytes)=size(&(&plans,&context,&provider.semantic_key(),&payload.candidate.span,&payload.candidate.definition_id,&payload.candidate.registration_id,&enter_receipts))else{self.exhausted=true;return;};
        if !self.charge(bytes.saturating_add(512)){return;}
        self.headers.insert((root,lease),Header {id,source,root,owner,span:payload.candidate.span.clone(),
            definition:payload.candidate.definition_id.clone(),registration:payload.candidate.registration_id.clone(),
            pattern:payload.candidate.pattern_index,plans,context,provider,enter_receipts,exit_receipts:None});
    }
    pub(in crate::host) fn begin(&mut self,source:[u8;32],request:&skript_parser::StructureEntryScopeRequest<'_>,world_current:bool)->Option<u64> {
        if self.sealed||self.exhausted||!world_current||self.frames.len()>=MAX_FRAMES{return None;}
        let header=self.headers.get(&(request.root.get(),request.native_lease.into()))?;
        if header.source!=source||header.owner!=request.owner||header.root!=request.root.get(){return None;}
        let plan=header.plans.entries.get(&request.entry)?;
        if plan.declaration_index!=request.declaration_index||plan.mode.parser_mode()!=Some(request.mode)
            ||debug_digest("entry-context",&plan.context.apply(&header.context))!=debug_digest("entry-context",request.context){return None;}
        let node=request.tree.get(request.entry)?;
        if node.parent!=Some(request.root)||node.kind!=skript_parser::RawNodeKind::Section{return None;}
        let body_span=crate::host::mapped_span_to_wit(node.body_span.clone()?);
        let entry_span=crate::host::mapped_span_to_wit(request.entry_span.mapped.clone());
        if !exact(&body_span)||!exact(&entry_span){return None;}
        let (header_id,root)=(header.id,header.root);
        let context=debug_digest("entry-context",request.context);
        let id=next()?;let bytes=size(&(&body_span,&entry_span,request.native_lease))?;
        if !self.charge(bytes.saturating_add(512)){return None;}
        self.frames.insert(id,Frame {header:header_id,root,lease:request.native_lease.into(),entry:request.entry.get(),index:request.declaration_index,
            entry_span,body_span,context,ended:false,completed:false,bound:None});Some(id)
    }
    pub(in crate::host) fn finish(&mut self,id:u64,completed:bool) {
        if let Some(frame)=self.frames.get_mut(&id){frame.ended=true;frame.completed=completed;}
    }
    pub(in crate::host) fn discard(&mut self,root:u64,lease:&str) {self.headers.remove(&(root,lease.into()));}
    pub(in crate::host) fn exit_header(&mut self,payload:&StructurePayload) {
        let Ok(metadata)=crate::host::metadata_entries(payload.candidate.metadata.clone()) else{return;};
        let Ok(plans)=skript_parser::decode_structure_entry_plans(&metadata,skript_parser::RawNodeId::new(payload.candidate.raw_node_id))else{return;};
        let Some(lease)=plans.native_lease.as_deref()else{return;};
        let Some(header)=self.headers.get_mut(&(payload.candidate.raw_node_id,lease.into())) else{return;};
        if payload.timing!=StructureTiming::ExitBody||payload.candidate.definition_id!=header.definition
            ||payload.candidate.registration_id!=header.registration||payload.candidate.pattern_index!=header.pattern{return;}
        if plans!=header.plans{return;}
        header.exit_receipts=receipt_roots(&payload.candidate.metadata);
    }
    pub(in crate::host) fn issue(&mut self,operation:&Operation,consumer:&str,payload:&mut EffectPayload)->Option<()> {
        if self.sealed||self.exhausted||self.inputs.len()>=MAX_INPUTS||operation.frames.is_empty()||operation.frames.len()>MAX_CHAIN{return None;}
        let candidate=payload.candidate.as_ref()?;let signature=effect_signature(payload)?;
        for id in &operation.frames {
            let frame=self.frames.get(id)?;let header=self.headers.get(&(frame.root,frame.lease.clone()))?;
            if frame.ended||header.id!=frame.header||header.source!=operation.source||!within(&candidate.span,&frame.body_span){return None;}
        }
        let (span,definition,registration,pattern)=(candidate.span.clone(),candidate.definition_id.clone(),candidate.registration_id.clone(),candidate.pattern_index);
        let bytes=size(&(&span,&definition,&registration,&operation.frames,consumer))?;
        let reference=format!("entry-input-{}",next()?);if !self.charge(bytes.saturating_add(512)){return None;}
        self.inputs.insert(reference.clone(),Input {consumer:consumer.into(),source:operation.source,frames:operation.frames.clone(),operation:signature,span,definition,registration,pattern});
        let metadata=&mut payload.candidate.as_mut()?.metadata;
        metadata.retain(|entry|entry.key!=INPUT);
        metadata.push(WitMetadataEntry {key:INPUT.into(),owner_component_id:None,
            value:serde_json::json!({"version":1,"reference":reference}).to_string()});Some(())
    }
    pub(in crate::host) fn claim_current(&self,reference:&str,consumer:&str,payload:&EffectPayload,operation:&Operation)->bool {
        if self.sealed||self.exhausted{return false;}
        let Some(input)=self.inputs.get(reference)else{return false;};
        input.consumer==consumer&&input.source==operation.source&&input.frames==operation.frames
            &&effect_signature(payload)==Some(input.operation)
            &&input.frames.iter().all(|id|self.frames.get(id).is_some_and(|f|!f.ended
                &&self.headers.get(&(f.root,f.lease.clone())).is_some_and(|h|h.id==f.header&&h.source==input.source)))
    }
}
pub(in crate::host) fn strip_payload(payload:&mut HookPayload) {
    if let HookPayload::Effect(effect)=payload && let Some(candidate)=&mut effect.candidate {
        candidate.metadata.retain(|entry|entry.key!=INPUT);
    }
}
pub(in crate::host) fn strip_input(output:&mut HookOutput) {
    if let Some(HookPayload::Effect(payload))=&mut output.replacement {
        if let Some(candidate)=&mut payload.candidate {candidate.metadata.retain(|entry|entry.key!=INPUT);}
    }
}

pub(in crate::host) fn source_identity(source:&skript_parser::MappedSource)->[u8;32] {
    debug_digest("entry-scope-source-v1",source)
}
fn same_span(a:&crate::host::MappedSpan,b:&crate::host::MappedSpan)->bool {debug_digest("scope-span",a)==debug_digest("scope-span",b)}
fn region(tree:&AstTree,root:u64)->Option<(AstTree,BTreeSet<u64>)> {
    let nodes=tree.nodes.iter().map(|n|(n.id,n)).collect::<BTreeMap<_,_>>();
    let mut ids=BTreeSet::new();let mut pending=vec![root];
    struct Size(usize);impl fmt::Write for Size {fn write_str(&mut self,s:&str)->fmt::Result {self.0=self.0.saturating_add(s.len());if self.0>128*1024{Err(fmt::Error)}else{Ok(())}}}
    let mut size=Size(0);
    while let Some(id)=pending.pop(){if !ids.insert(id){continue;}if ids.len()>256{return None;}
        let node=*nodes.get(&id)?;write!(&mut size,"{node:?}").ok()?;pending.extend(references(node));}
    Some((AstTree {roots:vec![root],nodes:ids.iter().map(|id|(*nodes[id]).clone()).collect()},ids))
}
fn graph(tree:&AstTree,root:u64)->Option<[u8;32]> {Some(debug_digest("native-entry-graph-v1",&canonical(&region(tree,root)?.0,false)?.tree))}
fn receipt_chain(roots:&BTreeSet<u64>,enter:&BTreeSet<u64>,receipts:&BTreeMap<u64,Receipt>,retained:&BTreeSet<u64>)->bool {
    let mut seen=BTreeSet::new();let mut pending=roots.iter().copied().collect::<Vec<_>>();
    while let Some(id)=pending.pop(){if !seen.insert(id){continue;}let Some(r)=receipts.get(&id)else{return false;};
        if seen.len()>4096||!r.valid||r.unbound||!retained.contains(&id)||r.parents.iter().any(|p|*p>=id){return false;}
        pending.extend(r.parents.iter().copied());}
    enter.is_subset(&seen)
}
impl Registry {
    pub(in crate::host) fn current_headers(&self,host:&ParserHost)->BTreeSet<u64> {
        self.headers.values().filter_map(|h|h.provider.is_current(host).then_some(h.id)).collect()
    }
    /// Called once from the native document projection, before any mutable AST hook.
    /// This settles only real selected candidates whose native scopes already exist.
    pub(in crate::host) fn bind_native(&mut self,source:[u8;32],syntax:&skript_parser::StructureDocument,
        tree:&AstTree,current_headers:&BTreeSet<u64>) {
        if self.sealed||self.exhausted{return;}self.sealed=true;
        for item in &syntax.roots {
            let skript_parser::StructureDocumentNode::Structure(matches)=item else{continue;};
            let Some(candidate)=matches.selected.as_ref().filter(|c|!c.entry_validation_failed)else{continue;};
            let root=candidate.raw_node_id.get();
            let Ok(plans)=skript_parser::decode_structure_entry_plans(&candidate.metadata,candidate.raw_node_id)else{continue;};
            let Some(lease)=plans.native_lease.as_deref()else{continue;};
            let Some(h)=self.headers.get(&(root,lease.into()))else{continue;};
            if !current_headers.contains(&h.id)||h.source!=source||h.registration!=candidate.matched.registration_id
                ||h.definition!=candidate.matched.definition_id||h.pattern!=candidate.matched.pattern_index as u64 {continue;}
            let Some(exit)=&h.exit_receipts else{continue;};
            if receipt_roots(&crate::host::metadata_to_wit(&candidate.metadata)).as_ref()!=Some(exit){continue;}
            if skript_parser::decode_structure_entry_plans(&candidate.metadata,candidate.raw_node_id).ok().as_ref()!=Some(&h.plans){continue;}
            let skript_parser::StructureBody::Entries(entries)=&candidate.body else{continue;};
            let mut ast_roots=tree.roots.iter().filter_map(|id|tree.nodes.iter().find(|n|n.id==*id))
                .filter(|n|n.kind==crate::host::SyntaxKind::Structure&&same_span(&n.span,&h.span)
                    &&n.summary.as_ref().is_some_and(|s|s.definition_id.as_deref()==Some(h.definition.as_str())
                        &&s.registration_id.as_deref()==Some(h.registration.as_str())&&s.pattern_index==Some(h.pattern)));
            let Some(ast_root)=ast_roots.next()else{continue;};if ast_roots.next().is_some(){continue;}
            let Some(digest)=graph(tree,ast_root.id)else{continue;};
            let Some((root_tree,_))=region(tree,ast_root.id)else{continue;};
            for f in self.frames.values_mut().filter(|f|f.root==root&&f.header==h.id&&f.ended&&f.completed) {
                let Some(entry)=entries.iter().find(|e|e.raw_node_id.is_some_and(|id|id.get()==f.entry)
                    &&same_span(&crate::host::mapped_span_to_wit(e.span.mapped.clone()),&f.entry_span)
                    &&matches!(e.value,skript_parser::StructureEntryValue::Trigger(_)))else{continue;};
                if !h.plans.entries.iter().any(|(id,p)|id.get()==f.entry&&p.declaration_index==f.index&&debug_digest("entry-context",&p.context.apply(&h.context))==f.context){continue;}
                let matching=root_tree.nodes.iter().filter(|n|n.syntax_id.starts_with("parser:structure-entry.")
                    &&same_span(&n.span,&f.entry_span)&&n.metadata.iter().any(|m|m.key=="parser.entry-key"&&m.value==entry.key)).count();
                if matching!=1{continue;}
                f.bound=Some(Bound {root_span:h.span.clone(),root_graph:digest,entry_span:f.entry_span.clone()});
            }
        }
    }
}

#[derive(Default)]
pub(super) struct Witnesses {inputs:BTreeMap<String,Authorization>,reverse:BTreeMap<u64,Vec<u64>>}
struct Authorization {consumer:String,operation:u64,headers:BTreeMap<u64,String>}
impl Registry {
    pub(super) fn witnesses(&self,tree:&AstTree,receipts:&BTreeMap<u64,Receipt>,retained:&BTreeSet<u64>)->Witnesses {
        let mut result=Witnesses::default();
        if !self.sealed||self.exhausted{return result;}
        let nodes=tree.nodes.iter().map(|n|(n.id,n)).collect::<BTreeMap<_,_>>();
        for n in &tree.nodes {for child in references(n){result.reverse.entry(child).or_default().push(n.id);}}
        let mut frame_views=BTreeMap::<u64,(BTreeSet<u64>,u64,String)>::new();
        for (id,f) in &self.frames {
            let Some(b)=&f.bound else{continue;};let Some(h)=self.headers.get(&(f.root,f.lease.clone())).filter(|h|h.id==f.header)else{continue;};
            let Some(exit)=&h.exit_receipts else{continue;};if !receipt_chain(exit,&h.enter_receipts,receipts,retained){continue;}
            let mut roots=tree.roots.iter().filter_map(|id|nodes.get(id).copied()).filter(|n|same_span(&n.span,&b.root_span)
                &&n.summary.as_ref().is_some_and(|s|s.definition_id.as_deref()==Some(h.definition.as_str())&&s.registration_id.as_deref()==Some(h.registration.as_str())&&s.pattern_index==Some(h.pattern)));
            let Some(root)=roots.next()else{continue;};if roots.next().is_some()||graph(tree,root.id)!=Some(b.root_graph){continue;}
            let Some((root_tree,_))=region(tree,root.id)else{continue;};
            let mut entries=root_tree.nodes.iter().filter(|n|n.syntax_id.starts_with("parser:structure-entry.")&&same_span(&n.span,&b.entry_span));
            let Some(entry)=entries.next()else{continue;};if entries.next().is_some(){continue;}
            let Some((_,body_ids))=region(tree,entry.id)else{continue;};
            frame_views.insert(*id,(body_ids,root.id,h.owner.clone()));
        }
        for (reference,input) in &self.inputs {
            let mut operations=tree.nodes.iter().filter(|n|n.kind==crate::host::SyntaxKind::Effect&&same_span(&n.span,&input.span)
                &&n.summary.as_ref().is_some_and(|s|s.definition_id.as_deref()==Some(input.definition.as_str())&&s.registration_id.as_deref()==Some(input.registration.as_str())&&s.pattern_index==Some(input.pattern)));
            let Some(operation)=operations.next()else{continue;};if operations.next().is_some(){continue;}
            let mut headers=BTreeMap::new();let mut valid=true;
            for frame in &input.frames {let Some((body,root,owner))=frame_views.get(frame)else{valid=false;break;};
                if !body.contains(&operation.id){valid=false;break;}headers.insert(*root,owner.clone());}
            if valid&&!headers.is_empty(){result.inputs.insert(reference.clone(),Authorization {consumer:input.consumer.clone(),operation:operation.id,headers});}
        }
        result
    }
}
impl Witnesses {
    pub(super) fn authorizes(&self,reference:&str,node:u64,consumer:&str,own:&BTreeMap<u64,BTreeSet<String>>)->bool {
        let Some(input)=self.inputs.get(reference).filter(|input|input.consumer==consumer&&input.operation==node)else{return false;};
        let mut pending=vec![node];let mut seen=BTreeSet::new();
        while let Some(id)=pending.pop(){if !seen.insert(id){continue;}let Some(writers)=own.get(&id)else{return false;};
            if seen.len()>8192||writers.iter().any(|writer|writer!=consumer&&input.headers.get(&id)!=Some(writer)){return false;}
            pending.extend(self.reverse.get(&id).into_iter().flatten().copied());}
        true
    }
}

#[cfg(test)]mod tests;

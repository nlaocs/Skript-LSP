//! Selected intrinsic contracts of a current accepted operation. This does not
//! certify its operands, body, runtime value or unrelated writer effects.
use super::*;
use serde::Deserialize;
pub(in crate::host) const DRAFT:&str="parser.semantic.intrinsic-facts.v1";
const MAX_RECORDS:usize=2048;
const MAX_BYTES:usize=2*1024*1024;
#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Draft {version:u32,metadata_keys:Vec<String>,#[serde(default)] inputs:Vec<Input>}
#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Input {capture:u64,metadata_keys:Vec<String>,public_schemas:Vec<String>}
#[derive(Debug)]
struct Child {capture:u64,span:super::super::MappedSpan,value:super::super::WitRegisteredExpressionChild,
    issuer:String,token:u64,metadata:Vec<WitMetadataEntry>,schemas:Vec<String>}
#[derive(Debug)]
struct Intrinsic {kind:super::super::SyntaxKind,span:super::super::MappedSpan,definition:String,registration:String,
    pattern:u64,issuer:String,metadata:Vec<WitMetadataEntry>,inputs:Vec<Child>}
#[derive(Debug,Default)]
pub(in crate::host) struct Registry {entries:BTreeMap<u64,Intrinsic>,bytes:usize,exhausted:bool}
fn selected(metadata:&[WitMetadataEntry],keys:&[String],issuer:&str)->Option<Vec<WitMetadataEntry>> {
    if keys.is_empty()||keys.len()>16||keys.iter().collect::<BTreeSet<_>>().len()!=keys.len(){return None;}
    keys.iter().map(|key| {
        if key.is_empty()||key.len()>256||key.starts_with("parser.")&&key!=super::super::semantic_transfer::METADATA{return None;}
        let mut entries=metadata.iter().filter(|m|m.key==*key);let value=entries.next()?;
        (entries.next().is_none()&&value.owner_component_id.as_deref()==Some(issuer)).then(||value.clone())
    }).collect()
}
// Only this subject's dispatch ancestry is checked. Admitted nested calls are
// different subjects, whose selected contracts need their own authentication.
fn direct(registry:&ReceiptRegistry,token:u64,issuer:&str,retained:&BTreeSet<u64>)->bool {
    let mut seen=BTreeSet::new();let mut pending=vec![token];
    while let Some(id)=pending.pop() {
        if !seen.insert(id){continue;}
        let Some(receipt)=registry.receipts.get(&id)else{return false;};
        if seen.len()>4096||!receipt.valid||!retained.contains(&id)
            ||(receipt.payload_writer||receipt.unbound)&&receipt.issuer!=issuer
            ||receipt.parents.iter().any(|p|*p>=id){return false;}
        pending.extend(receipt.parents.iter().copied());
    }true
}
pub(super) fn same_entries(current:&[WitMetadataEntry],selected:&[WitMetadataEntry])->bool {
    selected.iter().all(|value| {
        let mut entries=current.iter().filter(|m|m.key==value.key);
        entries.next().is_some_and(|m|m.value==value.value&&m.owner_component_id==value.owner_component_id)&&entries.next().is_none()
    })
}
fn root_receipt(metadata:&[WitMetadataEntry])->Option<u64> {
    let mut roots=BTreeSet::new();metadata_receipts(metadata,&mut roots).then_some(())?;
    (roots.len()==1).then(||*roots.first().unwrap())
}
impl Registry {
    pub(super) fn record(&mut self,token:u64,payload:&HookPayload,issuer:&str,receipts:&BTreeMap<u64,Receipt>) {
        if self.exhausted{return;}
        let (kind,span,definition,registration,pattern,metadata)=match payload {
            HookPayload::Condition(p)=>(super::super::SyntaxKind::Condition,&p.candidate.span,&p.candidate.definition_id,&p.candidate.registration_id,p.candidate.pattern_index,&p.candidate.metadata),
            HookPayload::Section(p) if matches!(p.timing,super::super::WitSectionTiming::EnterChildren)=>(super::super::SyntaxKind::Section,&p.candidate.span,&p.candidate.definition_id,&p.candidate.registration_id,p.candidate.pattern_index,&p.candidate.metadata),
            _=>return,
        };
        let mut requests=metadata.iter().filter(|m|m.key==DRAFT);
        let Some(request)=requests.next().filter(|m|m.owner_component_id.as_deref()==Some(issuer))else{return;};
        if requests.next().is_some()||request.value.len()>16*1024{return;}
        let Ok(draft)=serde_json::from_str::<Draft>(&request.value)else{return;};
        if draft.version!=1||draft.inputs.len()>8||draft.inputs.iter().map(|i|i.capture).collect::<BTreeSet<_>>().len()!=draft.inputs.len(){return;}
        let Some(metadata)=selected(metadata,&draft.metadata_keys,issuer)else{return;};
        let mut inputs=Vec::new();
        for input in draft.inputs {
            // Inputs are immutable native child views, not guest-supplied ASTs.
            let HookPayload::Condition(p)=payload else{return;};
            let mut captures=p.candidate.captures.iter().filter_map(|c|match c {
                super::super::WitConditionCapture::Expression(c) if c.capture_index==input.capture=>Some(c),_=>None});
            let Some(capture)=captures.next().filter(|c|c.resolution_id.is_some())else{return;};
            if captures.next().is_some(){return;}
            let mut bindings=p.candidate.expression_bindings.iter().filter(|b|b.capture_index==input.capture);
            let Some(binding)=bindings.next()else{return;};
            if bindings.next().is_some()||p.candidate.expression_bindings.iter().filter(|b|b.expression_index==binding.expression_index).count()!=1{return;}
            let Some(value)=usize::try_from(binding.expression_index).ok().and_then(|i|p.candidate.children.get(i))else{return;};
            let Some(child_token)=root_receipt(&value.metadata)else{return;};
            if receipts.get(&child_token).is_none_or(|r|!r.valid){return;}
            let mut owners=input.metadata_keys.iter().filter_map(|key|value.metadata.iter().find(|m|m.key==*key).and_then(|m|m.owner_component_id.as_deref())).collect::<BTreeSet<_>>();
            if owners.len()!=1{return;}
            let child_issuer=owners.pop_first().unwrap();
            let Some(selected)=selected(&value.metadata,&input.metadata_keys,child_issuer)else{return;};
            if input.public_schemas.len()>8||input.public_schemas.iter().any(|s|s.is_empty()||s.len()>256)
                ||input.public_schemas.iter().collect::<BTreeSet<_>>().len()!=input.public_schemas.len()
                ||input.public_schemas.iter().any(|id|value.public_data.iter().filter(|p|p.schema_id==*id).count()!=1){return;}
            inputs.push(Child {capture:input.capture,span:capture.span.clone(),value:value.clone(),issuer:child_issuer.to_owned(),
                token:child_token,metadata:selected,schemas:input.public_schemas});
        }
        let entry=Intrinsic {kind,span:span.clone(),definition:definition.clone(),registration:registration.clone(),pattern,
            issuer:issuer.into(),metadata,inputs};
        struct Size(usize);impl fmt::Write for Size {fn write_str(&mut self,s:&str)->fmt::Result {self.0=self.0.saturating_add(s.len());if self.0>128*1024{Err(fmt::Error)}else{Ok(())}}}
        let mut size=Size(0);if write!(&mut size,"{entry:?}").is_err(){return;}
        if self.entries.len()>=MAX_RECORDS||self.bytes.saturating_add(size.0)>MAX_BYTES {self.entries.clear();self.exhausted=true;return;}
        self.bytes+=size.0;self.entries.insert(token,entry);
    }
    // The native condition parser runs on request-local input. Only its exact
    // successful/adoptable call set can carry these claims into the caller.
    pub(super) fn rebase(&mut self,request:&super::super::ParseRequest,calls:&[HookCall]) {
        if calls.len()>8192 {self.entries.clear();self.exhausted=true;return;}
        let affine=request.span.origins.len()==1&&request.span.origins[0].kind==super::super::WitOriginKind::Exact
            &&request.span.virtual_range.end.checked_sub(request.span.virtual_range.start)==Some(request.input.len()as u64)
            &&request.span.origins[0].original_range.end.checked_sub(request.span.origins[0].original_range.start)==Some(request.input.len()as u64);
        let map=|span:&super::super::MappedSpan|->Option<super::super::MappedSpan> {
            if !affine||span.virtual_range.start>span.virtual_range.end||span.virtual_range.end>request.input.len()as u64{return None;}
            let native=super::super::parser_mapped_span_from_wit(span).ok()?;
            let local=skript_parser::MatchSpan {local_range:native.virtual_range,mapped:native};
            Some(super::super::nested_span_to_request(&local,request))
        };
        let tokens=calls.iter().filter_map(|c|c.semantic_receipt).collect::<BTreeSet<_>>();
        for token in tokens {
            let Some(entry)=self.entries.get_mut(&token)else{continue;};
            let root=map(&entry.span);let children=entry.inputs.iter().map(|c|map(&c.span)).collect::<Option<Vec<_>>>();
            if let (Some(root),Some(children))=(root,children) {
                entry.span=root;for (child,span) in entry.inputs.iter_mut().zip(children) {child.span=span;}
            }else{self.entries.remove(&token);}
        }
    }
    pub(super) fn has_receipt(&self,token:u64)->bool {self.entries.contains_key(&token)}
    pub(super) fn certify(&self,tree:&AstTree,registry:&ReceiptRegistry,retained:&BTreeSet<u64>)->BTreeMap<u64,value_facts::FactClaims> {
        let mut claims=BTreeMap::<u64,value_facts::FactClaims>::new();if self.exhausted{return claims;}
        let index=tree.nodes.iter().map(|n|(n.id,n)).collect::<BTreeMap<_,_>>();
        for node in &tree.nodes {
            let Some(root)=root_receipt(&node.metadata)else{continue;};
            let mut seen=BTreeSet::new();let mut pending=vec![root];
            while let Some(token)=pending.pop() {
                if !seen.insert(token)||seen.len()>4096{continue;}
                let Some(receipt)=registry.receipts.get(&token).filter(|r|r.valid)else{continue;};
                pending.extend(receipt.parents.iter().filter(|p|**p<token).copied());
                let Some(entry)=self.entries.get(&token)else{continue;};
                if !direct(registry,root,&entry.issuer,retained)||!entry.matches(node)||!same_entries(&node.metadata,&entry.metadata){continue;}
                // Header geometry and every selected child remain native/current.
                let mut verified=Vec::new();
                for input in &entry.inputs {
                    let Some(child)=capture(node,input.capture,&index)else{break;};
                    if !input.matches(child)||!direct(registry,input.token,&input.issuer,retained)
                        ||root_receipt(&child.metadata)!=Some(input.token)||!same_entries(&child.metadata,&input.metadata){break;}
                    verified.push((child.id,input));
                }
                if verified.len()!=entry.inputs.len(){continue;}
                claims.entry(node.id).or_default().metadata.extend(entry.metadata.iter().map(|m|(entry.issuer.clone(),m.key.clone())));
                for (id,input) in verified {
                    let claim=claims.entry(id).or_default();
                    claim.metadata.extend(input.metadata.iter().map(|m|(input.issuer.clone(),m.key.clone())));
                    claim.public_data.extend(input.schemas.iter().map(|s|(input.issuer.clone(),s.clone())));
                }
            }
        }claims
    }
}
fn capture<'a>(parent:&AstNode,capture:u64,index:&BTreeMap<u64,&'a AstNode>)->Option<&'a AstNode> {
    if !super::super::source_geometry::pattern_capture_geometry_is_current(parent){return None;}
    let prefix=format!("{capture}:");let mut ids=parent.captures.iter().filter(|c|c.name.starts_with(&prefix));
    let CaptureValue::Node(id)=ids.next()?.value else{return None;};
    if ids.next().is_some()||!parent.children.contains(&id){return None;}
    index.get(&id).copied()
}
impl Intrinsic {
    fn matches(&self,node:&AstNode)->bool {
        node.kind==self.kind&&debug_digest("intrinsic-span",&node.span)==debug_digest("intrinsic-span",&self.span)
            &&node.summary.as_ref().is_some_and(|s|s.definition_id.as_ref()==Some(&self.definition)
                &&s.registration_id.as_ref()==Some(&self.registration)&&s.pattern_index==Some(self.pattern))
    }
}
impl Child {
    fn matches(&self,node:&AstNode)->bool {
        let Some(s)=&node.summary else{return false;};let v=&self.value;
        self.span.virtual_range.start<=node.span.virtual_range.start&&node.span.virtual_range.end<=self.span.virtual_range.end
            &&node.span.virtual_range.start<node.span.virtual_range.end
            &&node.span.virtual_range.end-node.span.virtual_range.start==v.text.len() as u64
            &&node.text==v.text&&s.kind==v.kind&&s.definition_id==v.definition_id&&s.registration_id==v.registration_id
            &&s.pattern_index==v.pattern_index&&s.element_class==v.element_class&&s.return_type==v.return_type
            &&s.possible_return_types==v.possible_return_types&&format!("{:?}",s.possible_return_types_state)==format!("{:?}",v.possible_return_types_state)
            &&format!("{:?}",s.multiplicity)==format!("{:?}",v.multiplicity)&&format!("{:?}",s.public_data)==format!("{:?}",v.public_data)
            &&v.parser_id.as_ref().is_none_or(|p|node.syntax_id==format!("parser:expression.{p}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{AstContextOrigin,WitPossibleReturnTypesState};
    fn fixture(observer:bool)->(ReceiptRegistry,AstTree,Vec<HookCall>) {
        let mut registry=ReceiptRegistry::default();let source=skript_parser::MappedSource::identity("xyz");
        let mut child=super::super::tests::node(1);child.text="y".into();child.syntax_id="parser:expression.fixture-variable".into();
        child.span=super::super::super::mapped_span_to_wit(source.map_range(skript_parser::TextRange::new(1,2)).unwrap());
        child.summary=Some(super::super::super::WitParseSummary {kind:"variable".into(),definition_id:None,registration_id:None,element_class:None,pattern_index:None,
            return_type:Some("java.lang.String".into()),possible_return_types:vec![],possible_return_types_state:WitPossibleReturnTypesState::Complete,
            multiplicity:None,default_expression:None,public_data:vec![super::super::super::WitExpressionPublicData {schema_id:"shared.identity".into(),schema_version:1,json:"{}".into()}],metadata:vec![]});
        let owned=WitMetadataEntry {key:"owned.read".into(),value:"current".into(),owner_component_id:Some("value-owner".into())};child.metadata.push(owned.clone());
        let mut calls=vec![super::super::tests::write(&mut registry,&mut child,"value-owner")];
        if observer {
            let mut payload=super::super::tests::payload();if let HookPayload::Matching(p)=&mut payload {p.metadata=child.metadata.clone();}
            let mut writers=registry.begin(&payload);let token=registry.accept(&mut writers,&mut payload,"private-observer",HookMode::Observe,SemanticEffects::Local);
            if let HookPayload::Matching(p)=payload {child.metadata=p.metadata;}
            calls.push(HookCall {origin:super::super::super::HookCallOrigin::Executed,component_id:"private-observer".into(),subscription_id:"test".into(),semantic_receipt:token});
        }
        let child_token=root_receipt(&child.metadata).unwrap();let summary=child.summary.as_ref().unwrap();
        let input=Child {capture:0,span:child.span.clone(),value:super::super::super::WitRegisteredExpressionChild {
            text:child.text.clone(),kind:"variable".into(),parser_id:Some("fixture-variable".into()),definition_id:None,registration_id:None,pattern_index:None,element_class:None,
            return_type:summary.return_type.clone(),possible_return_types:vec![],possible_return_types_state:WitPossibleReturnTypesState::Complete,
            multiplicity:None,default_expression:None,public_data:summary.public_data.clone(),metadata:child.metadata.clone()},
            issuer:"value-owner".into(),token:child_token,metadata:vec![owned],schemas:vec!["shared.identity".into()]};
        let mut parent=super::super::tests::node(0);parent.kind=super::super::super::SyntaxKind::Condition;parent.text="xyz".into();
        parent.span=super::super::super::mapped_span_to_wit(source.map_range(skript_parser::TextRange::new(0,3)).unwrap());
        parent.context_origin=AstContextOrigin::Preserved;
        parent.summary=Some(super::super::super::WitParseSummary {kind:"condition".into(),definition_id:Some("guard".into()),registration_id:Some("guard.0".into()),
            element_class:None,pattern_index:Some(0),return_type:None,possible_return_types:vec![],possible_return_types_state:WitPossibleReturnTypesState::Complete,
            multiplicity:None,default_expression:None,public_data:vec![],metadata:vec![]});
        parent.children=vec![child.id];parent.captures.push(super::super::super::Capture {name:"0:fixture".into(),value:CaptureValue::Node(child.id)});
        let native=skript_parser::PatternCapture::Regex {capture_index:0,pattern_span:syntax_pattern_parser::syntax::Span::new(0,0),value:"y".into(),
            span:skript_parser::MatchSpan {local_range:skript_parser::TextRange::new(1,2),mapped:source.map_range(skript_parser::TextRange::new(1,2)).unwrap()},groups:vec![]};
        super::super::super::source_geometry::retain_pattern_captures(&mut parent,&[native]);
        let contract=WitMetadataEntry {key:"owned.guard".into(),value:"selected".into(),owner_component_id:Some("guard-owner".into())};parent.metadata.push(contract.clone());
        let call=super::super::tests::write(&mut registry,&mut parent,"guard-owner");let token=call.semantic_receipt.unwrap();calls.push(call);
        registry.intrinsic_facts.entries.insert(token,Intrinsic {kind:parent.kind,span:parent.span.clone(),definition:"guard".into(),registration:"guard.0".into(),pattern:0,
            issuer:"guard-owner".into(),metadata:vec![contract],inputs:vec![input]});
        (registry,AstTree {roots:vec![0],nodes:vec![parent,child]},calls)
    }
    #[test]
    fn intrinsic_claims_keep_actual_child_owner_and_do_not_grant_whole_node_authorship() {
        for observer in [false,true] {
            let (r,tree,calls)=fixture(observer);let retained=calls.iter().filter_map(|c|c.semantic_receipt).collect();
            let claims=r.intrinsic_facts.certify(&tree,&r,&retained);assert!(claims[&0].metadata.contains(&("guard-owner".into(),"owned.guard".into())));
            assert!(claims[&1].metadata.contains(&("value-owner".into(),"owned.read".into())));
            assert!(!claims[&1].metadata.contains(&("guard-owner".into(),"owned.read".into())));
            let proof=SemanticProofRegistry::capture(&tree,Some(&r),&calls);let view=proof.view(&tree);
            assert!(view.unique_issuer(&tree.nodes[0]).is_none());
            assert!(view.composed_metadata_valid(&tree.nodes[0],tree.nodes[0].metadata.iter().find(|m|m.key=="owned.guard").unwrap()));
        }
    }
    #[test]
    fn successful_request_adoption_rebases_intrinsic_spans_once_and_rejects_nonaffine_origins() {
        use crate::host::{ParseRequest,MappedSpan,WitTextRange,WitSourceOrigin,WitOriginKind};
        let (mut registry,_,calls)=fixture(false);
        let mut request=ParseRequest {request_id:1,parser_id:"host.condition".into(),input:"xyz".into(),expected_types:vec![],options:vec![],
            span:MappedSpan {virtual_range:WitTextRange {start:10,end:13},origins:vec![WitSourceOrigin {
                original_range:WitTextRange {start:40,end:43},kind:WitOriginKind::Exact,expansion:None}]}};
        let mut duplicate=calls.clone();duplicate.extend(calls.clone());
        registry.intrinsic_facts.rebase(&request,&duplicate);
        let entry=registry.intrinsic_facts.entries.values().next().unwrap();
        assert_eq!((entry.span.virtual_range.start,entry.span.virtual_range.end),(10,13));
        assert_eq!((entry.inputs[0].span.virtual_range.start,entry.inputs[0].span.virtual_range.end),(11,12));
        assert_eq!((entry.inputs[0].span.origins[0].original_range.start,entry.inputs[0].span.origins[0].original_range.end),(41,42));
        let (mut unsupported,_,calls)=fixture(false);request.span.origins.clear();
        unsupported.intrinsic_facts.rebase(&request,&calls);assert!(unsupported.intrinsic_facts.entries.is_empty());
    }
    #[test]
    fn intrinsic_claims_reject_modified_unadopted_or_partially_valid_native_inputs() {
        for bad in ["metadata","duplicate","schema","capture","range","pattern","foreign-writer","unadopted","invalid-receipt"] {
            let (mut r,mut tree,mut calls)=fixture(false);let token=root_receipt(&tree.nodes[1].metadata).unwrap();
            match bad {
                "metadata"=>tree.nodes[1].metadata.iter_mut().find(|m|m.key=="owned.read").unwrap().value="changed".into(),
                "duplicate"=>{let m=tree.nodes[1].metadata.iter().find(|m|m.key=="owned.read").unwrap().clone();tree.nodes[1].metadata.push(m);},
                "schema"=>tree.nodes[1].summary.as_mut().unwrap().public_data[0].json="{\"changed\":true}".into(),
                "capture"=>tree.nodes[0].captures.retain(|c|!c.name.starts_with("0:")),
                "range"=>tree.nodes[1].span.virtual_range.start+=1,
                "pattern"=>tree.nodes[0].summary.as_mut().unwrap().pattern_index=Some(1),
                "foreign-writer"=>calls.push(super::super::tests::write(&mut r,&mut tree.nodes[1],"foreign-modifier")),
                "unadopted"=>calls.retain(|c|c.semantic_receipt!=Some(token)),
                _=>r.receipts.get_mut(&token).unwrap().valid=false,
            }
            let retained=calls.iter().filter_map(|c|c.semantic_receipt).collect();
            assert!(r.intrinsic_facts.certify(&tree,&r,&retained).is_empty(),"{bad}");
        }
    }
}

//! Fact-scoped composition. Guest-visible seals are references, never authority.
//! The registry binds accepted output and the current native input subtree.
use super::*;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

pub(in crate::host) const CAPABILITY: &str = "parser.semantic-value-facts";
pub(in crate::host) const DRAFT: &str = "parser.semantic.value-domain.v1";
pub(in crate::host) const INPUT: &str = "parser.native-semantic-input.v1";
pub(in crate::host) const COMPOSE: &str = "parser.semantic.compose.v1";
const MAX_SUBJECTS: usize = 256;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Domain {
    version: u32,
    classes: Vec<String>,
    complete: bool,
    maybe_unset: bool,
    before_change: String,
    evidence: String,
}
impl Domain {
    fn valid(&self)->bool {
        self.version==1 && self.classes.len()<=16 && (!self.complete || !self.classes.is_empty())
            && self.classes.iter().all(|c|!c.is_empty() && c.len()<=1024 && c!="java.lang.Object")
            && self.classes.iter().collect::<BTreeSet<_>>().len()==self.classes.len()
            && matches!(self.before_change.as_str(),"unknown"|"default-clone")
            && !self.evidence.is_empty() && self.evidence.len()<=1024
    }
}
#[derive(Clone,Debug,Deserialize,Serialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct WireSeal {
    version: u32,
    receipt: u64,
    subject: String,
    issuer: String,
    value_domain: Option<Domain>,
}
#[derive(Debug)]
struct ValueOutput { signature:[u8;32], domain:Domain }
#[derive(Clone,Debug)]
struct Subject { wire:WireSeal, digest:[u8;32], tokens:BTreeSet<u64> }
#[derive(Clone,Debug,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct InputClaim { capture:u64, public_schemas:Vec<String> }
#[derive(Clone,Debug,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct ComposeDraft { version:u32, metadata_keys:Vec<String>, inputs:Vec<InputClaim> }
#[derive(Debug)]
struct Composition {
    issuer:String, span:super::super::MappedSpan, registration:String, pattern:u64,
    metadata:Vec<WitMetadataEntry>, inputs:Vec<(InputClaim,Subject)>,
}
#[derive(Debug,Default)]
pub(in crate::host) struct Registry {
    values:BTreeMap<u64,ValueOutput>,
    subjects:Mutex<BTreeMap<String,Subject>>,
    compositions:BTreeMap<u64,Composition>,
}

fn own_entry<'a>(metadata:&'a [WitMetadataEntry],key:&str,issuer:&str)->Option<&'a str> {
    let mut entries=metadata.iter().filter(|e|e.key==key);
    let entry=entries.next()?;
    (entries.next().is_none() && entry.owner_component_id.as_deref()==Some(issuer)).then_some(entry.value.as_str())
}
fn hex(value:&[u8;32])->String { value.iter().map(|b|format!("{b:02x}")).collect() }
fn registered_signature(value:&super::super::WitRegisteredExpressionPayload)->[u8;32] {
    debug_digest("value-output-v1",&(&value.span,&value.definition_id,&value.registration_id,
        &value.element_class,value.pattern_index,&value.effective_return_type,
        &value.effective_possible_return_types,&value.effective_possible_return_types_state,
        &value.effective_multiplicity))
}
fn expression_signature(value:&skript_parser::ExpressionNode)->Option<[u8;32]> {
    let skript_parser::ExpressionNodeKind::Registered {definition_id,registration_id,element_class,pattern_index}=&value.kind else{return None;};
    Some(debug_digest("value-output-v1",&(super::super::mapped_span_to_wit(value.span.mapped.clone()),definition_id,registration_id,
        element_class.as_str(),u64::try_from(*pattern_index).ok()?,value.return_type.as_ref().map(|t|t.as_str().to_owned()),
        value.possible_return_types.iter().map(|t|t.as_str().to_owned()).collect::<Vec<_>>(),
        match value.possible_return_types_state {
            syntaxes::PossibleReturnTypesState::Complete=>super::super::WitPossibleReturnTypesState::Complete,
            syntaxes::PossibleReturnTypesState::Partial=>super::super::WitPossibleReturnTypesState::Partial,
            syntaxes::PossibleReturnTypesState::Unresolved=>super::super::WitPossibleReturnTypesState::Unresolved,
        },
        value.multiplicity.map(super::super::multiplicity_to_wit))))
}
fn tokens(tree:&AstTree)->Option<BTreeSet<u64>> {
    let mut tokens=BTreeSet::new();
    for node in &tree.nodes {
        if !metadata_receipts(&node.metadata,&mut tokens) {return None;}
        if let Some(summary)=&node.summary {if !metadata_receipts(&summary.metadata,&mut tokens) {return None;}}
    }
    Some(tokens)
}
fn projection_digest(tree:&AstTree)->Option<[u8;32]> {
    if tree.nodes.len()>256 {return None;}
    let mut projected=canonical(tree,false)?.tree;
    // The current mapped source is sealed separately. Text is not reconstructed
    // from a fragment; source spans and the complete native semantic tree stay.
    for node in &mut projected.nodes {
        if node.span.origins.len()!=1 || node.span.origins[0].kind!=super::super::WitOriginKind::Exact
            || node.span.origins[0].expansion.is_some() {return None;}
        if !super::super::source_geometry::canonicalize_semantic_subject_text(node) {return None;}
        node.syntax_context=0;
    }
    struct Size(usize);
    impl fmt::Write for Size { fn write_str(&mut self,s:&str)->fmt::Result {self.0=self.0.saturating_add(s.len());if self.0>128*1024 {Err(fmt::Error)}else{Ok(())}} }
    write!(&mut Size(0),"{projected:?}").ok()?;
    Some(debug_digest("native-semantic-subject-v1",&projected))
}
fn subtree(tree:&AstTree,root:u64)->Option<AstTree> {
    let index=tree.nodes.iter().map(|n|(n.id,n)).collect::<BTreeMap<_,_>>();
    let mut ids=BTreeSet::new();let mut pending=vec![root];
    while let Some(id)=pending.pop() {
        if !ids.insert(id) {continue;}
        if ids.len()>256 {return None;}
        pending.extend(references(index.get(&id)?));
    }
    Some(AstTree {roots:vec![root],nodes:ids.into_iter().map(|id|index[&id].clone()).collect()})
}

impl Registry {
    pub(in crate::host) fn has_value(&self,expression:&skript_parser::ExpressionNode)->bool {
        let mut root=expression;
        while matches!(root.kind,skript_parser::ExpressionNodeKind::Grouped) && root.children.len()==1 {root=&root.children[0];}
        let mut ids=BTreeSet::new();
        metadata_receipts(&super::super::metadata_to_wit(&root.metadata),&mut ids)
            && ids.len()==1 && ids.first().and_then(|id|self.values.get(id))
                .is_some_and(|value|expression_signature(root)==Some(value.signature))
    }
    pub(in crate::host) fn record(&mut self,token:u64,payload:&HookPayload,issuer:&str) {
        match payload {
            HookPayload::RegisteredExpression(value)=> {
                let Some(json)=own_entry(&value.metadata,DRAFT,issuer) else{return;};
                if json.len()>16*1024 {return;}
                let Ok(domain)=serde_json::from_str::<Domain>(json) else{return;};
                if domain.valid() && self.values.len()<MAX_SUBJECTS {
                    self.values.insert(token,ValueOutput {signature:registered_signature(value),domain});
                }
            }
            HookPayload::Effect(value)=> {
                let Some(candidate)=value.candidate.as_ref() else{return;};
                let Some(json)=own_entry(&candidate.metadata,COMPOSE,issuer) else{return;};
                if json.len()>16*1024 || self.compositions.len()>=MAX_SUBJECTS {return;}
                let Ok(draft)=serde_json::from_str::<ComposeDraft>(json) else{return;};
                if draft.version!=1 || draft.inputs.is_empty() || draft.inputs.len()>8 || draft.metadata_keys.is_empty() || draft.metadata_keys.len()>16
                    || draft.inputs.iter().map(|i|i.capture).collect::<BTreeSet<_>>().len()!=draft.inputs.len()
                    || draft.metadata_keys.iter().collect::<BTreeSet<_>>().len()!=draft.metadata_keys.len() {return;}
                let Ok(subjects)=self.subjects.lock() else{return;};
                let mut inputs=Vec::new();
                for claim in draft.inputs {
                    if claim.public_schemas.len()>8 || claim.public_schemas.iter().any(|s|s.is_empty()||s.len()>256) {return;}
                    let mut captures=candidate.parsed_captures.iter().filter(|c|c.capture_index==claim.capture);
                    let Some(capture)=captures.next() else{return;};
                    if captures.next().is_some() || capture.status!=super::super::WitParseResultStatus::Success {return;}
                    let Some(summary)=capture.summary.as_ref() else{return;};
                    let mut seals=summary.metadata.iter().filter(|m|m.key==INPUT);
                    let Some(entry)=seals.next().filter(|e|e.owner_component_id.is_none()) else{return;};
                    if seals.next().is_some() {return;}
                    let Some(subject)=subjects.get(&entry.value) else{return;};
                    if !claim.public_schemas.is_empty() && subject.wire.issuer!=issuer {return;}
                    if claim.public_schemas.iter().any(|id|summary.public_data.iter().filter(|p|p.schema_id==*id).count()!=1) {return;}
                    inputs.push((claim,subject.clone()));
                }
                let mut metadata=Vec::new();
                for key in draft.metadata_keys {
                    if key.is_empty() || key.len()>256 || key.starts_with("parser.") {return;}
                    let Some(json)=own_entry(&candidate.metadata,&key,issuer) else{return;};
                    metadata.push(WitMetadataEntry {key,value:json.into(),owner_component_id:Some(issuer.into())});
                }
                self.compositions.insert(token,Composition {issuer:issuer.into(),span:candidate.span.clone(),registration:candidate.registration_id.clone(),
                    pattern:candidate.pattern_index,metadata,inputs});
            }
            _=>{}
        }
    }
    pub(in crate::host) fn seal(&self,receipts:&ReceiptRegistry,expression:&skript_parser::ExpressionNode)->Option<WitMetadataEntry> {
        let mut root=expression;
        while matches!(root.kind,skript_parser::ExpressionNodeKind::Grouped) && root.children.len()==1 {root=&root.children[0];}
        let metadata=super::super::metadata_to_wit(&root.metadata);
        let mut ids=BTreeSet::new();
        if !metadata_receipts(&metadata,&mut ids) || ids.len()!=1 {return None;}
        let token=*ids.first()?;let receipt=receipts.receipts.get(&token)?;
        if !receipt.valid || receipt.unbound || receipt.writers.len()!=1 || !receipt.writers.contains(&receipt.issuer) {return None;}
        let tree=super::super::ast_tree::expression_subject(expression);
        let digest=projection_digest(&tree)?;let tokens=tokens(&tree)?;
        if tokens.iter().any(|id|receipts.receipts.get(id).is_none_or(|r|!r.valid||r.unbound)) {return None;}
        let value_domain=self.values.get(&token).filter(|v|expression_signature(root)==Some(v.signature)).map(|v|v.domain.clone());
        let wire=WireSeal {version:1,receipt:token,subject:hex(&digest),issuer:receipt.issuer.clone(),value_domain};
        let key=serde_json::to_string(&wire).ok()?;
        let mut subjects=self.subjects.lock().ok()?;
        if !subjects.contains_key(&key) && subjects.len()>=MAX_SUBJECTS {return None;}
        subjects.insert(key.clone(),Subject {wire,digest,tokens});
        Some(WitMetadataEntry {key:INPUT.into(),value:key,owner_component_id:None})
    }
    pub(super) fn has_receipt(&self,token:u64)->bool {self.values.contains_key(&token)||self.compositions.contains_key(&token)}

    pub(super) fn certify(&self,tree:&AstTree,receipts:&ReceiptRegistry,retained:&BTreeSet<u64>,global:&BTreeSet<String>,own:&BTreeMap<u64,BTreeSet<String>>)->BTreeMap<u64,FactClaims> {
        let mut claims=BTreeMap::new();
        for parent in &tree.nodes {
            let mut parent_tokens=BTreeSet::new();
            if !metadata_receipts(&parent.metadata,&mut parent_tokens) {continue;}
            for token in parent_tokens {
                let Some(composition)=self.compositions.get(&token) else{continue;};
                let issuer=&composition.issuer;
                if !retained.contains(&token) || global.iter().any(|w|w!=issuer)
                    || own.get(&parent.id).is_none_or(|w|w.iter().any(|w|w!=issuer))
                    || !ancestor_authors(tree,parent.id,own,issuer)
                    || debug_digest("span",&parent.span)!=debug_digest("span",&composition.span)
                    || parent.summary.as_ref().is_none_or(|s|s.registration_id.as_ref()!=Some(&composition.registration)||s.pattern_index!=Some(composition.pattern)) {continue;}
                if composition.metadata.iter().any(|m|!parent.metadata.iter().any(|p|p.key==m.key&&p.value==m.value&&p.owner_component_id==m.owner_component_id)) {continue;}
                let Some(region)=subtree(tree,parent.id) else{continue;};
                let allowed=composition.inputs.iter().flat_map(|(_,s)|s.tokens.iter().copied()).collect::<BTreeSet<_>>();
                let Some(region_tokens)=tokens(&region) else{continue;};
                if region_tokens.iter().any(|t|!retained.contains(t)||receipts.receipts.get(t).is_none_or(|r|!r.valid||r.unbound||r.issuer!=*issuer&&!allowed.contains(t))) {continue;}
                let mut verified=Vec::new();
                for (input,subject) in &composition.inputs {
                    let prefix=format!("{}:",input.capture);
                    let ids=parent.captures.iter().filter(|c|c.name.starts_with(&prefix)).filter_map(|c|match c.value {CaptureValue::Node(id)=>Some(id),_=>None}).collect::<Vec<_>>();
                    let [id]=ids.as_slice() else{break;};
                    let Some(input_tree)=subtree(tree,*id) else{break;};
                    if projection_digest(&input_tree)!=Some(subject.digest) || !subject.tokens.is_subset(retained) {break;}
                    verified.push((*id,input));
                }
                if verified.len()!=composition.inputs.len() {continue;}
                let entry=claims.entry(parent.id).or_insert_with(FactClaims::default);
                for metadata in &composition.metadata {entry.metadata.insert((issuer.clone(),metadata.key.clone()));}
                for (id,input) in verified {
                    for schema in &input.public_schemas {claims.entry(id).or_insert_with(FactClaims::default).public_data.insert((issuer.clone(),schema.clone()));}
                }
            }
        }
        claims
    }
}

pub(super) fn ancestor_authors(tree:&AstTree,root:u64,own:&BTreeMap<u64,BTreeSet<String>>,issuer:&str)->bool {
    let mut reverse=BTreeMap::<u64,Vec<u64>>::new();
    for node in &tree.nodes {for child in references(node) {reverse.entry(child).or_default().push(node.id);}}
    let mut seen=BTreeSet::new();let mut pending=vec![root];
    while let Some(id)=pending.pop() {
        if !seen.insert(id) {continue;}
        if seen.len()>8192 || own.get(&id).is_none_or(|writers|writers.iter().any(|w|w!=issuer)) {return false;}
        pending.extend(reverse.get(&id).into_iter().flatten().copied());
    }
    true
}

#[derive(Clone,Debug,Default)]
pub(in crate::host) struct FactClaims {
    pub(super) metadata:BTreeSet<(String,String)>,
    pub(super) public_data:BTreeSet<(String,String)>,
}

pub(super) fn volatile(payload:&HookPayload)->bool {
    // A header-local cache may retain an entry *draft*. Its restored current
    // payload still passes normal dispatch and receives a fresh private entry
    // admission. No old entry receipt/certificate is stored or imported.
    let header_entry = matches!(payload, HookPayload::Structure(p)
        if matches!(p.timing, super::super::WitStructureTiming::EnterBody));
    let mut found=false;
    cache_metadata(payload,&mut |m|found|=m.iter().any(|e|matches!(e.key.as_str(),DRAFT|INPUT|COMPOSE)
        || e.key==super::entry_facts::DRAFT && !header_entry));
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::*;
    fn leaf()->skript_parser::ExpressionNode {
        let (_,root,_)=crate::host::capture_view_tests::fixture();
        let mut leaf=root.children[0].children[0].children[0].clone();
        leaf.children.clear();leaf.captures.clear();leaf.public_data.clear();leaf.metadata.clear();
        leaf.function=None;leaf.effects=None;
        leaf.kind=skript_parser::ExpressionNodeKind::Registered {definition_id:"owned.value".into(),registration_id:"owned.value.0".into(),
            element_class:syntaxes::ClassName("owned.Value".into()),pattern_index:0};
        leaf.return_type=Some(syntaxes::ClassName("java.lang.Object".into()));leaf.possible_return_types.clear();
        leaf.possible_return_types_state=syntaxes::PossibleReturnTypesState::Unresolved;
        leaf.multiplicity=Some(syntaxes::Multiplicity::Single);leaf
    }
    fn accept(registry:&mut ReceiptRegistry,expression:&mut skript_parser::ExpressionNode,issuer:&str)->HookCall {
        let mut value=crate::host::semantic_proof::tests::node(0);
        value.metadata=metadata_to_wit(&expression.metadata);
        let call=crate::host::semantic_proof::tests::write(registry,&mut value,issuer);
        expression.metadata=metadata_entries(value.metadata).unwrap();call
    }
    fn domain()->Domain {Domain {version:1,classes:vec!["java.lang.String".into()],complete:true,maybe_unset:true,
        before_change:"default-clone".into(),evidence:"owned source fixture".into()}}
    #[test]
    fn native_input_is_bound_to_current_receipt_summary_and_exact_tree() {
        let mut registry=ReceiptRegistry::default();let mut expression=leaf();
        let call=accept(&mut registry,&mut expression,"owned.provider");let token=call.semantic_receipt.unwrap();
        registry.value_facts.values.insert(token,ValueOutput {signature:expression_signature(&expression).unwrap(),domain:domain()});
        let entry=registry.value_facts.seal(&registry,&expression).unwrap();
        let wire:WireSeal=serde_json::from_str(&entry.value).unwrap();assert!(wire.value_domain.is_some());assert!(entry.owner_component_id.is_none());
        assert!(ReceiptRegistry::default().value_facts.seal(&ReceiptRegistry::default(),&expression).is_none(),"old/foreign registry");
        let mut moved=expression.clone();moved.span.mapped.virtual_range.start+=1;
        let moved=registry.value_facts.seal(&registry,&moved).unwrap();let moved:WireSeal=serde_json::from_str(&moved.value).unwrap();
        assert!(moved.value_domain.is_none(),"copied output does not keep the original value domain");assert_ne!(moved.subject,wire.subject);
        let mut changed=expression.clone();changed.possible_return_types_state=syntaxes::PossibleReturnTypesState::Complete;
        let changed=registry.value_facts.seal(&registry,&changed).unwrap();let changed:WireSeal=serde_json::from_str(&changed.value).unwrap();assert!(changed.value_domain.is_none());
        let mut grouped=expression.clone();grouped.kind=skript_parser::ExpressionNodeKind::Grouped;grouped.metadata.clear();grouped.children=vec![expression.clone()];
        let grouped=registry.value_facts.seal(&registry,&grouped).unwrap();let grouped:WireSeal=serde_json::from_str(&grouped.value).unwrap();assert!(grouped.value_domain.is_some());assert_ne!(grouped.subject,wire.subject);
        registry.receipts.get_mut(&token).unwrap().unbound=true;assert!(registry.value_facts.seal(&registry,&expression).is_none());
    }
    #[test]
    fn native_subject_matches_the_actual_source_projection() {
        let (source, _, _) = crate::host::capture_view_tests::fixture();
        let expression = leaf();
        let prepared = ast_tree::expression_subject(&expression);
        let current = ast_tree::expression_for_test(&source, &expression);
        assert_ne!(prepared.nodes[0].text, current.nodes[0].text);
        assert!(crate::host::source_geometry::pattern_capture_geometry_is_current(&current.nodes[0]));
        let prepared_digest = projection_digest(&prepared).expect("valid native prepared subject");
        let current_digest = projection_digest(&current).expect("valid current-source subject");
        assert_eq!(prepared_digest, current_digest,
            "the native subject must survive the real document AST projection");
    }
    #[test]
    fn subject_text_normalization_requires_the_original_native_geometry() {
        let (source, _, _) = crate::host::capture_view_tests::fixture();
        let current = ast_tree::expression_for_test(&source, &leaf());
        for mode in 0..5 {
            let mut changed = current.clone();
            let node = &mut changed.nodes[0];
            let seal = node.metadata.iter().position(|m|m.owner_component_id.is_none()
                && m.key == "parser.pattern-capture-seal.v1").unwrap();
            match mode {
                0 => { node.metadata.remove(seal); },
                1 => node.metadata.push(node.metadata[seal].clone()),
                2 => node.metadata[seal].value.push('0'),
                3 => node.text.push('x'),
                _ => node.span.virtual_range.start += 1,
            }
            assert!(projection_digest(&changed).is_none(), "invalid original geometry {mode}");
        }
    }
    #[test]
    fn subject_normalization_rejects_capture_changes_and_foreign_geometry() {
        let (source, expression, _) = crate::host::capture_view_tests::fixture();
        let current = ast_tree::expression_for_test(&source, &expression);
        assert!(projection_digest(&current).is_some());
        let root = current.nodes.iter().position(|n|n.id == current.roots[0]).unwrap();
        for mode in 0..3 {
            let mut changed = current.clone();
            let node = &mut changed.nodes[root];
            match mode {
                0 => {
                    let capture = node.captures.iter_mut().find(|c|
                        c.name.starts_with("parser.pattern-capture:")).unwrap();
                    let CaptureValue::Span(span) = &mut capture.value else { panic!() };
                    span.virtual_range.start += 1;
                },
                1 => {
                    let capture = node.captures.iter_mut().find(|c|
                        c.name.starts_with("parser.pattern-expected-types:")).unwrap();
                    capture.value = CaptureValue::Text(r#"["foreign-type"]"#.into());
                },
                _ => {
                    let seal = node.metadata.iter_mut().find(|m|
                        m.key == "parser.pattern-capture-seal.v1").unwrap();
                    seal.owner_component_id = Some("foreign".into());
                },
            }
            assert!(projection_digest(&changed).is_none(), "changed original capture {mode}");
        }
    }
    #[test]
    fn value_seal_rejects_ambiguous_provenance_and_additional_writers() {
        let mut registry=ReceiptRegistry::default();let mut expression=leaf();accept(&mut registry,&mut expression,"a");
        let mut replaced=expression.clone();replaced.span.mapped.origins[0].kind=skript_parser::OriginKind::Replaced;
        assert!(registry.value_facts.seal(&registry,&replaced).is_none());
        accept(&mut registry,&mut expression,"b");assert!(registry.value_facts.seal(&registry,&expression).is_none());
    }
    fn fixture()->(ReceiptRegistry,AstTree,Vec<HookCall>) {
        let mut registry=ReceiptRegistry::default();let mut target=leaf();let mut value=leaf();
        target.public_data.push(skript_parser::ExpressionPublicData {schema_id:"owned.identity".into(),schema_version:1,json:r#"{"name":"local"}"#.into()});
        let a=accept(&mut registry,&mut target,"core");let b=accept(&mut registry,&mut value,"foreign");
        let entry=registry.value_facts.seal(&registry,&target).unwrap();let target_subject=registry.value_facts.subjects.lock().unwrap()[&entry.value].clone();
        let entry=registry.value_facts.seal(&registry,&value).unwrap();let value_subject=registry.value_facts.subjects.lock().unwrap()[&entry.value].clone();
        let (source, _, _) = crate::host::capture_view_tests::fixture();
        let mut target=ast_tree::expression_for_test(&source,&target).nodes.remove(0);target.id=1;
        let mut value=ast_tree::expression_for_test(&source,&value).nodes.remove(0);value.id=2;
        let mut parent=crate::host::semantic_proof::tests::node(0);parent.kind=SyntaxKind::Effect;
        parent.span=target.span.clone();parent.children=vec![1,2];parent.captures=vec![Capture {name:"0:host.expression".into(),value:CaptureValue::Node(1)},Capture {name:"1:host.expression".into(),value:CaptureValue::Node(2)}];
        parent.summary=target.summary.clone();parent.metadata=vec![WitMetadataEntry {key:"owned.assignment".into(),value:"stored".into(),owner_component_id:Some("core".into())}];
        let c=crate::host::semantic_proof::tests::write(&mut registry,&mut parent,"core");
        registry.value_facts.compositions.insert(c.semantic_receipt.unwrap(),Composition {issuer:"core".into(),span:parent.span.clone(),registration:parent.summary.as_ref().unwrap().registration_id.clone().unwrap(),pattern:0,
            metadata:parent.metadata.iter().filter(|m|m.key=="owned.assignment").cloned().collect(),inputs:vec![(InputClaim {capture:0,public_schemas:vec!["owned.identity".into()]},target_subject),(InputClaim {capture:1,public_schemas:vec![]},value_subject)]});
        (registry,AstTree {roots:vec![0],nodes:vec![parent,target,value]},vec![a,b,c])
    }
    #[test]
    fn composition_only_authenticates_selected_current_facts_and_keeps_unique_issuer_strict() {
        let (registry,tree,calls)=fixture();let proof=SemanticProofRegistry::capture(&tree,Some(&registry),&calls);let view=proof.view(&tree);
        assert!(!view.provider_valid(&tree.nodes[0],"core"));
        let entry=tree.nodes[0].metadata.iter().find(|m|m.key=="owned.assignment").unwrap();
        assert!(view.composed_metadata_valid(&tree.nodes[0],entry));assert!(view.composed_public_data_valid(&tree.nodes[1],"owned.identity"));
        assert!(!view.composed_public_data_valid(&tree.nodes[2],"owned.identity"));
        assert!(!view.composed_metadata_valid(&tree.nodes[0],&entry.clone()));
        let source = skript_parser::MappedSource::identity("native source");
        let bound = SemanticProofRegistry::capture(&tree,Some(&registry),&calls).bind_source(&source);
        assert!(bound.view_for_source(&tree,&source).composed_metadata_valid(&tree.nodes[0],entry));
        let edited = skript_parser::MappedSource::identity("edited source");
        assert!(!bound.view_for_source(&tree,&edited).composed_metadata_valid(&tree.nodes[0],entry));
        let mut changed=tree.clone();changed.nodes[2].summary.as_mut().unwrap().return_type=Some("forged.Type".into());
        assert!(!proof.view(&changed).composed_public_data_valid(&changed.nodes[1],"owned.identity"));
        let cancelled=SemanticProofRegistry::capture(&tree,Some(&registry),&calls[..2]);
        assert!(!cancelled.view(&tree).composed_metadata_valid(&tree.nodes[0],entry));
    }
    #[test]
    fn composition_rejects_changed_capture_extra_writer_and_opaque_global_effect() {
        for mode in 0..4 {
            let (mut registry,mut tree,mut calls)=fixture();
            match mode {
                0=>tree.nodes[0].captures[1].value=CaptureValue::Node(1),
                1=>{let call=crate::host::semantic_proof::tests::write(&mut registry,&mut tree.nodes[2],"other");calls.push(call);},
                2=>{let token=calls[1].semantic_receipt.unwrap();registry.receipts.get_mut(&token).unwrap().unbound=true;},
                _=>{let call=crate::host::semantic_proof::tests::write(&mut registry,&mut tree.nodes[0],"other");calls.push(call);},
            }
            let proof=SemanticProofRegistry::capture(&tree,Some(&registry),&calls);let view=proof.view(&tree);
            let entry=tree.nodes[0].metadata.iter().find(|m|m.key=="owned.assignment").unwrap();
            assert!(!view.composed_metadata_valid(&tree.nodes[0],entry),"{mode}");
        }
    }
    #[test]
    fn volatile_fact_inputs_are_not_replayed_from_dispatch_or_body_cache() {
        let mut payload=crate::host::semantic_proof::tests::payload();
        if let HookPayload::Matching(p)=&mut payload {p.metadata.push(WitMetadataEntry {key:INPUT.into(),value:"copied".into(),owner_component_id:None});}
        assert!(volatile(&payload));assert!(with_payload_receipts_normalized(&mut payload, |_|()).is_none());
        let mut output=HookOutput {decision:crate::host::HookDecision::ContinueProcessing,replacement:Some(payload.clone()),effects:crate::host::empty_effects()};
        assert!(!normalize_output_receipts(&payload,&mut output));
        assert!(ReceiptRegistry::default().export_body(&[],&[(COMPOSE.into(),"{}".into())]).is_none());
    }
}

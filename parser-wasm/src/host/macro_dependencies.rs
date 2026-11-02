//! Source-bound unavailable macro names. These never authorize positive facts.
use super::*;
use serde::Deserialize;
use std::sync::atomic::{AtomicU64,Ordering};

pub(super) const PARSER_ID:&str="host.macro-declarations";
pub(super) const OUTPUT_KEY:&str="macro.unavailable-bindings.v1";
pub(super) const MAX_PROVIDER_BINDINGS:usize=64;
pub(super) const MAX_BINDINGS:usize=256;
pub(super) const MAX_OUTPUT_BYTES:usize=64*1024;
static NEXT_BINDING:AtomicU64=AtomicU64::new(1);

#[derive(Clone,Debug,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct WireDeclarations {version:u32,complete:bool,bindings:Vec<WireBinding>}
#[derive(Clone,Debug,Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct WireBinding {root:u64,namespace:String,name:String}
#[derive(Clone,Debug,PartialEq,Eq)]
pub(super) struct Binding {
    pub id:u64,pub namespace:String,pub name:String,
    /// All declarers, never a chosen value writer. Locations are snapshot data.
    pub owners:BTreeSet<String>,pub roots:Vec<usize>,
}
#[derive(Clone,Debug)]
pub(super) struct Snapshot {pub complete:bool,pub bindings:Vec<Binding>}
#[derive(Clone,Debug,Default)]
pub(super) struct Reads {pub namespaces:BTreeMap<String,bool>,pub bindings:BTreeSet<u64>}

fn valid_name(value:&str)->bool {!value.is_empty()&&value.len()<=256&&!value.chars().any(char::is_control)}
fn next_id()->Result<u64,String> {
    NEXT_BINDING.fetch_update(Ordering::Relaxed,Ordering::Relaxed,|id|id.checked_add(1))
        .map_err(|_|"macro binding identity space exhausted".into())
}
impl Snapshot {
    pub fn empty()->Self {Self {complete:true,bindings:Vec::new()}}
    pub fn same_meaning(&self,other:&Self)->bool {
        self.complete==other.complete && self.bindings.iter().map(|b|(&b.namespace,&b.name,&b.owners))
            .eq(other.bindings.iter().map(|b|(&b.namespace,&b.name,&b.owners)))
    }
    pub fn namespace(&self,namespace:&str)->Vec<&Binding> {
        self.bindings.iter().filter(|b|b.namespace==namespace).collect()
    }
    pub fn adopt(&self,reads:&Reads,values:&[crate::bindings::nlaocs::skript_parser_addon::types::MacroUnavailability])
        ->Result<Vec<String>,String> {
        use crate::bindings::nlaocs::skript_parser_addon::types::MacroUnavailability as U;
        if values.len()>64 {return Err("macro dependency reference count limit".into());}
        let mut names=BTreeSet::new();
        for value in values {match value {
            U::Binding(id)=>{
                let binding=self.bindings.iter().find(|b|b.id==*id)
                    .filter(|_|reads.bindings.contains(id)).ok_or("stale or unqueried macro binding")?;
                names.insert(format!("{}: {}",binding.namespace,binding.name));
            }
            U::IncompleteNamespace(namespace)=>{
                if !valid_name(namespace)||self.complete||reads.namespaces.get(namespace)!=Some(&false) {
                    return Err("macro namespace was not observed incomplete in this call".into());
                }
                names.insert(format!("{namespace}: <incomplete discovery>"));
            }
        }}
        Ok(names.into_iter().collect())
    }
}

/// Exact header inputs, ignoring trivia and numerical arena identities.
/// Recomputed only after an accepted edit, not at every unchanged node.
pub(super) fn root_signature(tree:&ParserRawTree)->Vec<(ParserRawNodeKind,String,u64,usize)> {
    tree.roots.iter().filter_map(|id|tree.get(*id)).filter(|n|!matches!(n.kind,ParserRawNodeKind::Blank|ParserRawNodeKind::Comment))
        .map(|n|(n.kind,n.text.clone(),u64::from(n.syntax_context.get()),n.children.len())).collect()
}

/// Each output is already owner-stamped by its own callback boundary. A healthy
/// provider must explicitly say complete, including a successful empty result.
pub(super) fn freeze(tree:&ParserRawTree,records:&[WitMetadataEntry],expected:&BTreeSet<String>,faulted:bool)
    ->Result<Snapshot,String> {
    let mut seen=BTreeSet::new();let mut complete=!faulted;let mut bytes=0usize;
    let mut grouped=BTreeMap::<(String,String),(BTreeSet<String>,Vec<usize>)>::new();let mut total=0usize;
    for record in records.iter().filter(|r|r.key==OUTPUT_KEY) {
        let owner=record.owner_component_id.as_ref().filter(|s|!s.is_empty()).ok_or("macro declaration has no owner")?;
        if !expected.contains(owner)||!seen.insert(owner.clone()) {return Err("duplicate or foreign macro declaration owner".into());}
        bytes=bytes.saturating_add(record.value.len());
        if bytes>MAX_OUTPUT_BYTES {return Err("macro declaration byte limit".into());}
        let wire:WireDeclarations=serde_json::from_str(&record.value).map_err(|e|format!("invalid macro declarations: {e}"))?;
        if wire.version!=1||wire.bindings.len()>MAX_PROVIDER_BINDINGS {return Err("macro declaration version/count limit".into());}
        complete&=wire.complete;let mut unique=BTreeSet::new();
        for binding in wire.bindings {
            total+=1;if total>MAX_BINDINGS {return Err("total macro declaration count limit".into());}
            if !valid_name(&binding.namespace)||!valid_name(&binding.name)
                ||!unique.insert((binding.root,binding.namespace.clone(),binding.name.clone())) {
                return Err("invalid or duplicate macro declaration".into());
            }
            let ordinal=tree.roots.iter().position(|id|id.get()==binding.root).ok_or("macro declaration is not a current root")?;
            let node=tree.get(tree.roots[ordinal]).ok_or("macro declaration root is absent")?;
            if matches!(node.kind,ParserRawNodeKind::Blank|ParserRawNodeKind::Comment)||node.parent.is_some()
                ||node.syntax_context.get()!=0 {return Err("macro declaration root has unsupported context/kind".into());}
            let entry=grouped.entry((binding.namespace,binding.name)).or_default();entry.0.insert(owner.clone());entry.1.push(ordinal);
        }
    }
    complete&=seen==*expected;
    let bindings=grouped.into_iter().map(|((namespace,name),(owners,roots))|Ok(Binding {id:next_id()?,namespace,name,owners,roots}))
        .collect::<Result<Vec<_>,String>>()?;
    Ok(Snapshot {complete,bindings})
}

pub(super) struct Prepared {
    pub snapshot:Arc<Snapshot>,pub calls:Vec<HookCall>,pub failures:Vec<ComponentFailure>,
    pub effects:HookEffects,pub work:usize,pub active:bool,
}

pub(super) fn validate_subscription(manifest:&ComponentManifest,subscription:&HookSubscription)->Result<(),HostError> {
    if !matches!(subscription.target,HookTarget::ParseStage)||subscription.phase!=HookPhase::Document
        ||subscription.mode!=HookMode::Transform||!selector_is_empty(&subscription.selector)
        ||!manifest.capabilities.iter().any(|c|c.id==crate::CAPABILITY_MACRO_DECLARATIONS&&c.required&&c.minimum_version==1)
        ||!manifest.capabilities.iter().any(|c|c.id==crate::CAPABILITY_PROJECT_STABLE_SEMANTICS&&c.required&&c.minimum_version==2)
        ||manifest.subscriptions.iter().filter(|s|s.capability_id==crate::CAPABILITY_MACRO_DECLARATIONS).count()!=1 {
        return Err(HostError::InvalidManifest {message:"macro declarations require one empty-selector parse-stage Document/Transform subscription, mandatory macro-declarations v1 and project-stable-semantics v2".into()});
    }
    Ok(())
}

/// Only this callback's response is visible to it. Native aggregation restores
/// the already authenticated records after output validation, outside the guest.
pub(super) fn validate_output(original:&HookPayload,prior:&HookPayload,output:&mut HookOutput,owner:&str,
    tree:&ParserRawTree,accesses:&StateReadWriteSet)->Result<(),String> {
    if !accesses.reads.is_empty()||!accesses.writes.is_empty()||!accesses.namespace_revisions.is_empty()
        ||!output.effects.context_updates.is_empty()||!output.effects.parse_requests.is_empty()||!output.effects.parse_results.is_empty()
        ||!matches!(output.decision,HookDecision::ContinueProcessing) {
        return Err("macro declarations are readonly, nonterminal header discovery; no State, context, parsing or registry effects".into());
    }
    let HookPayload::Parser(input)=original else{return Err("invalid macro declaration carrier".into());};
    let Some(HookPayload::Parser(next))=&mut output.replacement else{return Err("macro declaration provider omitted an explicit completion record".into());};
    if input.parser_id!=PARSER_ID||next.parser_id!=input.parser_id||next.request_id!=0||!next.input.is_empty()
        ||!next.expected_types.is_empty()||!same_mapped_span(&input.span,&next.span)||input.options.len()!=1||next.options.len()!=2 {
        return Err("macro declaration provider changed its immutable carrier".into());
    }
    let native=next.options.iter().filter(|o|o.key==declaration_prepass::TREE_OPTION).collect::<Vec<_>>();
    if native.len()!=1||!same_metadata_entries(&input.options,&[(*native[0]).clone()]) {return Err("macro declaration tree lease changed".into());}
    let records=next.options.iter().filter(|o|o.key==OUTPUT_KEY).collect::<Vec<_>>();
    if records.len()!=1 {return Err("macro declarations require one explicit completion record".into());}
    let mut record=(*records[0]).clone();
    if record.owner_component_id.as_deref().is_some_and(|id|id!=owner) {return Err("foreign macro declaration owner".into());}
    record.owner_component_id=Some(owner.into());
    freeze(tree,&[record.clone()],&BTreeSet::from([owner.into()]),false)?;
    let HookPayload::Parser(prior)=prior else{return Err("invalid prior macro carrier".into());};
    let mut options=input.options.clone();
    for prior in prior.options.iter().filter(|o|o.key==OUTPUT_KEY) {
        if prior.owner_component_id.as_deref()==Some(owner){return Err("repeated macro declaration owner".into());}
        options.push(prior.clone());
    }
    options.push(record);next.options=options;
    Ok(())
}

impl ParserHost {
    pub(super) fn prepare_macro_declarations(&mut self,transaction:&ParseTransaction,context:&InvocationContext,
        source:&MappedSource,tree:Arc<ParserRawTree>,remaining:usize)->Result<Prepared,HostError> {
        self.check_native_cancellation()?;
        if transaction.document_id()?!=context.document_id||transaction.document_revision()?!=context.document_revision {
            return Err(StateError::InvalidInput {message:"macro declaration source differs from transaction".into()}.into());
        }
        let candidates=self.registry.matching_capability(&DispatchTarget::ParseStage,HookPhase::Document,crate::CAPABILITY_MACRO_DECLARATIONS,None);
        let expected=candidates.iter().filter(|c|!self.components[c.component_index].unloaded)
            .map(|c|self.components[c.component_index].manifest.component_id.clone()).collect::<BTreeSet<_>>();
        if expected.is_empty(){return Ok(Prepared {snapshot:Arc::new(Snapshot::empty()),calls:vec![],failures:vec![],effects:empty_effects(),work:0,active:false});}
        let work=candidates.len();
        if work>remaining {return Err(HostError::TreeMacroCallQuotaExceeded {limit:self.config.max_tree_macro_calls});}
        let state=transaction.savepoint()?;
        let input=HookPayload::Parser(ParseRequest {request_id:0,parser_id:PARSER_ID.into(),input:String::new(),expected_types:vec![],options:vec![],
            span:mapped_span_to_wit(source.map_range(ParserTextRange::new(0,source.virtual_source().len())).map_err(|e|HostError::SourceMapping {message:e.to_string()})?)});
        let result=self.dispatch_with_transaction_for_capability(transaction,DispatchRequest {context:context.clone(),target:DispatchTarget::ParseStage,phase:HookPhase::Document,payload:input},
            crate::CAPABILITY_MACRO_DECLARATIONS,Some(Arc::clone(&tree)),None);
        // Enforce readonly discovery even on a recoverable guest fault. The
        // dispatcher already rejects State accesses and creates no registry lease.
        transaction.rollback_to(&state)?;
        let mut result=result?;
        self.check_native_cancellation()?;
        normalize_text_macro_effects(source,&mut result.effects,"macro-declarations.effects").map_err(|message|HostError::SourceMapping {message})?;
        let records=match &result.payload {HookPayload::Parser(p)=>p.options.as_slice(),_=>&[]};
        let faulted=result.fault.is_some()||!result.failures.is_empty()||!matches!(result.decision,HookDecision::ContinueProcessing);
        let snapshot=match freeze(&tree,records,&expected,faulted) {
            Ok(snapshot)=>snapshot,
            Err(message)=>{
                result.failures.push(ComponentFailure {affected_span:None,component_id:"<macro-declarations>".into(),subscription_id:PARSER_ID.into(),
                    error:HostError::InvalidHookOutput {component_id:"<macro-declarations>".into(),subscription_id:PARSER_ID.into(),message}});
                Snapshot {complete:false,bindings:vec![]}
            }
        };
        Ok(Prepared {snapshot:Arc::new(snapshot),calls:result.calls,failures:result.failures,effects:result.effects,work,active:true})
    }
}

#[cfg(test)]mod tests {
    fn protocol_host()->ParserHost {
        let mut host=ParserHost::new(include_bytes!("../../../artifacts/core-library.wasm"),HostConfig {
            runtime_profile:RuntimeProfile {skript_version:Some("2.16.0".into()),..Default::default()},..Default::default()}).unwrap();
        host.load_addon(include_bytes!("../../../artifacts/tree-macro-addon.wasm")).unwrap();host
    }
    fn no_live_declaration_resources(host:&ParserHost) {
        for component in &host.components {
            let data=component.store.data();
            assert!(data.tree_view.is_none()&&data.invocation.is_none()&&data.dynamic_syntax_update.is_none()&&data.project_declaration.is_none());
            assert!(!data.editor_readonly,"readonly mode leaked from discovery");
        }
    }
    #[test]
    fn declaration_failure_revokes_leases_and_restores_readonly_mode() {
        for text in ["missing-declarations\n","trap-declarations\n"] {
            let mut host=protocol_host();let source=MappedSource::identity(text);
            let tree=Arc::new(skript_parser::parse_raw_tree(&source,skript_parser::RawTreeOptions::for_skript_version(2,16)));
            let context=InvocationContext {invocation_id:1,subscription_id:String::new(),document_id:"file:///macro/lifetime.sk".into(),document_revision:1,expansion:None,syntax_context:0};
            let tx=host.begin_parse("file:///macro",&context.document_id,1).unwrap();
            let prepared=host.prepare_macro_declarations(&tx,&context,&source,tree,4096).unwrap();
            assert!(!prepared.snapshot.complete);assert!(!prepared.failures.is_empty());
            no_live_declaration_resources(&host);assert!(tx.read_write_set().unwrap().writes.is_empty());tx.cancel().unwrap();
        }
    }
    #[test]
    fn cancellation_after_discovery_never_publishes_old_snapshots_and_retry_works() {
        let mut saw_executed_cancel=false;
        for budget in 0..6 {
            let mut host=protocol_host();let source=MappedSource::identity("declare-unavailable computed\n");
            let tree=Arc::new(skript_parser::parse_raw_tree(&source,skript_parser::RawTreeOptions::for_skript_version(2,16)));
            let context=InvocationContext {invocation_id:1,subscription_id:String::new(),document_id:"file:///macro/cancel.sk".into(),document_revision:1,expansion:None,syntax_context:0};
            let tx=host.begin_parse("file:///macro",&context.document_id,1).unwrap();
            host.native_cancellation=Some(DocumentCancellationToken::after_native_probes(budget));
            let result=host.prepare_macro_declarations(&tx,&context,&source,Arc::clone(&tree),4096);
            let executed=host.components.iter().find(|c|c.manifest.component_id=="nlaocs.test.tree-macro").unwrap().store.data().guest_hook_calls>0;
            if matches!(result,Err(HostError::Cancelled(_))) {saw_executed_cancel|=executed;}
            else {assert!(result.unwrap().snapshot.complete);}
            no_live_declaration_resources(&host);host.native_cancellation=None;
            let retry=host.prepare_macro_declarations(&tx,&context,&source,tree,4096).unwrap();
            assert!(retry.snapshot.complete);assert_eq!(retry.snapshot.bindings.len(),1);
            no_live_declaration_resources(&host);assert!(tx.read_write_set().unwrap().writes.is_empty());tx.cancel().unwrap();
        }
        assert!(saw_executed_cancel,"exercise a callback that returned before cancellation, not only a preflight exit");
    }

    use super::*;
    fn tree()->ParserRawTree {skript_parser::parse_raw_tree(&MappedSource::identity("first:\n    stop\nsecond:\n    stop\n"),skript_parser::RawTreeOptions::for_skript_version(2,15))}
    fn record(owner:&str,root:u64,complete:bool)->WitMetadataEntry {WitMetadataEntry {owner_component_id:Some(owner.into()),key:OUTPUT_KEY.into(),
        value:serde_json::json!({"version":1,"complete":complete,"bindings":[{"root":root,"namespace":"test.macros","name":"unseen name"}]}).to_string()}}
    #[test]fn header_signature_ignores_body_only_text_and_trivia_but_not_header_inputs() {
        let raw=|text:&str|skript_parser::parse_raw_tree(&MappedSource::identity(text),skript_parser::RawTreeOptions::for_skript_version(2,15));
        let first=raw("option first:\n    get:\n        return 1\n");
        let body=raw("# different root IDs\noption first:\n    get:\n        arbitrary and much longer raw body\n");
        assert_eq!(root_signature(&first),root_signature(&body));
        assert_ne!(root_signature(&first),root_signature(&raw("option renamed:\n    get:\n        return 1\n")));
        assert_ne!(root_signature(&first),root_signature(&raw("option first:\n    get:\n        return 1\n    other:\n        body\n")));
    }
    #[test]fn successful_empty_is_distinct_from_missing_disabled_or_partial_provider() {
        let t=tree();let expected=BTreeSet::from(["owner".into()]);
        let empty=WitMetadataEntry {owner_component_id:Some("owner".into()),key:OUTPUT_KEY.into(),value:r#"{"version":1,"complete":true,"bindings":[]}"#.into()};
        assert!(freeze(&t,&[empty.clone()],&expected,false).unwrap().complete);
        assert!(!freeze(&t,&[],&expected,false).unwrap().complete);
        assert!(!freeze(&t,&[empty],&expected,true).unwrap().complete);
        assert!(!freeze(&t,&[record("owner",t.roots[0].get(),false)],&expected,false).unwrap().complete);
    }
    #[test]fn owners_compose_only_unavailability_without_a_value_winner() {
        let t=tree();let owners=BTreeSet::from(["one".into(),"two".into()]);
        let s=freeze(&t,&[record("one",t.roots[0].get(),true),record("two",t.roots[1].get(),true)],&owners,false).unwrap();
        assert!(s.complete);assert_eq!(s.bindings.len(),1);assert_eq!(s.bindings[0].owners,owners);assert_eq!(s.bindings[0].roots,vec![0,1]);
        let next=freeze(&t,&[record("one",t.roots[0].get(),true),record("two",t.roots[1].get(),true)],&s.bindings[0].owners,false).unwrap();
        assert!(s.same_meaning(&next));assert_ne!(s.bindings[0].id,next.bindings[0].id);
    }
    #[test]fn malformed_foreign_nested_and_oversized_declarations_fail_closed() {
        let t=tree();let expected=BTreeSet::from(["owner".into()]);
        for case in 0..7 {
            let mut r=record("owner",t.roots[0].get(),true);let mut w:serde_json::Value=serde_json::from_str(&r.value).unwrap();
            match case {
                0=>r.owner_component_id=Some("foreign".into()),1=>w["version"]=2.into(),
                2=>w["bindings"][0]["root"]=t.get(t.roots[0]).unwrap().children[0].get().into(),
                3=>w["bindings"][0]["name"]="x".repeat(257).into(),
                4=>w["bindings"][0]["namespace"]="".into(),
                5=>{let duplicate=w["bindings"][0].clone();w["bindings"].as_array_mut().unwrap().push(duplicate);},
                _=>w["value"]="not allowed".into(),
            }
            r.value=w.to_string();assert!(freeze(&t,&[r],&expected,false).is_err(),"case {case}");
        }
    }
    #[test]fn only_current_observations_can_report_a_binding_or_incomplete_namespace() {
        use crate::bindings::nlaocs::skript_parser_addon::types::MacroUnavailability as U;
        let t=tree();let s=freeze(&t,&[record("owner",t.roots[0].get(),true)],&BTreeSet::from(["owner".into()]),false).unwrap();
        let id=s.bindings[0].id;assert!(s.adopt(&Reads::default(),&[U::Binding(id)]).is_err());
        let reads=Reads {bindings:BTreeSet::from([id]),namespaces:BTreeMap::from([("test.macros".into(),true)])};
        assert_eq!(s.adopt(&reads,&[U::Binding(id)]).unwrap(),vec!["test.macros: unseen name"]);
        assert!(s.adopt(&reads,&[U::Binding(id+1)]).is_err());
        assert!(s.adopt(&reads,&[U::IncompleteNamespace("test.macros".into())]).is_err());
        let mut incomplete=s.clone();incomplete.complete=false;
        let reads=Reads {namespaces:BTreeMap::from([("test.macros".into(),false)]),..Default::default()};
        assert!(incomplete.adopt(&reads,&[U::IncompleteNamespace("test.macros".into())]).is_ok());
        assert!(incomplete.adopt(&Reads::default(),&[U::IncompleteNamespace("test.macros".into())]).is_err());
    }
}

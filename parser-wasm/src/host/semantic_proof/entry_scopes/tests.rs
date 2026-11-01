use super::*;
use crate::bindings::nlaocs::skript_parser_addon::types as wit;
use crate::host::semantic_proof::tests::{node,write};

fn provider()->ProviderStamp {ProviderStamp::entry_scope_test_fixture()}
fn raw()->(skript_parser::MappedSource,skript_parser::RawTree) {
    let source=skript_parser::MappedSource::identity("root:\n    check:\n        probe\n");
    let tree=skript_parser::parse_raw_tree(&source,skript_parser::RawTreeOptions::for_skript_version(2,15));(source,tree)
}
fn payload(tree:&skript_parser::RawTree,lease:&str)->StructurePayload {
    let root=tree.get(tree.roots[0]).unwrap();let entry=root.children[0];
    let metadata=vec![WitMetadataEntry {key:skript_parser::STRUCTURE_ENTRY_LAYOUT_METADATA.into(),owner_component_id:None,
        value:serde_json::json!({"version":1,"token":lease,"structureNode":root.id.get(),"entries":[]}).to_string()},
        WitMetadataEntry {key:skript_parser::STRUCTURE_ENTRY_PLANS_METADATA.into(),owner_component_id:Some("owner".into()),
        value:serde_json::json!({"version":1,"token":lease,"structureNode":root.id.get(),"entries":[{"node":entry.get(),"declarationIndex":0,
            "mode":"trigger","adoptUnknown":false,"context":{"eventClasses":null,"values":[]}}]}).to_string()},
        WitMetadataEntry {key:RECEIPT.into(),owner_component_id:None,value:"1".into()}];
    StructurePayload {input:"owned fixture".into(),body_tree:wit::RawTree {roots:vec![],nodes:vec![],diagnostics:vec![],indentation:None},
        context:wit::ParseContext {syntax_context:0,event_classes:vec![],section_stack:vec![],values:vec![]},
        timing:StructureTiming::EnterBody,type_options:vec![],candidate:wit::StructureCandidate {
            raw_node_id:root.id.get(),definition_id:"owned.header".into(),registration_id:"owned.header.0".into(),element_class:None,priority:0,registration_order:0,pattern_index:0,
            pattern:"root".into(),span:crate::host::mapped_span_to_wit(root.code_span.clone().unwrap()),
            declared_node_type:wit::StructureNodeType::Section,actual_node_type:wit::StructureNodeType::Section,
            regex_captures:vec![],tags:vec![],mark:0,marks:vec![],parsed_captures:vec![],body_mode:StructureBodyMode::Entries,
            child_node_ids:vec![entry.get()],entries:vec![],handler:None,declarations:vec![],metadata,
        }}
}
fn begin(registry:&mut Registry,tree:&skript_parser::RawTree,source:[u8;32],lease:&str,owner:&str,context:&ExpressionParseContext)->Option<u64> {
    let root=tree.get(tree.roots[0]).unwrap();let entry=tree.get(root.children[0]).unwrap();
    let mapped=entry.code_span.clone().unwrap();let span=skript_parser::MatchSpan {local_range:mapped.virtual_range,mapped};
    registry.begin(source,&skript_parser::StructureEntryScopeRequest {tree,root:root.id,entry:entry.id,declaration_index:0,
        owner,native_lease:lease,mode:skript_parser::SectionBodyMode::Trigger,context,entry_span:&span},true)
}
fn effect(tree:&skript_parser::RawTree)->EffectPayload {
    let root=tree.get(tree.roots[0]).unwrap();let entry=tree.get(root.children[0]).unwrap();let probe=tree.get(entry.children[0]).unwrap();
    let span=crate::host::mapped_span_to_wit(probe.code_span.clone().unwrap());
    EffectPayload {input:"owned fixture".into(),context:wit::ParseContext {syntax_context:0,event_classes:vec![],section_stack:vec![],values:vec![]},
        raw_node_id:probe.id.get(),span:span.clone(),timing:wit::EffectTiming::After,
        candidate:Some(wit::EffectCandidate {raw_node_id:probe.id.get(),definition_id:"owned.assignment".into(),registration_id:"owned.assignment.0".into(),
            element_class:None,priority:0,registration_order:0,pattern_index:0,pattern:"probe".into(),span,captures:vec![],tags:vec![],mark:0,marks:vec![],handler:None,metadata:vec![],parsed_captures:vec![]}),
        alternatives:vec![],failure:None,near_match:None}
}
fn register(registry:&mut Registry,tree:&skript_parser::RawTree,source:[u8;32],lease:&str) {
    registry.register_header(source,&payload(tree,lease),ExpressionParseContext::default(),provider(),&BTreeSet::from(["owner".into()]));
}
fn scope_reference(payload:&EffectPayload)->String {
    let m=payload.candidate.as_ref().unwrap().metadata.iter().find(|m|m.key==INPUT).unwrap();
    serde_json::from_str::<serde_json::Value>(&m.value).unwrap()["reference"].as_str().unwrap().into()
}

#[test]
fn selected_entry_lease_survives_discarded_alternatives_and_ends_on_failure() {
    for completed in [false,true] {
        let (source,tree)=raw();let source=source_identity(&source);let mut registry=Registry::default();
        register(&mut registry,&tree,source,"first");register(&mut registry,&tree,source,"discarded");
        registry.discard(tree.roots[0].get(),"discarded");
        let ticket=begin(&mut registry,&tree,source,"first","owner",&ExpressionParseContext::default()).expect("later alternative must not erase selected lease");
        let operation=Operation {source,frames:vec![ticket]};let mut effect=effect(&tree);
        registry.issue(&operation,"core",&mut effect).unwrap();let reference=scope_reference(&effect);
        assert!(registry.claim_current(&reference,"core",&effect,&operation));
        assert!(!registry.claim_current(&reference,"other",&effect,&operation));
        assert!(!registry.claim_current(&reference,"core",&effect,&Operation {source,frames:vec![]}));
        assert!(!registry.claim_current(&reference,"core",&effect,&Operation {source:[0;32],frames:vec![ticket]}));
        let mut moved=effect.clone();moved.candidate.as_mut().unwrap().span.virtual_range.start+=1;
        assert!(!registry.claim_current(&reference,"core",&moved,&operation));
        registry.finish(ticket,completed);assert!(!registry.claim_current(&reference,"core",&effect,&operation));
        assert!(registry.issue(&operation,"core",&mut effect).is_none());
        assert!(registry.witnesses(&AstTree {roots:vec![],nodes:vec![]},&BTreeMap::new(),&BTreeSet::new()).inputs.is_empty(),"ended is not final adoption");
    }
}
#[test]
fn stale_foreign_context_missing_or_exhausted_scopes_do_not_grant_inputs() {
    let (source,tree)=raw();let source=source_identity(&source);let mut registry=Registry::default();register(&mut registry,&tree,source,"first");
    let context=ExpressionParseContext::default();
    assert!(begin(&mut registry,&tree,[0;32],"first","owner",&context).is_none());
    assert!(begin(&mut registry,&tree,source,"stale","owner",&context).is_none());
    assert!(begin(&mut registry,&tree,source,"first","foreign",&context).is_none());
    let mut changed=context.clone();changed.event_classes.push(syntaxes::ClassName("foreign.Context".into()));
    assert!(begin(&mut registry,&tree,source,"first","owner",&changed).is_none());
    let ticket=begin(&mut registry,&tree,source,"first","owner",&context).unwrap();let op=Operation {source,frames:vec![ticket]};
    let mut output=effect(&tree);registry.issue(&op,"core",&mut output).unwrap();let reference=scope_reference(&output);
    registry.charge(MAX_BYTES);assert!(!registry.claim_current(&reference,"core",&output,&op));assert!(registry.issue(&op,"core",&mut output).is_none());
}
#[test]
fn entry_exit_receipts_must_include_the_original_retained_enter_chain() {
    let mut receipts=ReceiptRegistry::default();let mut header=node(0);
    let enter=write(&mut receipts,&mut header,"owner");let exit=write(&mut receipts,&mut header,"owner");
    let enter=BTreeSet::from([enter.semantic_receipt.unwrap()]);let exit=BTreeSet::from([exit.semantic_receipt.unwrap()]);
    let retained=enter.union(&exit).copied().collect();
    assert!(receipt_chain(&exit,&enter,&receipts.receipts,&retained));
    assert!(!receipt_chain(&exit,&BTreeSet::from([999]),&receipts.receipts,&retained));
    assert!(!receipt_chain(&exit,&enter,&receipts.receipts,&exit));
    receipts.receipts.get_mut(enter.first().unwrap()).unwrap().unbound=true;
    assert!(!receipt_chain(&exit,&enter,&receipts.receipts,&retained));
}
#[test]
fn scope_witness_only_admits_the_exact_header_owner_for_one_selected_operation() {
    let mut witnesses=Witnesses::default();witnesses.reverse=BTreeMap::from([(2,vec![1]),(1,vec![0])]);
    witnesses.inputs.insert("current".into(),Authorization {consumer:"core".into(),operation:2,headers:BTreeMap::from([(0,"owner".into())])});
    let own=BTreeMap::from([(0,BTreeSet::from(["owner".into()])),(1,BTreeSet::new()),(2,BTreeSet::from(["core".into()]))]);
    assert!(witnesses.authorizes("current",2,"core",&own));assert!(!witnesses.authorizes("current",1,"core",&own));
    assert!(!witnesses.authorizes("current",2,"other",&own));assert!(!witnesses.authorizes("forged",2,"core",&own));
    for id in [0,1,2] {let mut changed=own.clone();changed.get_mut(&id).unwrap().insert("unrelated".into());assert!(!witnesses.authorizes("current",2,"core",&changed));}
}
#[test]
fn transient_scope_input_is_removed_but_composition_reference_remains() {
    let (_,tree)=raw();let mut payload=effect(&tree);let candidate=payload.candidate.as_mut().unwrap();
    candidate.metadata.push(WitMetadataEntry {key:INPUT.into(),owner_component_id:None,value:"input".into()});
    candidate.metadata.push(WitMetadataEntry {key:super::super::value_facts::COMPOSE.into(),owner_component_id:Some("core".into()),value:"composition".into()});
    let mut output=HookOutput {decision:crate::host::HookDecision::ContinueProcessing,replacement:Some(HookPayload::Effect(payload)),effects:crate::host::empty_effects()};
    strip_input(&mut output);let Some(HookPayload::Effect(payload))=output.replacement else{panic!()};
    assert!(payload.candidate.unwrap().metadata.iter().all(|m|m.key!=INPUT));
}


#[test]
fn a_live_real_reference_requires_the_private_current_dispatch_operation() {
    let (source,tree)=raw();let source=source_identity(&source);let mut scopes=Registry::default();
    register(&mut scopes,&tree,source,"current");
    let ticket=begin(&mut scopes,&tree,source,"current","owner",&ExpressionParseContext::default()).unwrap();
    let operation=Operation {source,frames:vec![ticket]};let mut payload=effect(&tree);
    let mut receipts=ReceiptRegistry::default();
    let (_,fixture,_)=crate::host::capture_view_tests::fixture();
    let mut operand=fixture.children[0].children[0].children[0].clone();
    operand.children.clear();operand.captures.clear();operand.public_data.clear();operand.metadata.clear();operand.function=None;operand.effects=None;
    operand.kind=skript_parser::ExpressionNodeKind::Registered {definition_id:"owned.value".into(),registration_id:"owned.value.0".into(),
        element_class:syntaxes::ClassName("owned.Value".into()),pattern_index:0};
    let root=tree.get(tree.roots[0]).unwrap();let entry=tree.get(root.children[0]).unwrap();let probe=tree.get(entry.children[0]).unwrap();
    let mapped=probe.code_span.clone().unwrap();operand.span=skript_parser::MatchSpan {local_range:mapped.virtual_range,mapped};
    let mut n=node(0);n.metadata.push(WitMetadataEntry {key:"owned.input".into(),value:"current".into(),owner_component_id:Some("core".into())});
    let accepted=write(&mut receipts,&mut n,"core");assert!(accepted.semantic_receipt.is_some());
    operand.metadata=crate::host::metadata_entries(n.metadata).unwrap();
    let seal=receipts.value_facts.seal(&receipts,&operand).expect("real current private subject");
    let projection=crate::host::ast_tree::expression_subject(&operand);
    let mut summary=projection.nodes.iter().find(|n|n.id==projection.roots[0]).unwrap().summary.clone().unwrap();summary.metadata.push(seal);
    let candidate=payload.candidate.as_mut().unwrap();
    candidate.parsed_captures.push(wit::ParsedCapture {capture_index:0,parser_id:"host.expression".into(),status:wit::ParseResultStatus::Success,
        text:"probe".into(),span:candidate.span.clone(),expected_types:vec![],summary:Some(summary),attachments:vec![],diagnostics:vec![]});
    scopes.issue(&operation,"core",&mut payload).unwrap();let reference=scope_reference(&payload);
    assert!(scopes.claim_current(&reference,"core",&payload,&operation));
    payload.candidate.as_mut().unwrap().metadata.extend([
        WitMetadataEntry {key:"owned.fact".into(),owner_component_id:Some("core".into()),value:"current selected fact".into()},
        WitMetadataEntry {key:super::super::value_facts::COMPOSE.into(),owner_component_id:Some("core".into()),
            value:serde_json::json!({"version":2,"contextInput":reference,"metadataKeys":["owned.fact"],"inputs":[{"capture":0,"publicSchemas":[]}]}).to_string()},
    ]);
    let payload=HookPayload::Effect(payload);
    receipts.value_facts.record(100,&payload,"core");
    assert!(!receipts.value_facts.has_receipt(100),"a real live ref must not authorize a public/additional dispatch");
    receipts.value_facts.record_scoped(101,&payload,"core",&scopes,&operation);
    assert!(receipts.value_facts.has_receipt(101),"the same payload and private current operation must be usable");
    // A real live scope still admits only explicit, current, bounded input facts.
    for mode in 0..14 {
        let HookPayload::Effect(mut effect)=payload.clone()else{unreachable!()};
        let candidate=effect.candidate.as_mut().unwrap();
        let mut draft:serde_json::Value=serde_json::from_str(&candidate.metadata.iter().find(|m|m.key==super::super::value_facts::COMPOSE).unwrap().value).unwrap();
        draft["inputs"][0]["metadataKeys"]=serde_json::json!(["owned.input"]);
        let summary=candidate.parsed_captures[0].summary.as_mut().unwrap();
        match mode {
            0=>{},
            1=>draft["inputs"][0]["metadataKeys"]=serde_json::json!(["missing"]),
            2=>draft["inputs"][0]["metadataKeys"]=serde_json::json!(["owned.input","owned.input"]),
            3=>summary.metadata.iter_mut().find(|m|m.key=="owned.input").unwrap().owner_component_id=Some("foreign".into()),
            4=>{let m=summary.metadata.iter().find(|m|m.key=="owned.input").unwrap().clone();summary.metadata.push(m);},
            5=>summary.metadata.iter_mut().find(|m|m.key=="owned.input").unwrap().value="x".repeat(16*1024),
            6=>draft["inputs"][0]["metadataKeys"]=serde_json::json!(["parser.native-semantic-input.v1"]),
            7=>draft["inputs"][0]["capture"]=serde_json::json!(99),
            8=>{draft["version"]=1.into();draft.as_object_mut().unwrap().remove("contextInput");draft["inputs"][0]["metadataKeys"]=serde_json::json!([]);},
            9=>{draft["version"]=1.into();draft.as_object_mut().unwrap().remove("contextInput");},
            10=>draft["inputs"][0]["metadataKeys"]=serde_json::Value::Null,
            11=>draft["inputs"][0]["metadataKeys"]=serde_json::json!((0..17).map(|i|format!("key-{i}")).collect::<Vec<_>>()),
            12=>{
                summary.metadata.iter_mut().find(|m|m.key=="owned.input").unwrap().value="x".repeat(8200);
                let mut second=candidate.parsed_captures[0].clone();second.capture_index=1;candidate.parsed_captures.push(second);
                let mut claim=draft["inputs"][0].clone();claim["capture"]=1.into();draft["inputs"].as_array_mut().unwrap().push(claim);
            },
            _=>summary.metadata.iter_mut().find(|m|m.key=="owned.input").unwrap().value="x".repeat(16*1024-"owned.input".len()-"core".len()),
        }
        candidate.metadata.iter_mut().find(|m|m.key==super::super::value_facts::COMPOSE).unwrap().value=draft.to_string();
        // The current native request, not the guest metadata, supplies admission.
        scopes.issue(&operation,"core",&mut effect).unwrap();
        let fresh=scope_reference(&effect);
        let candidate=effect.candidate.as_mut().unwrap();
        if draft["version"]==2 {draft["contextInput"]=fresh.into();}
        candidate.metadata.iter_mut().find(|m|m.key==super::super::value_facts::COMPOSE).unwrap().value=draft.to_string();
        let token=200+mode;receipts.value_facts.record_scoped(token,&HookPayload::Effect(effect),"core",&scopes,&operation);
        assert_eq!(receipts.value_facts.has_receipt(token),mode==0||mode==13,"selected input record mode {mode}");
    }
    scopes.finish(ticket,true);receipts.value_facts.record_scoped(102,&payload,"core",&scopes,&operation);
    assert!(!receipts.value_facts.has_receipt(102),"completion has expired the live input");
}

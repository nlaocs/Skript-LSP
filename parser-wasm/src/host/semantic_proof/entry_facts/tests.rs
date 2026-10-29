use super::*;
use crate::host::SyntaxKind;
use crate::host::semantic_proof::tests::{node,write};

fn fact()->WitMetadataEntry {WitMetadataEntry {key:"owned.entry".into(),value:"nullable text entry".into(),owner_component_id:Some("header".into())}}
fn fixture()->(ReceiptRegistry,AstTree,Vec<HookCall>) {
    let (_,root,_)=crate::host::capture_view_tests::fixture();
    let expression=&root.children[0].children[0].children[0];
    let mut header=crate::host::ast_tree::expression_subject(expression).nodes.remove(0);
    header.id=0;header.kind=SyntaxKind::Structure;header.syntax_id="header.0".into();header.text="function sample(value: text)".into();
    header.children=vec![1];header.captures.clear();header.function=None;header.metadata=vec![fact()];
    let summary=header.summary.as_mut().unwrap();summary.kind="structure".into();summary.definition_id=Some("header".into());
    summary.registration_id=Some("header.0".into());summary.pattern_index=Some(0);
    let mut body=node(1);let mut registry=ReceiptRegistry::default();
    let entry=write(&mut registry,&mut header,"header");
    registry.entry_facts.entries.insert(entry.semantic_receipt.unwrap(),Entry {issuer:"header".into(),span:header.span.clone(),header:header.text.clone(),
        definition:"header".into(),registration:"header.0".into(),pattern:0,metadata:vec![fact()]});
    let body_call=write(&mut registry,&mut body,"body");
    (registry,AstTree {roots:vec![0],nodes:vec![header,body]},vec![entry,body_call])
}
fn certified(registry:&ReceiptRegistry,tree:&AstTree,calls:&[HookCall])->bool {
    let proof=SemanticProofRegistry::capture(tree,Some(registry),calls);let view=proof.view(tree);
    view.composed_metadata_valid(&tree.nodes[0],tree.nodes[0].metadata.iter().find(|m|m.key=="owned.entry").unwrap())
}

#[test]
fn entry_composition_keeps_whole_body_issuer_strict() {
    let (registry,tree,calls)=fixture();
    let proof=SemanticProofRegistry::capture(&tree,Some(&registry),&calls);let view=proof.view(&tree);
    assert!(view.native_valid(&tree.nodes[0]));assert!(view.unique_issuer(&tree.nodes[0]).is_none());
    assert!(certified(&registry,&tree,&calls));
    let mut unrelated=fact();unrelated.key="owned.body-purity".into();
    assert!(!view.composed_metadata_valid(&tree.nodes[0],&unrelated));
    assert!(!view.composed_public_data_valid(&tree.nodes[0],"owned.entry"));
}
#[test]
fn exit_receipt_follows_only_current_header_parents() {
    let (mut registry,mut tree,mut calls)=fixture();
    let exit=write(&mut registry,&mut tree.nodes[0],"header");calls.push(exit);
    assert!(certified(&registry,&tree,&calls));
    registry.receipts.get_mut(&calls[0].semantic_receipt.unwrap()).unwrap().valid=false;
    assert!(!certified(&registry,&tree,&calls));
}
#[test]
fn own_ancestor_global_and_unbound_writers_are_not_future_body_writers() {
    for mode in 0..4 {
        let (mut registry,mut tree,mut calls)=fixture();
        match mode {
            0=>{let c=write(&mut registry,&mut tree.nodes[0],"other-header");calls.push(c);},
            1=>{let mut parent=node(2);parent.children=vec![0];let c=write(&mut registry,&mut parent,"ancestor");tree.roots=vec![2];tree.nodes.push(parent);calls.push(c);},
            2=>{let c=write(&mut registry,&mut node(9),"unattached");calls.push(c);},
            _=>registry.receipts.get_mut(&calls[1].semantic_receipt.unwrap()).unwrap().unbound=true,
        }
        assert!(!certified(&registry,&tree,&calls),"conflict {mode}");
    }
}
#[test]
fn changed_or_ambiguous_header_and_facts_fail_closed() {
    for mode in 0..7 {
        let (registry,mut tree,calls)=fixture();
        match mode {
            0=>tree.nodes[0].metadata[0].value="different".into(),
            1=>tree.nodes[0].metadata[0].owner_component_id=Some("forged".into()),
            2=>tree.nodes[0].text.push('x'),
            3=>tree.nodes[0].span.virtual_range.end+=1,
            4=>tree.nodes[0].summary.as_mut().unwrap().registration_id=Some("other".into()),
            5=>tree.nodes[0].summary.as_mut().unwrap().pattern_index=Some(1),
            _=>{let mut duplicate=tree.nodes[0].clone();duplicate.id=2;duplicate.children.clear();tree.nodes.push(duplicate);tree.roots.push(2);},
        }
        assert!(!certified(&registry,&tree,&calls),"mutation {mode}");
    }
}
#[test]
fn cancelled_missing_receipt_old_source_and_mutable_projection_reject() {
    let (registry,tree,calls)=fixture();
    assert!(!certified(&registry,&tree,&calls[1..]));
    assert!(!certified(&ReceiptRegistry::default(),&tree,&calls));
    let source=skript_parser::MappedSource::identity("function sample(value: text):\n");
    let proof=SemanticProofRegistry::capture(&tree,Some(&registry),&calls).bind_source(&source);
    let fact=&tree.nodes[0].metadata[0];
    assert!(proof.view_for_source(&tree,&source).composed_metadata_valid(&tree.nodes[0],fact));
    assert!(!proof.view_for_source(&tree,&skript_parser::MappedSource::identity("edited 😀 source")).composed_metadata_valid(&tree.nodes[0],fact));
    let mut edited=tree.clone();edited.nodes[1].text.push('x');
    assert!(!proof.view(&edited).composed_metadata_valid(&edited.nodes[0],&edited.nodes[0].metadata[0]));
}
#[test]
fn entry_request_is_bounded_owner_specific_and_not_a_protected_metadata_claim() {
    let metadata=||vec![fact(),WitMetadataEntry {key:DRAFT.into(),value:r#"{"version":1,"metadataKeys":["owned.entry"]}"#.into(),owner_component_id:Some("header".into())}];
    assert!(selected(&metadata(),"header").is_some());
    assert!(selected(&metadata(),"other").is_none());
    for mode in 0..6 {
        let mut m=metadata();match mode {
            0=>m[1].value=r#"{"version":2,"metadataKeys":["owned.entry"]}"#.into(),
            1=>m[1].value=r#"{"version":1,"metadataKeys":["owned.entry","owned.entry"]}"#.into(),
            2=>m[1].value=r#"{"version":1,"metadataKeys":["parser.protected"]}"#.into(),
            3=>m[0].value="x".repeat(16*1024+1),
            4=>m.push(m[0].clone()),
            _=>m[0].owner_component_id=None,
        };assert!(selected(&m,"header").is_none(),"draft {mode}");
    }
}
#[test]
fn entry_authority_is_fresh_even_when_a_header_local_draft_is_cached() {
    let (registry,tree,calls)=fixture();
    assert!(registry.export_body(&calls,&[]).is_none());
    let mut payload=crate::host::semantic_proof::tests::payload();
    if let HookPayload::Matching(p)=&mut payload {p.metadata.push(WitMetadataEntry {key:DRAFT.into(),value:"{}".into(),owner_component_id:Some("header".into())});}
    assert!(value_facts::volatile(&payload));assert!(with_payload_receipts_normalized(&mut payload, |_|()).is_none());
    assert!(registry.export_body(&[],&[(DRAFT.into(),"{}".into())]).is_none());
    assert!(tree.nodes[0].children.contains(&1));
    let header=entry_payload();assert!(!value_facts::volatile(&header));
    let mut output=crate::host::HookOutput {decision:crate::host::HookDecision::ContinueProcessing,
        replacement:Some(header.clone()),effects:crate::host::empty_effects()};
    assert!(normalize_output_receipts(&header,&mut output));
    let HookPayload::Structure(mut exit)=header else{unreachable!()};
    exit.timing=crate::host::WitStructureTiming::ExitBody;
    assert!(value_facts::volatile(&HookPayload::Structure(exit)));
}

fn entry_payload()->HookPayload {
    use crate::host::*;
    use crate::bindings::nlaocs::skript_parser_addon::types as wit;
    let input="function sample(value: text):\n    send \"x\"\n";
    let source=MappedSource::identity(input);
    let tree=skript_parser::parse_raw_tree(&source,skript_parser::RawTreeOptions::for_skript_version(2,15));
    let header_end=input.find(':').unwrap_or(0); // Use the final colon, not the parameter separator.
    let header_end=input[..input.find('\n').unwrap()].rfind(':').unwrap().max(header_end);
    let span=mapped_span_to_wit(source.map_range(ParserTextRange::new(0,header_end)).unwrap());
    HookPayload::Structure(WitStructurePayload {input:input.into(),body_tree:parser_raw_tree_to_wit(&tree),
        context:WitParseContext {syntax_context:0,event_classes:vec![],section_stack:vec![],values:vec![]},timing:WitStructureTiming::EnterBody,type_options:vec![],
        candidate:WitStructureCandidate {raw_node_id:tree.roots[0].get(),definition_id:"header".into(),registration_id:"header.0".into(),element_class:None,
            priority:0,registration_order:0,pattern_index:0,pattern:"fixture".into(),span:span.clone(),declared_node_type:WitStructureNodeType::Section,actual_node_type:WitStructureNodeType::Section,
            regex_captures:vec![],tags:vec![],mark:0,marks:vec![],parsed_captures:vec![],body_mode:WitStructureBodyMode::Trigger,child_node_ids:vec![],entries:vec![],handler:None,
            metadata:vec![fact(),WitMetadataEntry {key:DRAFT.into(),value:r#"{"version":1,"metadataKeys":["owned.entry"]}"#.into(),owner_component_id:Some("header".into())}],
            declarations:vec![WitParserDeclaration::DocumentFunction(WitFunctionDeclaration {source:input.into(),span:WitTextRange {start:0,end:header_end as u64},
                scope:wit::FunctionDeclarationScope::Global,name:"sample".into(),parameters:vec![],returns:wit::FunctionReturnDeclaration {class_name:None,single:true},metadata:vec![]})]}})
}
#[test]
fn record_requires_current_declaration_source_geometry_and_native_header_owner() {
    let mut registry=Registry::default();registry.record(1,&entry_payload(),"header");assert!(registry.has_receipt(1));
    for mode in 0..10 {
        let mut payload=entry_payload();let HookPayload::Structure(p)=&mut payload else{unreachable!()};
        match mode {
            0=>{let crate::bindings::nlaocs::skript_parser_addon::types::ParserDeclaration::DocumentFunction(d)=&mut p.candidate.declarations[0];d.source="old source".into();},
            1=>{let crate::bindings::nlaocs::skript_parser_addon::types::ParserDeclaration::DocumentFunction(d)=&mut p.candidate.declarations[0];d.span.end=p.input.len() as u64;},
            2=>{let crate::bindings::nlaocs::skript_parser_addon::types::ParserDeclaration::DocumentFunction(d)=&mut p.candidate.declarations[0];d.span.start=d.span.end;},
            3=>p.candidate.raw_node_id=u64::MAX,
            4=>{let mut raw=p.body_tree.nodes[0].clone();raw.id=u64::MAX;p.body_tree.nodes.push(raw);},
            5=>p.timing=crate::bindings::nlaocs::skript_parser_addon::types::StructureTiming::ExitBody,
            6=>p.candidate.body_mode=crate::bindings::nlaocs::skript_parser_addon::types::StructureBodyMode::Raw,
            7=>p.candidate.declarations.clear(),
            8=>p.candidate.span.origins[0].kind=crate::host::WitOriginKind::Replaced,
            _=>p.candidate.metadata[1].owner_component_id=None,
        }
        let mut registry=Registry::default();registry.record(1,&payload,"header");assert!(!registry.has_receipt(1),"admission {mode}");
    }
}


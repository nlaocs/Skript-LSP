//! Native unavailability protocol over actual test Components; never GET execution.
use parser_wasm::host::{HostConfig,HostError,InvocationContext,ParserHost,TreeMacroRequest,DependencyUnavailableKind};
use skript_parser::{MappedSource,RawTreeOptions,parse_raw_tree};
const CORE:&[u8]=include_bytes!("../../artifacts/core-library.wasm");
const ADDON:&[u8]=include_bytes!("../../artifacts/tree-macro-addon.wasm");
fn host()->ParserHost {
    let mut h=ParserHost::new(CORE,HostConfig {runtime_profile:parser_wasm::RuntimeProfile {skript_version:Some("2.16.0".into()),..Default::default()},..Default::default()}).unwrap();
    h.load_addon(ADDON).unwrap();h
}
fn context(revision:u64)->InvocationContext {InvocationContext {invocation_id:revision,subscription_id:String::new(),document_id:"file:///macro/test.sk".into(),document_revision:revision,expansion:None,syntax_context:0}}
fn request(revision:u64,text:&str)->TreeMacroRequest {
    let source=MappedSource::identity(text);let tree=parse_raw_tree(&source,RawTreeOptions::for_skript_version(2,16));
    TreeMacroRequest {context:context(revision),source,tree}
}

#[test]
fn readonly_discovery_restores_stateful_tree_access_and_unavailability_rolls_back_current_writes() {
    let mut h=host();let text="declare-unavailable computed\nconsume-unavailable\n";
    let tx=h.begin_parse("file:///macro","file:///macro/test.sk",1).unwrap();
    let result=h.expand_tree_in_parse(&tx,request(1,text)).unwrap();
    assert!(result.discovery_complete);assert!(result.declaration_failures.is_empty());assert!(result.failures.is_empty(),"{:?}",result.failures);
    assert_eq!(result.unavailable.len(),1);assert_eq!(result.unavailable[0].kind,DependencyUnavailableKind::SourceMacro);
    assert_eq!(result.source.virtual_source(),text);
    let writes=tx.read_write_set().unwrap().writes;
    assert!(writes.iter().any(|w|w.key.contains("declare-unavailable")),"ordinary tree writes follow readonly discovery");
    assert!(!writes.iter().any(|w|w.key.contains("consume-unavailable")),"unavailable callback's State cannot survive");
    assert!(result.calls.iter().filter(|c|c.target==result.tree.roots[1]).all(|c|!c.accepted));tx.cancel().unwrap();
    let text="declare-unavailable computed\nunmodified owner:\n    consume-unavailable\n    one\n";
    let tx=h.begin_parse("file:///macro","file:///macro/test.sk",2).unwrap();
    let result=h.expand_tree_in_parse(&tx,request(2,text)).unwrap();
    assert!(result.failures.is_empty()&&result.declaration_failures.is_empty());assert_eq!(result.unavailable.len(),1);
    assert!(!result.unavailable_all);assert!(result.source.virtual_source().contains("one-expanded"),"unrelated original sibling edit survives");
    assert!(result.source.virtual_source().contains("consume-unavailable"));
    let writes=tx.read_write_set().unwrap().writes;
    assert!(writes.iter().any(|w|w.key=="tree.expand:one"));
    assert!(!writes.iter().any(|w|w.key.contains("consume-unavailable")));tx.cancel().unwrap();
}

#[test]
fn generated_unavailable_restores_original_target_source_state_and_acceptance() {
    for generated in ["generate-unavailable","generate-quoted-unavailable","generate-nested-unavailable"] {
        let mut h=host();let text=format!("declare-unavailable computed\n{generated}\n");
        let tx=h.begin_parse("file:///macro","file:///macro/test.sk",1).unwrap();
        let result=h.expand_tree_in_parse(&tx,request(1,&text)).unwrap();
        assert!(result.failures.is_empty(),"{generated}: {:?}",result.failures);assert!(result.declaration_failures.is_empty());
        assert_eq!(result.unavailable.len(),1,"generated quoted markers must not bypass dependency checks");
        assert_eq!(result.source.virtual_source(),text,"outer edit is withdrawn");
        assert!(tx.read_write_set().unwrap().writes.iter().all(|w|w.key.contains("declare-unavailable")),"intermediate generated Sections also roll back");
        assert!(result.calls.iter().all(|c|c.target==result.tree.roots[0]||!c.accepted&&c.expansion.is_none()),"all generated callback suffixes are withdrawn");tx.cancel().unwrap();
    }
}

#[test]
fn generated_declaration_change_withdraws_all_tree_edits_and_state() {
    let mut h=host();let text="one\ngenerate-declaration\n";
    let tx=h.begin_parse("file:///macro","file:///macro/test.sk",1).unwrap();
    let result=h.expand_tree_in_parse(&tx,request(1,text)).unwrap();
    assert!(result.declaration_failures.is_empty(),"{:?}",result.declaration_failures);
    assert!(result.unavailable_all);assert!(!result.discovery_complete);assert_eq!(result.source.virtual_source(),text);
    assert_eq!(result.tree.roots.len(),2);assert!(result.calls.iter().all(|c|!c.accepted&&c.expansion.is_none()));
    assert!(tx.read_write_set().unwrap().writes.is_empty());assert_eq!(result.unavailable.len(),1);
    assert_eq!(result.unavailable[0].span.virtual_range.end,text.len()as u64);tx.cancel().unwrap();
}

#[test]
fn explicit_empty_incomplete_fault_and_missing_response_never_share_completeness() {
    for (revision,text,complete,failed) in [(1,"ordinary\n",true,false),(2,"incomplete-declarations\nconsume-incomplete\n",false,false),
        (3,"missing-declarations\n",false,true),(4,"trap-declarations\n",false,true)] {
        let mut h=host();let tx=h.begin_parse("file:///macro","file:///macro/test.sk",revision).unwrap();
        let result=h.expand_tree_in_parse(&tx,request(revision,text)).unwrap();
        assert_eq!(result.discovery_complete,complete,"{text}");assert_eq!(!result.declaration_failures.is_empty(),failed,"{text}");
        if text.contains("consume-incomplete") {assert_eq!(result.unavailable.len(),1);assert!(result.failures.is_empty());}
        tx.cancel().unwrap();
    }
}

#[test]
fn unqueried_stale_and_falsely_incomplete_dependencies_are_rejected_not_certified() {
    let mut h=host();
    for (revision,operation) in [(1,"remember-binding"),(2,"stale-binding"),(3,"unqueried-binding"),(4,"false-incomplete")] {
        let text=format!("declare-unavailable computed\n{operation}\n");
        let tx=h.begin_parse("file:///macro","file:///macro/test.sk",revision).unwrap();
        let result=h.expand_tree_in_parse(&tx,request(revision,&text)).unwrap();
        assert!(result.declaration_failures.is_empty());
        if revision==1 {assert!(result.failures.is_empty());}else{
            assert!(result.failures.iter().any(|f|matches!(f.error,HostError::InvalidTreeMacroOutput {..})),"{operation}: {:?}",result.failures);
            assert!(result.unavailable.is_empty(),"invalid references are failures, not authenticated dependency records");
        }
        tx.cancel().unwrap();
    }
}

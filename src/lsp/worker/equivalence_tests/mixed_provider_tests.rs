//! Real Core + unrelated State writers retain only genuinely independent facts.
use super::*;
use parser_wasm::state::{NamespaceVisibility,StateEncoding,StateScope,StateValue};
const ADDONS:[&[u8];2]=[
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),"/artifacts/semantic-state-addon-a.wasm")),
    include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),"/artifacts/semantic-state-addon-b.wasm")),
];
const SOURCE:&str="on load:\n    send \"setup\" to console\non join:\n    set {_x} to 1\n    set {_text} to \"hello\"\n    send {_x}\n";
fn mixed(count:usize,mode:&str)->ParserAnalyzer {
    let mut analyzer=analyzer();
    for (i,addon) in ADDONS.iter().take(count).enumerate() {
        analyzer.host.load_addon(addon).unwrap();
        let config=analyzer.host.begin_parse("file:///workspace",&format!("file:///workspace/config-{i}"),1).unwrap();
        let id=if i==0 {"test.semantic-state-a"}else{"test.semantic-state-b"};
        let mut call=config.begin_invocation(id).unwrap();
        call.put(StateScope::Project,NamespaceVisibility::Private,"semantic-state","mode",StateValue::new("test.semantic-state",StateEncoding::Raw,mode.as_bytes())).unwrap();
        call.commit().unwrap();config.commit().unwrap();
    }
    analyzer
}
fn hints(analysis:&Analysis)->serde_json::Value {
    serde_json::to_value(analysis.semantic.as_ref().unwrap().write_type_hints()).unwrap()
}
#[test]
fn isolated_private_writes_preserve_core_types_with_one_or_two_other_addons() {
    let baseline=analyze(&mut analyzer(),SOURCE,1);
    let expected=hints(&baseline);assert_eq!(expected.as_array().unwrap().len(),2,"{expected}");
    let at=position(SOURCE,SOURCE.find("{_x}").unwrap()+2);
    assert!(matches!(baseline.semantic.as_ref().unwrap().type_at(at),Some((_,semantic::InferredType::Values {classes,..})) if classes.contains("java.lang.Long")));
    for count in 1..=2 {
        let actual=analyze(&mut mixed(count,"local"),SOURCE,1);
        assert_eq!(hints(&actual),expected,"count={count}");
        assert_eq!(diagnostics(&actual).0,diagnostics(&baseline).0);
        let read=position(SOURCE,SOURCE.rfind("{_x}").unwrap()+2);
        assert_eq!(actual.semantic.as_ref().unwrap().hover(read),baseline.semantic.as_ref().unwrap().hover(read));
    }
}
#[test]
fn shared_durable_and_caught_failed_access_do_not_borrow_core_authority() {
    for mode in ["shared","document","caught-error"] {
        let actual=analyze(&mut mixed(1,mode),SOURCE,1);
        assert!(diagnostics(&actual).0.is_empty(),"mode={mode}: {:?}",diagnostics(&actual).0);
        assert!(hints(&actual).as_array().unwrap().is_empty(),"mode={mode}: {}",hints(&actual));
    }
}
#[test]
fn mixed_private_state_results_follow_current_source_edits_and_reopen() {
    let mut warm=mixed(2,"local");let initial=analyze(&mut warm,SOURCE,1);assert_eq!(hints(&initial).as_array().unwrap().len(),2);
    let changed=SOURCE.replace("to 1","to \"日本😀\"");
    let edited=analyze(&mut warm,&changed,2);let fresh=analyze(&mut mixed(2,"local"),&changed,1);
    assert_eq!(hints(&edited),hints(&fresh));assert_ne!(hints(&edited),hints(&initial));
    let at=position(&changed,changed.find("{_x}").unwrap()+2);
    assert!(matches!(edited.semantic.as_ref().unwrap().type_at(at),Some((_,semantic::InferredType::Values {classes,..})) if classes.contains("java.lang.String")));
    let uri="file:///workspace/output-equivalence.sk";
    assert!(warm.host.release_document("file:///workspace",&document_key(uri),2).unwrap());
    let reopened=analyze(&mut warm,SOURCE,3);assert_eq!(hints(&reopened),hints(&initial));
}

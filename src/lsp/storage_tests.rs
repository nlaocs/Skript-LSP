//! Source-backed storage model regressions; public fixtures only.
use super::{semantic, positions::LineIndex};
use lsp_types::{InlayHintLabel, Position};
use parser_wasm::{DocumentParseRequest, DocumentParserConfig, RuntimeProfile, host::{HostConfig, ParserHost}};
use std::{path::Path, sync::{Arc, LazyLock}};
use syntaxes::Catalog;

fn catalog() -> Arc<Catalog> {
    static CATALOG: LazyLock<Arc<Catalog>> = LazyLock::new(|| Arc::new(ssg::load(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("parser-wasm/tests/data/type-parser-versions/skript-2.15.4-storage-domain")
    ).unwrap().into_catalog()));
    Arc::clone(&CATALOG)
}

fn parse(source: &str) -> (parser_wasm::DocumentParseResult, semantic::Document) {
    let mut host = ParserHost::new(crate::core_library_component(), HostConfig {
        syntax_catalog: Some(catalog()), runtime_profile:RuntimeProfile {skript_version:Some("2.15.4".into()),..RuntimeProfile::default()},
        ..HostConfig::default()
    }).unwrap();
    let result = host.parse_document(DocumentParseRequest::new("file:///storage-public", "file:///storage-public/test.sk",1,source),DocumentParserConfig::default()).unwrap();
    assert!(result.component_failures.is_empty(),"source: {source}\n{:?}",result.component_failures);
    assert!(result.syntax.diagnostics.is_empty(),"{:?}",result.syntax.diagnostics);
    let document=semantic::Document::from_result(&result,Arc::from(source),"file:///storage-public/test.sk".parse().unwrap(),catalog()).unwrap();
    (result,document)
}

#[test]
fn standard_storage_model_write_sites_follow_assignments_and_keep_reads_hint_free() {
    let cases = [
        ("join-single", "on join:\n    set {_target} to player\n    send {_target}\n",vec![(1,": player?")]),
        ("join-repeated-and-reassigned", "on join:\n    set {_target} to player\n    set {_target} to player\n    set {_target} to \"fresh\"\n    send {_target}\n",vec![(1,": player?"),(2,": player?"),(3,": string")]),
        ("optional-command", "command /storageoptional [<player>]:\n    trigger:\n        set {_target} to player\n        if arg-1 is set:\n            set {_target} to arg-1\n",vec![(2,": player?"),(4,": player?")]),
        ("required-command", "command /storagerequired <player>:\n    trigger:\n        set {_target} to arg-1\n",vec![(2,": player?")]),
        ("players-only", "command /storageplayers:\n    executable by: players\n    trigger:\n        set {_target} to player\n",vec![(3,": player?")]),
        ("literal-and-property-controls", "on join:\n    set {_n} to 123\n    set {_text} to \"fresh\"\n    set {_name} to player's name\n",vec![(1,": long"),(2,": string"),(3,": textcomponent?")]),
        ("function-return-bounds", "function publicStoredNumber() :: number:\n    return 3\nfunction publicStoredText() :: text:\n    return \"fresh\"\non load:\n    set {_n} to publicStoredNumber()\n    set {_text} to publicStoredText()\n",vec![(5,": number?"),(6,": string?")]),
        ("branch-write-sites", "command /storagebranches [<player>]:\n    trigger:\n        if arg-1 is set:\n            set {_target} to arg-1\n        else:\n            set {_target} to \"fresh\"\n        send {_target}\n",vec![(3,": player?"),(5,": string")]),
        ("independent-local-scopes", "command /storagefirst:\n    trigger:\n        set {_same} to player\ncommand /storagesecond:\n    trigger:\n        set {_same} to \"fresh\"\n",vec![(2,": player?"),(5,": string")]),
    ];
    for (name,source,expected) in cases {
        let (_,document)=parse(source);
        let hints=document.write_type_hints();
        let actual=hints.iter().map(|h|(h.position.line,match &h.label { InlayHintLabel::String(s)=>s.as_str(),_=>panic!("unexpected label") })).collect::<Vec<_>>();
        assert_eq!(actual,expected,"{name}");
        assert!(hints.iter().all(|h|h.text_edits.is_none()),"virtual hints only");
        for (row,label) in expected.iter().filter(|(_,l)|l.contains("player")) {
            let line=source.lines().nth(*row as usize).unwrap();
            let column=line.find("{_").unwrap()+3;
            let hover=document.hover(Position::new(*row,column as u32)).unwrap().1;
            assert!(hover.contains("provider-certified post-storage type"),"{name}: {label}: {hover}");
        }
    }
}

#[test]
fn standard_storage_model_keeps_source_positions_current_and_does_not_reuse_old_assignment_facts() {
    let original="on join:\n    set {_target} to player\n";
    let (result,first)=parse(original);
    assert!(matches!(&first.write_type_hints()[0].label,InlayHintLabel::String(s) if s==": player?"));
    let edited="on join:\n    set {_target} to \"\u{65b0}\u{3057}\u{3044}\u{5024}\u{1f600}\"\n    send {_target}\n";
    assert!(semantic::Document::from_result(&result,Arc::from(edited),"file:///storage-public/test.sk".parse().unwrap(),catalog()).is_err());
    let (_,second)=parse(edited);
    let hints=second.write_type_hints();
    assert_eq!(hints.len(),1);
    assert!(matches!(&hints[0].label,InlayHintLabel::String(s) if s==": string"));
    let lines=LineIndex::new(edited);
    assert_eq!(hints[0].position,lines.position(edited,edited.find("{_target}").unwrap()+"{_target}".len()).unwrap());
}


#[test]
fn storage_model_cold_standalone_effect_remains_recoverable_without_preparation() {
    use parser_wasm::host::InvocationContext;
    use skript_parser::{EffectParseRequest,EffectParserConfig,ExpressionParseContext,MappedSource,RawTreeOptions,parse_raw_tree};
    let mut host=ParserHost::new(crate::core_library_component(),HostConfig {
        syntax_catalog:Some(catalog()),
        runtime_profile:RuntimeProfile {skript_version:Some("2.15.4".into()),..RuntimeProfile::default()},
        ..HostConfig::default()
    }).unwrap();
    let transaction=host.begin_parse("file:///storage-public","file:///storage-public/cold.sk",1).unwrap();
    let source=MappedSource::identity("set {_x} to abs(3)");
    let tree=parse_raw_tree(&source,RawTreeOptions::for_skript_version(2,15));
    let result=host.parse_effect_in_parse(&transaction,InvocationContext {
        invocation_id:1,subscription_id:String::new(),document_id:"file:///storage-public/cold.sk".into(),
        document_revision:1,expansion:None,syntax_context:0
    },EffectParseRequest {source:&source,node:tree.get(tree.roots[0]).unwrap(),context:ExpressionParseContext::default()},EffectParserConfig::default()).unwrap();
    assert!(result.failures.is_empty(),"{:?}",result.failures);
    assert!(result.matches.selected.is_some(),"{:?}",result.matches.unknown);
    let change=result.matches.selected.as_ref().unwrap().metadata.get("nlaocs.core-library/skript.semantic.variable-change").expect("host-stamped Core contract");
    let change:serde_json::Value=serde_json::from_str(change).unwrap();
    assert!(change["storageResult"].is_null(),"cold backend must not add a storage guarantee: {change}");
    transaction.cancel().unwrap();
}

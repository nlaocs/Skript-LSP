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
    assert!(result.syntax.diagnostics.is_empty(),"source: {source}\n{:?}",result.syntax.diagnostics);
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

#[test]
fn option_source_projection_preserves_unaffected_copies_and_exact_write_targets() {
    for value in ["1", "{@value}"] {
        let source=format!("options:\n    value: 1\non load:\n    set {{_a}} to 7\n    set {{_b}} to {{_a}}\n    set {{_option}} to {value}\n    set {{_c}} to 8\n    set {{_d}} to {{_c}}\n");
        let (_,document)=parse(&source);
        let hints=document.write_type_hints();
        assert_eq!(hints.len(),if value=="1" {5}else{4},"{value}: {hints:?}");
        if value!="1" { assert!(hints.iter().all(|hint|hint.position.line!=5),"generated target has no Exact source/context certificate"); }
        assert!(hints.iter().all(|hint|matches!(&hint.label,InlayHintLabel::String(label) if label==": long")),"{value}: {hints:?}");
        let hover=document.hover(Position::new(7,18)).unwrap().1;
        assert!(hover.contains("Current type: long") || hover.contains("**Current type:** `long`"),"{hover}");
    }
}

#[test]
fn generated_option_source_targets_do_not_emit_hints_or_preserve_unproved_reaching_facts() {
    let source="options:\n    target: {_hidden}\non load:\n    set {_old} to 7\n    set {@target} to \"fresh\"\n    set {_copy} to {_old}\n";
    let (_,document)=parse(source);
    let hints=document.write_type_hints();
    assert_eq!(hints.len(),2,"{hints:?}");
    assert!(hints.iter().all(|hint|hint.position.line!=4),"generated binding must not become an original source target");
    assert!(matches!(&hints[1].label,InlayHintLabel::String(label) if label==": ?"),"uneditable generated writes still invalidate possibly affected facts: {hints:?}");
}

#[test]
fn option_source_projection_rejects_stale_witnesses_and_preserves_utf16_target_geometry() {
    let source="options:\n    value: 123456\non load:\n    set {_x\u{1f98a}} to 7\n    set {_expanded} to {@value}\n    set {_copy} to 8\n";
    let (mut result,document)=parse(source);
    let hints=document.write_type_hints();
    assert_eq!(hints.len(),2,"{hints:?}");
    assert!(hints.iter().all(|h|matches!(&h.label,InlayHintLabel::String(s) if s==": long")));
    let offset=source.find("{_x\u{1f98a}}").unwrap()+"{_x\u{1f98a}}".len();
    assert_eq!(hints[0].position,LineIndex::new(source).position(source,offset).unwrap());
    let changed=source.replace("123456","\"fresh\"");
    assert!(semantic::Document::from_result(&result,Arc::from(changed),"file:///storage-public/test.sk".parse().unwrap(),catalog()).is_err());
    // A public AST mutation cannot borrow the private final-forest witness.
    result.ast.nodes.iter_mut().find(|n|n.kind==parser_wasm::host::SyntaxKind::Effect).unwrap().text.push(' ');
    let altered=semantic::Document::from_result(&result,Arc::from(source),"file:///storage-public/test.sk".parse().unwrap(),catalog()).unwrap();
    assert!(altered.write_type_hints().is_empty());
}

#[test]
fn source_map_only_mutation_cannot_relocate_private_semantic_proof() {
    use skript_parser::{ExpansionGraph,MappedSource,SourceMapSegment,SourceOrigin,TextRange};
    let source="on load:\n    set {_x} to 1\n    set {_x} to 2\n";
    let (mut result,document)=parse(source);
    assert_eq!(document.write_type_hints().len(),2);
    let first=source.find("{_x}").unwrap();
    let second=source.rfind("{_x}").unwrap();
    let len="{_x}".len();
    let spans=[(0,first,0,first),(first,first+len,second,second+len),(first+len,second,first+len,second),(second,second+len,first,first+len),(second+len,source.len(),second+len,source.len())];
    result.source=MappedSource::new(source,source,ExpansionGraph::default(),spans.into_iter().map(|(a,b,c,d)|SourceMapSegment::new(TextRange::new(a,b),SourceOrigin::exact(TextRange::new(c,d),None))).collect()).unwrap();
    assert!(result.ast.nodes.iter().all(|node|!result.semantic_witnesses().native_valid(node)));
    let altered=semantic::Document::from_result(&result,Arc::from(source),"file:///storage-public/test.sk".parse().unwrap(),catalog()).unwrap();
    assert!(altered.write_type_hints().is_empty());
}

#[test]
fn legacy_properties_certify_storage_in_event_command_and_parameter_scopes() {
    for source in [
        "on join:\n    set {_id} to uuid of player\n    send {_id}\n",
        "command /publicuuid <offlineplayer>:\n    trigger:\n        set {_id} to uuid of arg-1\n        send {_id}\n",
        "command /publicoptionaluuid [<offlineplayer>]:\n    trigger:\n        set {_id} to uuid of arg-1\n",
        "function publicUUIDScope(person: offlineplayer):\n    set {_id} to uuid of {_person}\non join:\n    set {_id} to \"fresh\"\n",
    ] {
        let (result,document)=parse(source);
        let hints=document.write_type_hints();
        assert_eq!(hints.len(),if source.starts_with("function ") {2}else{1},"{source}: {hints:?}");
        let first=hints.first().expect(source);
        assert!(matches!(&first.label,InlayHintLabel::String(label) if label==": uuid?"),"{source}: {hints:?}");
        let at=source.find("{_id}").unwrap()+2;
        let hover=document.hover(LineIndex::new(source).position(source,at).unwrap()).unwrap().1;
        assert!(hover.contains("provider-certified post-storage type"),"{hover}");
        if hints.len()>1 { assert!(matches!(&hints[1].label,InlayHintLabel::String(label) if label==": string"),"separate scope: {hints:?}"); }
        let changed=source.replacen("uuid of player","123",1);
        if changed!=source {
            assert!(semantic::Document::from_result(&result,Arc::from(changed.as_str()),"file:///storage-public/test.sk".parse().unwrap(),catalog()).is_err());
            let (_,edited)=parse(&changed);
            assert!(matches!(&edited.write_type_hints()[0].label,InlayHintLabel::String(label) if label==": long"));
        }
    }
}

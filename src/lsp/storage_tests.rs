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
fn declared_event_contracts_reach_set_and_lookup_for_two_unpinned_events() {
    for event in ["craft","preparing craft"] {
        let source=format!("on {event}:\n    set {{_value}} to event-string\n    set {{_copy}} to {{_value}}\n");
        let (result,document)=parse(&source);
        let hints=document.write_type_hints();
        assert!(matches!(&hints[0].label,InlayHintLabel::String(s) if s==": string?"),"{source}: {hints:?}");
        let change=result.ast.nodes.iter().flat_map(|n|&n.metadata)
            .find(|m|m.key=="skript.semantic.variable-change").expect("Core SET contract");
        let change:serde_json::Value=serde_json::from_str(&change.value).unwrap();
        assert!(change["storageResult"].is_null(),"this case must use the generic declaration path");
        assert_eq!(change["declaredStorage"]["types"],serde_json::json!(["java.lang.String"]));
        let at=source.rfind("{_value}").unwrap()+2;
        let hover=document.hover(LineIndex::new(&source).position(&source,at).unwrap()).unwrap().1;
        assert!(hover.contains("Current type: string?") || hover.contains("**Current type:** `string?`"),"{hover}");
        assert!(hover.contains("declared bounds after standard storage transfer"),"{hover}");
    }
}

#[test]
fn an_opaque_caller_does_not_turn_a_stored_declaration_into_a_current_read_proof() {
    let source="on craft:\n    set {_value} to event-string\n    broadcast {_value}\n";
    let (_,document)=parse(source);
    let at=source.rfind("{_value}").unwrap()+2;
    let hover=document.hover(LineIndex::new(source).position(source,at).unwrap()).unwrap().1;
    assert!(hover.contains("Current type: ?") || hover.contains("**Current type:** `?`"),"{hover}");
    assert!(hover.contains("history only"),"{hover}");
}

#[test]
fn fixed_local_nullable_copies_ignore_unrelated_defaults_and_invalidate_after_edits() {
    let source="variables:\n    {value} = \"fallback\"\non craft:\n    set {_value} to event-string\n    set {_copy} to {_value}\n";
    let (result,document)=parse(source);
    let hints=document.write_type_hints();
    let actual=hints.iter().map(|hint| (hint.position,match &hint.label {
        InlayHintLabel::String(label)=>label.as_str(),_=>panic!("unexpected label"),
    })).collect::<Vec<_>>();
    assert_eq!(actual,vec![(Position::new(3,16),": string?"),(Position::new(4,15),": string?")]);

    // The audited fixed-name local lookup does not consult {value}'s default.
    // Its explicit empty result preserves unset, not unconditional presence.
    let read_start=source.rfind("{_value}").unwrap() as u64;
    let read=result.ast.nodes.iter()
        .filter(|node| node.span.virtual_range.start==read_start)
        .flat_map(|node| &node.metadata)
        .find(|entry| entry.key=="skript.semantic.variable-read")
        .expect("copy RHS lookup contract");
    assert_eq!(read.owner_component_id.as_deref(),Some("nlaocs.core-library"));
    let read:serde_json::Value=serde_json::from_str(&read.value).unwrap();
    assert_eq!(read["version"],2);
    assert_eq!(read["missingValue"],"empty");

    let edited=source.replace("event-string","\"new\u{1f600}\"");
    assert!(semantic::Document::from_result(&result,Arc::from(edited.as_str()),"file:///storage-public/test.sk".parse().unwrap(),catalog()).is_err());
    let (_,edited_document)=parse(&edited);
    let edited_hints=edited_document.write_type_hints();
    assert_eq!(edited_hints.len(),2);
    assert!(edited_hints.iter().all(|hint|matches!(&hint.label,InlayHintLabel::String(label) if label==": string")));
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
        let (result,document)=parse(source);
        for change in result.ast.nodes.iter().flat_map(|n|&n.metadata).filter(|m|m.key=="skript.semantic.variable-change") {
            let change:serde_json::Value=serde_json::from_str(&change.value).unwrap();
            if !change["storageResult"].is_null() {
                assert!(change["declaredStorage"].is_null(),"native storage proof takes precedence without duplicate inference: {name}");
            }
        }
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
        assert_exact_long_write_targets(&source, &document, &["{_a}", "{_b}", "{_option}", "{_c}", "{_d}"]);
        if value == "{@value}" {
            let marker = source.find("{@value}").unwrap();
            assert!(document.hover(LineIndex::new(&source).position(&source, marker + 2).unwrap()).is_none());
        }
        let hover=document.hover(Position::new(7,18)).unwrap().1;
        assert!(hover.contains("Current type: long") || hover.contains("**Current type:** `long`"),"{hover}");
    }
}

fn assert_exact_long_write_targets(source: &str, document: &semantic::Document, targets: &[&str]) {
    let hints = document.write_type_hints();
    assert_eq!(hints.len(), targets.len(), "{hints:?}");
    let index = LineIndex::new(source);
    for (target, hint) in targets.iter().zip(&hints) {
        let start = source.find(*target).unwrap();
        assert_eq!(hint.position, index.position(source, start + target.len()).unwrap());
        assert!(matches!(&hint.label, InlayHintLabel::String(label) if label == ": long"));
        assert!(hint.text_edits.is_none());
        let hover = document.hover(index.position(source, start + 2).unwrap()).unwrap().1;
        assert!(hover.contains("Current type: long") || hover.contains("**Current type:** `long`"), "{hover}");
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
    assert_exact_long_write_targets(source, &document, &["{_x\u{1f98a}}", "{_expanded}", "{_copy}"]);
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

#[test]
fn scalar_add_dispatch_tracks_present_numeric_results_and_repeated_updates() {
    let source="on load:\n    set {_n} to 1\n    add 1 to {_n}\n    add 0.5 to {_n}\n    set {_copy} to {_n}\n    set {_n} to \"new\"\n";
    let (_,document)=parse(source);
    let hints=document.write_type_hints();
    let labels=hints.iter().map(|h|(h.position.line,match &h.label {InlayHintLabel::String(s)=>s.clone(),_=>panic!("unexpected label")})).collect::<Vec<_>>();
    assert_eq!(labels,[(1,": long".into()),(2,": double | long".into()),(3,": double | long".into()),(4,": double | long".into()),(5,": string".into())]);
    let copy=source.find("set {_copy} to {_n}").unwrap()+"set {_copy} to ".len();
    let definitions=document.definitions_at(LineIndex::new(source).position(source,copy+2).unwrap(),&[]);
    assert_eq!(definitions.len(),1);assert_eq!(definitions[0].range.start.line,3,"reaching definition is the latest proved update");
    let (_,same_operation)=parse("on load:\n    set {_n} to 1\n    give 2 to {_n}\n");
    assert!(matches!(&same_operation.write_type_hints()[1].label,InlayHintLabel::String(s) if s==": double | long"),"dispatch follows current values, not the give spelling");
    let overflow="on load:\n    set {_n} to 9223372036854775807\n    add 1 to {_n}\n";
    let (_,document)=parse(overflow);
    assert!(matches!(&document.write_type_hints()[1].label,InlayHintLabel::String(s) if s==": double | long"),"overflow must allow Double, never retain only Long");
}

#[test]
fn unresolved_value_changes_are_not_binding_definitions_and_do_not_preserve_old_types() {
    let cases=[
        ("give", "on join:\n    set {_target} to player\n    give stone to {_target}\n    set {_copy} to {_target}\n",2usize,"{_target}"),
        ("unknown-delta", "on load:\n    set {_target} to 1\n    add {public.unknown} to {_target}\n    set {_copy} to {_target}\n",2,"{_target}"),
        ("reassigned-nonnumeric", "on load:\n    set {_target} to 1\n    set {_target} to \"fresh\"\n    add 1 to {_target}\n    set {_copy} to {_target}\n",3,"{_target}"),
        ("unset", "on load:\n    delete {_target}\n    add 1 to {_target}\n    set {_copy} to {_target}\n",2,"{_target}"),
        ("nullable-argument", "function publicMutation(value: number):\n    add 1 to {_value}\n    set {_copy} to {_value}\n",1,"{_value}"),
    ];
    for (name,source,row,target) in cases {
        let (_,document)=parse(source);let hints=document.write_type_hints();
        assert!(hints.iter().all(|h|h.position.line!=row as u32),"{name}: no definite binding write was proved: {hints:?}");
        let last=hints.last().unwrap();assert!(matches!(&last.label,InlayHintLabel::String(s) if s==": ?"),"{name}: unknown effects must not preserve a historical type: {hints:?}");
        let offset=source.rfind(target).unwrap()+2;
        let definitions=document.definitions_at(LineIndex::new(source).position(source,offset).unwrap(),&[]);
        assert!(definitions.iter().all(|location|location.range.start.line!=row as u32),"{name}: unknown dispatch cannot become a definition: {definitions:?}");
        if name=="nullable-argument" {
            assert!(definitions.iter().all(|location|location.range.start.line==0),"only the explicit parameter declaration may remain: {definitions:?}");
        } else {assert!(definitions.is_empty(),"{name}: historical writes cannot revive: {definitions:?}");}
    }
}

#[test]
fn explicit_global_and_local_list_binding_writes_keep_existing_navigation_roles() {
    let source="on load:\n    set {public.mutation} to 1\n    delete {public.mutation}\n    set {_copy} to {public.mutation}\n    set {_items::*} to 1\n    add 2 to {_items::*}\n";
    let (_,document)=parse(source);let hints=document.write_type_hints();
    for row in [1,2,4,5] {assert!(hints.iter().any(|h|h.position.line==row),"definite binding role preserved at {row}: {hints:?}");}
    let at=source.rfind("{public.mutation}").unwrap()+2;
    let locations=document.definitions_at(LineIndex::new(source).position(source,at).unwrap(),&[]);
    assert!(locations.iter().any(|l|l.range.start.line==1),"global SET stays a definition");
    assert!(locations.iter().any(|l|l.range.start.line==2),"global DELETE stays a binding operation");
}

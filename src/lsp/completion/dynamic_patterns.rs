//! Current native document patterns, separate from the shared startup Catalog.
//! These are editing candidates, not proof of handler execution or type safety.
use super::*;

const MAX_PATTERNS: usize = 256;
const MAX_BYTES: usize = 64 * 1024;

pub(super) fn literal_prefix<'a>(prefix: &'a str, folded: &str) -> Option<(usize, &'a str)> {
    let minimum=prefix.len().saturating_sub(folded.len().saturating_mul(4));
    prefix.char_indices().filter(|(start, _)| *start>=minimum && (*start == 0
        || prefix[..*start].chars().next_back().is_some_and(|c| !word(c))))
        .find_map(|(start, _)| {
            let query = &prefix[start..];
            (!query.is_empty() && folded.starts_with(&query.to_lowercase())).then_some((start, query))
        })
}

#[derive(Default)]
pub(super) struct View {
    pub(super) templates: Vec<Template>,
    pub(super) patterns: Vec<Pattern>,
    pub(super) limited: bool,
    identities: HashSet<String>,
}

impl View {
    pub(super) fn owns(&self, identity: &str) -> bool { self.identities.contains(identity) }
    pub(super) fn from_snapshot(snapshot: &syntaxes::DynamicSyntaxSnapshot) -> Self {
        let mut view = Self::default();
        let mut bytes = 0usize;
        for candidate in &snapshot.candidates {
            let syntaxes::SyntaxCandidateSource::Dynamic(id) = &candidate.source else { continue; };
            let Some(definition) = snapshot.definitions.get(id) else { view.limited = true; continue; };
            // The document projection supports literal events, expressions and actions. The
            // native registry can contain other kinds/shapes without pretending
            // that this editor view has complete coverage of them.
            if !matches!(definition.kind,SyntaxKind::Event|SyntaxKind::Expression|SyntaxKind::Effect|SyntaxKind::Condition) { view.limited = true; continue; }
            for (pattern_index, pattern) in definition.patterns.iter().enumerate() {
                bytes = bytes.saturating_add(pattern.source.len());
                if view.patterns.len() >= MAX_PATTERNS || bytes > MAX_BYTES
                    || pattern.source.len() > MAX_TEMPLATE_BYTES {
                    view.limited = true; continue;
                }
                let mut text = String::new();
                for element in &pattern.parsed.elements {
                    if let PatternElement::Literal(value) = &element.value { text.push_str(value); }
                    else { text.clear(); break; }
                }
                if text.trim().is_empty() { view.limited = true; continue; }
                let Some(graph) = continuation::Graph::with_prefix("", &pattern.parsed.elements) else {
                    view.limited = true; continue;
                };
                let identity = format!("{}:{pattern_index}", id.qualified());
                view.identities.insert(identity.clone());
                let detail = if definition.kind==SyntaxKind::Expression {format!("Expression / {} / declared {} ({:?}); handler not executed",
                    id.component_id, definition.return_type.as_deref().unwrap_or("unknown"), definition.return_multiplicity)}
                    else {format!("{:?} / {}; opaque handler, execution/truth/purity unverified; context/arguments unverified",definition.kind,id.component_id)};
                view.patterns.push(Pattern { documentation: None, kind: definition.kind,
                    label: pattern.source.clone(), detail: detail.clone(), identity: identity.clone(), graph });
                view.templates.push(Template { finite_capture: false, documentation: None,
                    kind: definition.kind, label: pattern.source.clone(), detail, identity,
                    folded: text.to_lowercase(), leading: text.clone(), parts: vec![Part::Text(if definition.kind==SyntaxKind::Event {format!("{text}:")}else{text})] });
            }
        }
        view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(catalog: &Catalog, pattern: &str) -> syntaxes::DynamicSyntaxSnapshot {
        snapshot_kind(catalog,pattern,SyntaxKind::Expression)
    }

    fn snapshot_kind(catalog:&Catalog,pattern:&str,kind:SyntaxKind)->syntaxes::DynamicSyntaxSnapshot {
        let id = syntaxes::DynamicSyntaxId::new("test.declarations", "greeting");
        let definition = syntaxes::DynamicSyntaxDefinition {
            id: id.clone(), kind,
            patterns: vec![syntaxes::DynamicPattern { source: pattern.into(),
                parsed: syntax_pattern_parser::syntax::parse(pattern, catalog.plural_rules()).unwrap() }],
            priority: 0, before: vec![], after: vec![], return_type: (kind==SyntaxKind::Expression).then(||"java.lang.String".into()),
            return_multiplicity: (kind==SyntaxKind::Expression).then_some(syntaxes::DynamicMultiplicity::Single),
            structure_node_type: None, structure_body_mode: None, entry_validator: None,
            handler: "test.expression".into(), metadata: BTreeMap::new(), component_load_order: 0,
            declaration_order: 0,
        };
        syntaxes::DynamicSyntaxSnapshot { document_id: "file:///current.sk".into(), document_revision: 1,
            registry_revision: 1, definitions: BTreeMap::from([(id.clone(), definition)]), overrides: BTreeMap::new(),
            candidates: vec![syntaxes::RankedSyntaxCandidate { source: syntaxes::SyntaxCandidateSource::Dynamic(id),
                kind, overrides: vec![] }] }
    }

    #[test]
    fn current_literal_patterns_complete_multiple_words_without_mutating_startup_catalog() {
        let catalog = crate::lsp::test_support::runtime_catalog();
        let index = Arc::new(CatalogIndex::with_documentation_format(&catalog,
            skript_parser::RawTreeOptions::for_skript_version(2,15), lsp_types::MarkupKind::PlainText));
        for (pattern,prefix) in [("review greeting","rev"),("review greeting","review gr"),("review result","review re")] {
            let source: Arc<str> = format!("# 😀\non load:\n    set {{_x}} to {prefix}\n").into();
            let mut host=crate::new_parser_host(parser_wasm::HostConfig { syntax_catalog:Some(Arc::clone(&catalog)), ..Default::default() }).unwrap();
            let result=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///dynamic-editing",
                "file:///dynamic-editing/current.sk",1,source.to_string()),Default::default()).unwrap();
            let mut document = Document::from_result(&result, Arc::clone(&source), Arc::clone(&index)).unwrap();
            document.dynamic_patterns = View::from_snapshot(&snapshot(&catalog,pattern));
            let offset = source.find(prefix).unwrap() + prefix.len();
            let list = document.syntax_at(document.lines.position(&source,offset).unwrap(),false).unwrap();
            let proposals=list.items.iter().filter(|i| i.label==pattern).collect::<Vec<_>>();
            assert_eq!(proposals.len(),1,"one native dynamic edit for {prefix}");
            let Some(CompletionTextEdit::Edit(edit))=&proposals[0].text_edit else {panic!("plain edit")};
            let mut applied=source.to_string();
            let start=document.lines.offset(&source,edit.range.start).unwrap();
            let end=document.lines.offset(&source,edit.range.end).unwrap();
            applied.replace_range(start..end,&edit.new_text);
            assert!(applied.contains(&format!("    set {{_x}} to {pattern}\n")),"{applied}");
            assert!(!index.templates.iter().any(|t| t.label == pattern));
            let fresh = Document::from_result(&result,Arc::clone(&source),Arc::clone(&index)).unwrap();
            let list = fresh.syntax_at(fresh.lines.position(&source,offset).unwrap(),false).unwrap();
            assert!(!list.items.iter().any(|i| i.label==pattern));
        }
    }

    #[test]
    fn current_action_patterns_follow_statement_and_condition_routes() {
        let catalog=crate::lsp::test_support::runtime_catalog();
        let mut host=crate::new_parser_host(parser_wasm::HostConfig {
            syntax_catalog:Some(catalog.clone()),
            runtime_profile:parser_wasm::RuntimeProfile {skript_version:Some("2.15.4".into()),..Default::default()},
            ..Default::default()
        }).unwrap();
        let mut index=CatalogIndex::with_documentation_format(&catalog,
            skript_parser::RawTreeOptions::for_skript_version(2,15),lsp_types::MarkupKind::PlainText);
        index.certify_types(&host);
        let index=Arc::new(index);
        for (revision,(kind,pattern,line,expected)) in [
            (SyntaxKind::Effect,"review notice","review no",Some("review notice")),
            (SyntaxKind::Effect,"review notice","rev",Some("review notice")),
            (SyntaxKind::Effect,"review notice","if review no:",None),
            (SyntaxKind::Effect,"review notice","set {_x} to review no",None),
            (SyntaxKind::Condition,"review ready","review re",Some("review ready")),
            (SyntaxKind::Condition,"review ready","if review re:",Some("if review ready:")),
            (SyntaxKind::Condition,"review ready","if rev:",Some("if review ready:")),
            (SyntaxKind::Condition,"review ready","if rev: # keep",Some("if review ready: # keep")),
            (SyntaxKind::Condition,"review ready","set {_x} to review re",None),
        ].into_iter().enumerate() {
            let source:Arc<str>=format!("# Unicode 日本語 🦀\non load:\n    {line}\n").into();
            let result=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///actions",
                "file:///actions/current.sk",revision as u64+1,source.to_string()),Default::default()).unwrap();
            let mut doc=Document::from_result(&result,source.clone(),index.clone()).unwrap();
            let view=View::from_snapshot(&snapshot_kind(&catalog,pattern,kind));
            assert_eq!(view.templates[0].kind,kind);
            assert!(view.templates[0].detail.contains("unverified"));
            doc.dynamic_patterns=view;
            let needle=if line.contains("no") {"review no"}else if line.contains("re:")||line.ends_with(" re") {"review re"}else{"rev"};
            let offset=source.find(needle).unwrap()+needle.len();
            let position=doc.lines.position(&source,offset).unwrap();
            for snippets in [false,true] {
                let list=doc.syntax_at(position,snippets).unwrap();
                let proposals=list.items.iter().filter(|i|i.label==pattern).collect::<Vec<_>>();
                if let Some(expected)=expected {
                    assert_eq!(proposals.len(),1,"{kind:?}: {line}");
                    let Some(CompletionTextEdit::Edit(edit))=&proposals[0].text_edit else {panic!("native edit")};
                    let mut applied=source.to_string();
                    let range=doc.lines.offset(&source,edit.range.start).unwrap()..doc.lines.offset(&source,edit.range.end).unwrap();
                    applied.replace_range(range,&edit.new_text);
                    assert!(applied.contains(&format!("    {expected}\n")),"{applied}");
                    assert_eq!(proposals[0].data.as_ref().unwrap()["syntaxKind"],format!("{kind:?}"));
                }else {assert!(proposals.is_empty(),"action leaked into wrong grammar: {line}");}
            }
            let fresh=Document::from_result(&result,source.clone(),index.clone()).unwrap();
            assert!(!fresh.syntax_at(position,false).unwrap().items.iter().any(|i|i.label==pattern),"withdrawn declarations must disappear");
        }
    }

    #[test]
    fn current_event_patterns_follow_direct_and_provider_declared_header_routes() {
        let catalog=crate::lsp::test_support::runtime_catalog();
        let mut host=crate::new_parser_host(parser_wasm::HostConfig {syntax_catalog:Some(catalog.clone()),
            runtime_profile:parser_wasm::RuntimeProfile {skript_version:Some("2.15.4".into()),..Default::default()},
            ..Default::default()}).unwrap();
        let mut index=CatalogIndex::with_documentation_format(&catalog,
            skript_parser::RawTreeOptions::for_skript_version(2,15),lsp_types::MarkupKind::PlainText);
        index.certify_types(&host);
        assert!(index.capture_routes.values().flatten().any(|(_,parser)|parser=="host.event"));
        let index=Arc::new(index);
        for (revision,(line,expected)) in [
            ("rev",Some("review fired:")),("review fi",Some("review fired:")),
            ("on rev",Some("on review fired:")),("on review fi:",Some("on review fired:")),
            ("on rev: # keep",Some("on review fired: # keep")),
            ("    review fi",None),("    set {_x} to rev",None),
        ].into_iter().enumerate() {
            let body=if expected.is_none() {format!("on load:\n{line}\n")}else{format!("{line}\n")};
            let source:Arc<str>=format!("# Unicode 日本語 😀\n{body}").into();
            let result=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///events",
                "file:///events/current.sk",revision as u64+1,source.to_string()),Default::default()).unwrap();
            let mut doc=Document::from_result(&result,source.clone(),index.clone()).unwrap();
            doc.dynamic_patterns=View::from_snapshot(&snapshot_kind(&catalog,"review fired",SyntaxKind::Event));
            let offset=source.find(line).unwrap()+line.find(':').unwrap_or(line.len());
            let position=doc.lines.position(&source,offset).unwrap();
            for snippets in [false,true] {
                let list=doc.syntax_at(position,snippets).unwrap();
                let proposals=list.items.iter().filter(|i|i.label=="review fired:").collect::<Vec<_>>();
                if let Some(expected)=expected {
                    assert_eq!(proposals.len(),1,"current event edit for {line}");
                    let Some(CompletionTextEdit::Edit(edit))=&proposals[0].text_edit else {panic!("plain event edit")};
                    let mut applied=source.to_string();
                    let range=doc.lines.offset(&source,edit.range.start).unwrap()..doc.lines.offset(&source,edit.range.end).unwrap();
                    applied.replace_range(range,&edit.new_text);
                    assert!(applied.contains(&format!("\n{expected}\n")),"{applied}");
                    assert_eq!(proposals[0].data.as_ref().unwrap()["syntaxKind"],"Event");
                }else {assert!(proposals.is_empty(),"Event must not enter body/RHS grammar: {line}");}
            }
            let fresh=Document::from_result(&result,source.clone(),index.clone()).unwrap();
            assert!(!fresh.syntax_at(position,false).unwrap().items.iter().any(|i|i.label=="review fired:"));
        }
    }

    #[test]
    #[ignore="requires the explicit custom-event profile and matching candidate Wasm"]
    fn current_event_patterns_with_actual_external_declarations() {
        let snapshot=std::env::var("SKRIPT_REFLECT_SNAPSHOT").expect("explicit snapshot");
        let candidate=std::env::var("SKRIPT_REFLECT_CANDIDATE_WASM").expect("explicit candidate");
        let catalog=Arc::new(ssg::load(std::path::Path::new(&snapshot)).unwrap().into_catalog());
        let mut host=crate::new_parser_host(parser_wasm::HostConfig {syntax_catalog:Some(catalog.clone()),..Default::default()}).unwrap();
        host.load_addon(&std::fs::read(candidate).unwrap()).unwrap();
        let mut index=CatalogIndex::with_documentation_format(&catalog,host.raw_tree_options(),lsp_types::MarkupKind::PlainText);
        index.certify_types(&host);
        assert!(!index.typing.as_ref().unwrap().finite_captures_current());
        assert!(index.capture_routes.values().flatten().any(|(_,parser)|parser=="host.event"));
        let index=Arc::new(index);
        for (number,line) in ["rev","review fi","on rev","on review fi:","on rev: # keep"].into_iter().enumerate() {
            let source:Arc<str>=format!("# Unicode 日本語 😀\nevent \"review id\":\n    pattern: review fired\n{line}\n    send \"event\" to console\n").into();
            let revision=number as u64*2+1;
            let result=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///mixed-events",
                "file:///mixed-events/current.sk",revision,source.to_string()),Default::default()).unwrap();
            assert!(result.component_failures.is_empty(),"{:?}",result.component_failures);
            let doc=Document::from_result(&result,source.clone(),index.clone()).unwrap();
            let start=source.find(&format!("\n{line}\n")).unwrap()+1;
            let offset=start+line.find(':').unwrap_or(line.len());
            let position=doc.lines.position(&source,offset).unwrap();
            let list=doc.syntax_at(position,false).unwrap();
            let proposals=list.items.iter().filter(|i|i.label=="review fired:").collect::<Vec<_>>();
            assert_eq!(proposals.len(),1,"actual mixed Event completion for {line}");
            let Some(CompletionTextEdit::Edit(edit))=&proposals[0].text_edit else {panic!("native edit")};
            let mut applied=source.to_string();
            applied.replace_range(doc.lines.offset(&source,edit.range.start).unwrap()..doc.lines.offset(&source,edit.range.end).unwrap(),&edit.new_text);
            let reparsed=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///mixed-events",
                "file:///mixed-events/current.sk",revision+1,applied),Default::default()).unwrap();
            assert!(reparsed.component_failures.is_empty());
            assert!(reparsed.syntax.diagnostics.is_empty(),"{:?}",reparsed.syntax.diagnostics);
            assert!(reparsed.ast.nodes.iter().any(|n|n.kind==parser_wasm::host::SyntaxKind::Event
                &&n.syntax_id.starts_with("dynamic:nlaocs.skript-reflect/")),"inserted event has a current native declaration");
        }
    }

    #[test]
    #[ignore="requires the explicit external declarations profile and matching candidate Wasm"]
    fn current_action_patterns_with_actual_external_declarations() {
        let snapshot=std::env::var("SKRIPT_REFLECT_SNAPSHOT").expect("explicit snapshot");
        let candidate=std::env::var("SKRIPT_REFLECT_CANDIDATE_WASM").expect("explicit candidate");
        let catalog=Arc::new(ssg::load(std::path::Path::new(&snapshot)).unwrap().into_catalog());
        let mut host=crate::new_parser_host(parser_wasm::HostConfig {
            syntax_catalog:Some(catalog.clone()),..Default::default()
        }).unwrap();
        host.load_addon(&std::fs::read(candidate).unwrap()).unwrap();
        assert!(!host.completion_catalog_patterns_are_closed());
        assert!(!host.completion_candidate_bindings_are_closed());
        let mut index=CatalogIndex::with_documentation_format(&catalog,host.raw_tree_options(),lsp_types::MarkupKind::PlainText);
        index.certify_types(&host);
        assert!(!index.typing.as_ref().unwrap().finite_captures_current());
        assert!(index.typing.as_ref().unwrap().advisory_routes_current());
        assert!(index.capture_routes.values().flatten().any(|(_,parser)|parser=="host.condition"));
        let index=Arc::new(index);
        let definitions="local effect review notice:\n    trigger:\n        continue\nlocal condition review ready:\n    check:\n        continue\n";
        let mut previous=None;
        for (revision,(line,pattern,expected)) in [
            ("review no","review notice",Some("review notice")),
            ("if review re:","review ready",Some("if review ready:")),
            ("if rev:","review ready",Some("if review ready:")),
            ("if rev: # keep","review ready",Some("if review ready: # keep")),
            ("if review no:","review notice",None),
            ("set {_x} to review re","review ready",None),
        ].into_iter().enumerate() {
            let source:Arc<str>=format!("# 日本語 🦀\n{definitions}on load:\n    {line}\n").into();
            let result=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///external-declarations",
                "file:///external-declarations/current.sk",revision as u64+1,source.to_string()),Default::default()).unwrap();
            assert!(result.component_failures.is_empty(),"{:?}",result.component_failures);
            assert_eq!(result.dynamic_syntax.definitions.len(),2);
            let doc=Document::from_result(&result,source.clone(),index.clone()).unwrap();
            let needle=if line.starts_with("if rev:") {"rev"}else if line.contains("no") {"review no"}else{"review re"};
            let offset=source.rfind(needle).unwrap()+needle.len();
            let position=doc.lines.position(&source,offset).unwrap();
            let list=doc.syntax_at(position,false).unwrap();
            let proposals=list.items.iter().filter(|item|item.label==pattern).collect::<Vec<_>>();
            if let Some(expected)=expected {
                assert_eq!(proposals.len(),1,"mixed completion for {line}");
                assert!(proposals[0].detail.as_ref().unwrap().contains("arguments unverified"));
                let Some(CompletionTextEdit::Edit(edit))=&proposals[0].text_edit else {panic!("plain edit")};
                let mut applied=source.to_string();
                applied.replace_range(doc.lines.offset(&source,edit.range.start).unwrap()..doc.lines.offset(&source,edit.range.end).unwrap(),&edit.new_text);
                assert!(applied.contains(&format!("    {expected}\n")),"{applied}");
                if pattern=="review ready" {previous=Some((doc,position));}
            }else {assert!(proposals.is_empty(),"wrong grammar: {line}");}
        }
        assert!(host.load_addon(&[]).is_err());
        let (doc,position)=previous.unwrap();
        assert!(!doc.index.typing.as_ref().unwrap().advisory_routes_current());
        assert!(!doc.syntax_at(position,false).unwrap().items.iter().any(|item|item.label=="review ready"),
            "expired providers withdraw advisory nested routes");
    }
    #[test]
    #[ignore="requires the explicit handler-entry profile and matching current guest Component"]
    fn current_handler_entry_completion_inserts_current_expressions_but_not_unknown_parse_gates() {
        let snapshot=std::env::var("SKRIPT_REFLECT_SNAPSHOT").expect("explicit snapshot");
        let candidate=std::env::var("SKRIPT_REFLECT_CANDIDATE_WASM").expect("explicit candidate");
        let catalog=Arc::new(ssg::load(std::path::Path::new(&snapshot)).unwrap().into_catalog());
        let mut host=crate::new_parser_host(parser_wasm::HostConfig {syntax_catalog:Some(catalog.clone()),..Default::default()}).unwrap();
        host.load_addon(&std::fs::read(candidate).unwrap()).unwrap();
        let mut index=CatalogIndex::with_documentation_format(&catalog,host.raw_tree_options(),lsp_types::MarkupKind::PlainText);
        index.certify_types(&host);let index=Arc::new(index);
        for (number,(prefix,expected)) in [("review la",Some("review label")),("review ga",None)].into_iter().enumerate() {
            let source:Arc<str>=format!("# 日本語 🦀\nlocal expression review label:\n    return type: string\n    get:\n        return \"ok\"\n    set string:\n        set {{_inside}} to {prefix}\nlocal expression review gated:\n    return type: boolean\n    parse:\n        continue\n    get:\n        return true\non load:\n    set {{_control}} to 123\n").into();
            let revision=number as u64*2+1;
            let result=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///handler-editor","file:///handler-editor/current.sk",revision,source.to_string()),Default::default()).unwrap();
            assert!(result.component_failures.is_empty(),"{:?}",result.component_failures);
            let doc=Document::from_result(&result,source.clone(),index.clone()).unwrap();
            let needle=format!("set {{_inside}} to {prefix}");
            let position=doc.lines.position(&source,source.find(&needle).unwrap()+needle.len()).unwrap();
            let list=doc.syntax_at(position,false).unwrap();
            assert!(!list.items.iter().any(|item|item.label=="review gated"),"unknown parse gate cannot supply a positive use candidate");
            if let Some(expected)=expected {
                let matches=list.items.iter().filter(|item|item.label==expected).collect::<Vec<_>>();
                assert_eq!(matches.len(),1,"adopted Unknown changer body is an executable expression slot");
                let Some(CompletionTextEdit::Edit(edit))=&matches[0].text_edit else {panic!("plain native insertion")};
                let mut applied=source.to_string();
                applied.replace_range(doc.lines.offset(&source,edit.range.start).unwrap()..doc.lines.offset(&source,edit.range.end).unwrap(),&edit.new_text);
                assert!(applied.contains("set {_inside} to review label"));
                let accepted=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///handler-editor","file:///handler-editor/current.sk",revision+1,applied),Default::default()).unwrap();
                assert!(accepted.component_failures.is_empty());assert!(accepted.syntax.diagnostics.is_empty(),"{:?}",accepted.syntax.diagnostics);
                assert!(accepted.ast.nodes.iter().any(|node|node.text=="review label" && node.kind==parser_wasm::host::SyntaxKind::Expression));
            }
        }
    }


    #[test]
    #[ignore="requires the explicit handler-entry profile and matching current guest Component"]
    fn current_handler_entry_invalid_delta_keeps_unrelated_core_inlay_type() {
        let snapshot=std::env::var("SKRIPT_REFLECT_SNAPSHOT").expect("explicit snapshot");
        let candidate=std::env::var("SKRIPT_REFLECT_CANDIDATE_WASM").expect("explicit candidate");
        let catalog=Arc::new(ssg::load(std::path::Path::new(&snapshot)).unwrap().into_catalog());
        let mut host=crate::new_parser_host(parser_wasm::HostConfig {syntax_catalog:Some(catalog.clone()),..Default::default()}).unwrap();
        host.load_addon(&std::fs::read(candidate).unwrap()).unwrap();
        let valid="local expression review counter:\n    return type: number\n    get:\n        return 1\n    set number:\n        set {_delta} to change value\n    add numbers:\n        set {_batch::*} to change values\non load:\n    set review counter to 2\n    add 1 and 2 to review counter\n    set {_control} to 123\n";
        for (number,source) in [valid.to_owned(),valid.replace("set {_batch::*} to change values","set {_batch} to change values")].into_iter().enumerate() {
            let source:Arc<str>=source.into();let uri="file:///handler-editor/inlay.sk";
            let result=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///handler-editor",uri,number as u64+1,source.to_string()),Default::default()).unwrap();
            assert!(result.component_failures.is_empty(),"{:?}",result.component_failures);
            assert_eq!(result.syntax.diagnostics.is_empty(),number==0);
            let semantic=crate::lsp::semantic::Document::from_result(&result,source.clone(),uri.parse().unwrap(),catalog.clone()).unwrap();
            let position=crate::lsp::positions::LineIndex::new(&source).position(&source,source.find("{_control}").unwrap()+"{_control}".len()).unwrap();
            let hints=semantic.write_type_hints();
            let node=result.ast.nodes.iter().find(|node|node.text=="set {_control} to 123").unwrap();
            let witnesses=result.semantic_witnesses();
            eprintln!("case {number}: native={}, issuer={:?}, composed={:?}, core={:x}",witnesses.native_valid(node),witnesses.unique_issuer(node),node.metadata.iter().filter(|m|m.owner_component_id.as_deref()==Some("nlaocs.core-library")).map(|m|(&m.key,witnesses.composed_metadata_valid(node,m))).collect::<Vec<_>>(),<sha2::Sha256 as sha2::Digest>::digest(crate::core_library_component()));
            let control=hints.iter().find(|hint|hint.position==position).unwrap_or_else(||panic!("case {number}: independent binding hint missing; current hints={hints:?}"));
            assert!(matches!(&control.label,lsp_types::InlayHintLabel::String(label) if label==": long"),"case {number}: {control:?}");
        }
    }


    #[test]
    #[ignore="requires explicit custom-event-values profile and matching current guest Component"]
    fn current_event_values_keep_core_hints_and_alias_completion_across_edits() {
        let snapshot=std::env::var("SKRIPT_REFLECT_SNAPSHOT").expect("explicit snapshot");
        let candidate=std::env::var("SKRIPT_REFLECT_CANDIDATE_WASM").expect("explicit candidate");
        let catalog=Arc::new(ssg::load(std::path::Path::new(&snapshot)).unwrap().into_catalog());
        let mut host=crate::new_parser_host(parser_wasm::HostConfig {syntax_catalog:Some(catalog.clone()),..Default::default()}).unwrap();
        host.load_addon(&std::fs::read(candidate).unwrap()).unwrap();
        let mut index=CatalogIndex::with_documentation_format(&catalog,host.raw_tree_options(),lsp_types::MarkupKind::PlainText);
        index.certify_types(&host);let index=Arc::new(index);
        let uri="file:///event-value-editor/value.sk";
        let source:Arc<str>="# 🧪 Unicode\nlocal event \"value id\":\n    patterns:\n        value arrived\n        alternate value arrived\n    event values: string\non alternate val:\n    set {_message} to event-string\n    set {_copy} to {_message}\n    set {_control} to 123\n".into();
        let first=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///event-value-editor",uri,1,source.to_string()),Default::default()).unwrap();
        let doc=Document::from_result(&first,source.clone(),index.clone()).unwrap();
        let at=doc.lines.position(&source,source.find("on alternate val:").unwrap()+"on alternate val".len()).unwrap();
        let completion=doc.syntax_at(at,false).unwrap();
        let matching=completion.items.iter().filter(|item|item.label=="alternate value arrived:").collect::<Vec<_>>();
        assert_eq!(matching.len(),1,"local alias is a current Event insertion candidate");
        let Some(CompletionTextEdit::Edit(edit))=&matching[0].text_edit else {panic!("native text edit")};
        let mut inserted=source.to_string();
        inserted.replace_range(doc.lines.offset(&source,edit.range.start).unwrap()..doc.lines.offset(&source,edit.range.end).unwrap(),&edit.new_text);
        let valid:Arc<str>=inserted.into();
        let mut baseline=None;
        for (revision,text,expected) in [(2,valid.to_string(),true),(3,valid.replace("event values: string","event values: number"),false),(4,valid.to_string(),true)] {
            let result=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///event-value-editor",uri,revision,text.clone()),Default::default()).unwrap();
            assert!(result.component_failures.is_empty(),"{:?}",result.component_failures);
            assert_eq!(result.syntax.diagnostics.is_empty(),expected,"{:?}",result.syntax.diagnostics);
            let semantic=crate::lsp::semantic::Document::from_result(&result,text.clone().into(),uri.parse().unwrap(),catalog.clone()).unwrap();
            let hints=semantic.write_type_hints();
            let lines=crate::lsp::positions::LineIndex::new(&text);
            let position=|name:&str|lines.position(&text,text.find(name).unwrap()+name.len()).unwrap();
            let label=|name:&str|hints.iter().find(|hint|hint.position==position(name)).map(|hint|match &hint.label {lsp_types::InlayHintLabel::String(s)=>s.clone(),_=>panic!("string hint")});
            assert_eq!(label("{_control}"),Some(": long".into()),"event selection must not author independent Core facts");
            if expected {
                assert_eq!(label("{_message}"),Some(": string?".into()),"declared event value does not prove presence");
                assert_eq!(label("{_copy}"),Some(": string?".into()));
                let at=lines.position(&text,text.find("event-string").unwrap()+2).unwrap();
                let variable_at=lines.position(&text,text.rfind("{_message}").unwrap()+2).unwrap();
                let (_,variable_hover)=semantic.hover(variable_at).expect("saved variable hover");
                assert!(variable_hover.contains("string"));
                let expression_hover=crate::lsp::hover::Document::from_result(result,text.clone().into(),catalog.clone(),lsp_types::MarkupKind::PlainText).unwrap();
                assert!(expression_hover.at(at).is_some(),"expression hover uses the actual syntax-hover path");
                let serialized=serde_json::to_value(&hints).unwrap();
                if let Some(old)=&baseline {assert_eq!(&serialized,old);}else{baseline=Some(serialized);}
            }else{assert_ne!(label("{_message}"),Some(": string?".into()),"old declaration cannot survive type-list edits");}
        }
        assert!(host.release_document("file:///event-value-editor",uri,4).unwrap());
        let reopened=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///event-value-editor",uri,1,valid.to_string()),Default::default()).unwrap();
        assert!(reopened.syntax.diagnostics.is_empty());
        let semantic=crate::lsp::semantic::Document::from_result(&reopened,valid,uri.parse().unwrap(),catalog).unwrap();
        assert_eq!(serde_json::to_value(semantic.write_type_hints()).unwrap(),baseline.unwrap());
    }

}

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
            // The document projection supports literal expressions and actions. The
            // native registry can contain other kinds/shapes without pretending
            // that this editor view has complete coverage of them.
            if !matches!(definition.kind,SyntaxKind::Expression|SyntaxKind::Effect|SyntaxKind::Condition) { view.limited = true; continue; }
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
                    folded: text.to_lowercase(), leading: text.clone(), parts: vec![Part::Text(text)] });
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
            ("if review no:","review notice",None),
            ("set {_x} to review re","review ready",None),
        ].into_iter().enumerate() {
            let source:Arc<str>=format!("# 日本語 🦀\n{definitions}on load:\n    {line}\n").into();
            let result=host.parse_document(parser_wasm::DocumentParseRequest::new("file:///external-declarations",
                "file:///external-declarations/current.sk",revision as u64+1,source.to_string()),Default::default()).unwrap();
            assert!(result.component_failures.is_empty(),"{:?}",result.component_failures);
            assert_eq!(result.dynamic_syntax.definitions.len(),2);
            let doc=Document::from_result(&result,source.clone(),index.clone()).unwrap();
            let needle=if line.contains("no") {"review no"}else{"review re"};
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
}

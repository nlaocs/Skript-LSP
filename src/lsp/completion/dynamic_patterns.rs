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
            // The initial document projection supports literal expressions. The
            // native registry can contain other kinds/shapes without pretending
            // that this editor view has complete coverage of them.
            if definition.kind != SyntaxKind::Expression { view.limited = true; continue; }
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
                let detail = format!("Expression / {} / declared {} ({:?}); handler not executed",
                    id.component_id, definition.return_type.as_deref().unwrap_or("unknown"), definition.return_multiplicity);
                view.patterns.push(Pattern { documentation: None, kind: SyntaxKind::Expression,
                    label: pattern.source.clone(), detail: detail.clone(), identity: identity.clone(), graph });
                view.templates.push(Template { finite_capture: false, documentation: None,
                    kind: SyntaxKind::Expression, label: pattern.source.clone(), detail, identity,
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
        let id = syntaxes::DynamicSyntaxId::new("test.declarations", "greeting");
        let definition = syntaxes::DynamicSyntaxDefinition {
            id: id.clone(), kind: SyntaxKind::Expression,
            patterns: vec![syntaxes::DynamicPattern { source: pattern.into(),
                parsed: syntax_pattern_parser::syntax::parse(pattern, catalog.plural_rules()).unwrap() }],
            priority: 0, before: vec![], after: vec![], return_type: Some("java.lang.String".into()),
            return_multiplicity: Some(syntaxes::DynamicMultiplicity::Single),
            structure_node_type: None, structure_body_mode: None, entry_validator: None,
            handler: "test.expression".into(), metadata: BTreeMap::new(), component_load_order: 0,
            declaration_order: 0,
        };
        syntaxes::DynamicSyntaxSnapshot { document_id: "file:///current.sk".into(), document_revision: 1,
            registry_revision: 1, definitions: BTreeMap::from([(id.clone(), definition)]), overrides: BTreeMap::new(),
            candidates: vec![syntaxes::RankedSyntaxCandidate { source: syntaxes::SyntaxCandidateSource::Dynamic(id),
                kind: SyntaxKind::Expression, overrides: vec![] }] }
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
}

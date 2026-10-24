use super::*;
use super::parameters::tests::{catalog, parse};

fn op(list: &CompletionList) -> Option<&CompletionItem> {
    list.items.iter().find(|item| item.label == "op"
        && item.kind == Some(CompletionItemKind::KEYWORD)
        && item.data.as_ref().is_some_and(|data| data["syntaxKind"] == "Condition"))
}

#[test]
fn nested_native_condition_words_work_in_prepared_and_final_views() {
    for source in [
        "on join:\n    player is o\n",
        "on join:\n    if player is o\n",
        "on join:\n    if player is o:\n        stop\n",
        "on join:\n    send \"🧪\"\n    if player is op: # 🧪\n        stop\n",
    ] {
        let (prepared, document, result, _) = parse(source);
        assert!(result.component_failures.is_empty());
        let offset = source.find("is o").unwrap() + 4;
        let lines = LineIndex::new(source);
        let position = lines.position(source, offset).unwrap();
        let final_list = document.at(position, false).unwrap();
        let early_list = match prepared.at(position, false).unwrap() {
            PreparedQuery::Ready(list) => list,
            PreparedQuery::NeedsAnalysis => panic!("confirmed parent should provide grammar guidance: {source}"),
        };
        for list in [&early_list, &final_list] {
            let item = op(list).unwrap_or_else(|| panic!("missing op at {position:?}: {source}"));
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else { panic!() };
            assert_eq!(edit.new_text, if source[offset..].starts_with('p') { "o" } else { "op" });
            assert_eq!(edit.range.start, lines.position(source, offset - 1).unwrap());
            assert_eq!(edit.range.end, position);
            assert!(item.detail.as_ref().unwrap().contains("arguments unverified"));
        }
    }
    let (_, _, result, _) = parse("on join:\n    if player is op:\n        send \"Public operator\" to player\n");
    assert!(result.syntax.diagnostics.is_empty(), "{:?}", result.syntax.diagnostics);
}

fn routed_document(source: &str) -> Document {
    let catalog = catalog();
    let mut index = CatalogIndex::with_documentation_format(
        &catalog, skript_parser::RawTreeOptions::for_skript_version(2, 15),
        lsp_types::MarkupKind::PlainText,
    );
    let host = parser_wasm::host::ParserHost::new(crate::core_library_component(), parser_wasm::host::HostConfig {
        syntax_catalog: Some(catalog.clone()),
        runtime_profile: parser_wasm::RuntimeProfile { skript_version: Some("2.15.4".into()), ..Default::default() },
        ..Default::default()
    }).unwrap();
    index.certify_types(&host);
    index.patterns.clear();
    index.templates.clear();
    index.functions.clear();
    index.capture_routes.clear();
    for (identity, label, kind) in [
        ("outer", "guard <.+>", SyntaxKind::Section),
        ("nested", "check <.+>", SyntaxKind::Condition),
        ("leaf", "player is op", SyntaxKind::Condition),
    ] {
        let parsed = syntax_pattern_parser::syntax::parse(label, catalog.plural_rules()).unwrap();
        index.patterns.push(Pattern { documentation: None, kind, label: label.into(),
            detail: "public routed fixture".into(), identity: identity.into(),
            graph: continuation::Graph::new(&parsed.elements).unwrap() });
    }
    for id in ["outer", "nested"] {
        index.capture_routes.insert(id.into(), vec![(0, "host.condition".into())]);
    }
    let mut document = Document::from_source(Arc::from(source), Arc::new(index)).unwrap();
    for line in document.context.values_mut().filter(|line| !line.root) {
        line.scope = Scope::Statements;
    }
    document
}

#[test]
fn declared_nested_domains_are_advisory_current_and_bounded() {
    let source = "root:\n    guard check player is o\n";
    let mut document = routed_document(source);
    let position = LineIndex::new(source).position(source, source.find("is o").unwrap() + 4).unwrap();
    let list = document.at(position, false).unwrap();
    let item = op(&list).expect("follow two declared captures, independent of keyword spelling");
    assert_eq!(item.data.as_ref().unwrap()["syntaxIdentity"], "leaf");
    Arc::get_mut(&mut document.index).unwrap().capture_routes.clear();
    assert!(op(&document.at(position, false).unwrap()).is_none());
    let mut document = routed_document(source);
    Arc::get_mut(&mut document.index).unwrap().typing = None;
    assert!(op(&document.at(position, false).unwrap()).is_none(), "missing provider certificate");
    for inside in ["guard check( player is o", "guard \"check player is o"] {
        let source = format!("root:\n    {inside}\n");
        let document = routed_document(&source);
        let line = &document.context[&1];
        let offset = source.find("is o").unwrap() + 3;
        let mut budget = continuation::Budget::new();
        let (words, _) = document.continuation_words(line, &source[line.code_start..offset], "o", &mut budget);
        assert!(!words.iter().any(|word| word.3 == "op"), "unbalanced capture: {inside}");
    }
    let source = format!("root:\n    guard {}player is o\n", "check ".repeat(24));
    let document = routed_document(&source);
    let line = &document.context[&1];
    let mut budget = continuation::Budget::new();
    let offset = source.find("is o").unwrap() + 3;
    let (_, limited) = document.continuation_words(line, &source[line.code_start..offset], "o", &mut budget);
    assert!(limited, "recursive route depth is bounded");
}

#[test]
fn converging_routes_preserve_a_later_shallower_path() {
    let source = format!("root:\n    guard {}player is o\n", "check ".repeat(12));
    let mut document = routed_document(&source);
    let index = Arc::get_mut(&mut document.index).unwrap();
    let parsed = syntax_pattern_parser::syntax::parse("guard check <.+>", index.typing.as_ref().unwrap().catalog.plural_rules()).unwrap();
    index.patterns.insert(0, Pattern { documentation: None, kind: SyntaxKind::Section,
        label: "guard check <.+>".into(), detail: "public shortcut fixture".into(),
        identity: "shortcut".into(), graph: continuation::Graph::new(&parsed.elements).unwrap() });
    index.capture_routes.insert("shortcut".into(), vec![(0, "host.condition".into())]);
    let offset = source.find("is o").unwrap() + 4;
    let position = LineIndex::new(&source).position(&source, offset).unwrap();
    assert!(op(&document.at(position, false).unwrap()).is_some(), "shallower route survives a deeper predecessor");
}

use super::*;
use crate::lsp::completion::parameters::tests::parse;

fn values(list: &CompletionList) -> Vec<&CompletionItem> {
    list.items.iter().filter(|i| i.data.as_ref().is_some_and(|d| d["typeValue"].is_object())).collect()
}

#[test]
fn finite_values_reach_real_argument_slots_early_and_final() {
    let source = "on load:\n    dye sto red\n    dye stone re\n    dye stone dark g\n    set game mode of player to sur\n    send \"a\" to p\n";
    let (early, view, _, _) = parse(source);
    for (needle, wanted, code) in [
        ("dye sto", "stone", "itemtype"),
        ("stone re", "red", "color"),
        ("dark g", "dark grey", "color"),
        ("to sur", "survival", "gamemode"),
    ] {
        let offset = source.find(needle).unwrap() + needle.len();
        let position = LineIndex::new(source).position(source, offset).unwrap();
        let PreparedQuery::Ready(prepared) = early.at(position, false).unwrap() else {
            panic!("prepared slot: {needle}")
        };
        for list in [prepared, view.at(position, false).unwrap()] {
            let proposals = values(&list);
            let item = proposals.iter().find(|i| i.label == wanted)
                .unwrap_or_else(|| panic!("{needle}: {:?}", proposals.iter().map(|i| &i.label).collect::<Vec<_>>()));
            assert_eq!(item.kind, Some(CompletionItemKind::VALUE));
            assert_eq!(item.data.as_ref().unwrap()["typeValue"]["codeName"], code);
            assert_eq!(item.data.as_ref().unwrap()["typeValue"]["contextVerified"], false);
            assert!(list.items.len() <= MAX_ITEMS);
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else { panic!() };
            assert_eq!(edit.range.end, position);
            assert_eq!(edit.new_text, wanted);
            if code == "color" {
                assert!(!proposals.iter().any(|i| i.label == "redstone"));
            }
        }
    }
    let position = LineIndex::new(source).position(source, source.rfind("to p").unwrap() + 4).unwrap();
    assert!(values(&view.at(position, false).unwrap()).is_empty(), "known incompatible audiences slot");
}

#[test]
fn value_edits_preserve_suffix_utf16_and_merge_duplicate_sources() {
    let source = "# 😀\non load:\n    dye stone dark grey\n    set game mode of player to survival\n";
    let (_, view, _, _) = parse(source);
    for (text, prefix) in [("dark grey", "da"), ("dark grey", "dark g"), ("survival", "sur")] {
        let start = source.find(text).unwrap();
        let offset = start + prefix.len();
        let position = LineIndex::new(source).position(source, offset).unwrap();
        let list = view.at(position, false).unwrap();
        let proposals = values(&list);
        let matching = proposals.iter().filter(|i| i.label == text).collect::<Vec<_>>();
        assert_eq!(matching.len(), 1, "duplicate supplier/enum/parser data");
        let Some(CompletionTextEdit::Edit(edit)) = &matching[0].text_edit else { panic!() };
        assert_eq!(edit.range.start, LineIndex::new(source).position(source, start).unwrap());
        assert_eq!(edit.range.end, position);
        assert_eq!(edit.new_text, prefix);
    }
}

#[test]
fn utf16_on_the_same_line_and_following_grammar_are_preserved() {
    let source = "on load:\n    consume 😀 dark grey\n";
    let (early, mut view, _, _) = parse(source);
    drop(early);
    let index = Arc::get_mut(&mut view.index).unwrap();
    let parsed = syntax_pattern_parser::syntax::parse("consume 😀 %*color%", index.typing.as_ref().unwrap().catalog.plural_rules()).unwrap();
    index.patterns = vec![Pattern {
        documentation: None, kind: SyntaxKind::Effect, label: "public unicode prefix".into(),
        detail: "public fixture".into(), identity: "public:unicode-slot".into(),
        graph: continuation::Graph::new(&parsed.elements).unwrap(),
    }];
    let start = source.find("dark").unwrap();
    let position = LineIndex::new(source).position(source, start + 2).unwrap();
    let list = view.at(position, false).unwrap();
    let item = values(&list).into_iter().find(|i| i.label == "dark grey").unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else { panic!() };
    assert_eq!(edit.range.start.character, "    consume 😀 ".encode_utf16().count() as u32);
    assert_eq!(edit.range.end, position);
    assert_eq!(edit.new_text, "da");
    // An adjacent grammar argument is not owned by the item literal proposal.
    let source = "on load:\n    dye stone red\n";
    let (_, view, _, _) = parse(source);
    let offset = source.find("stone").unwrap() + 2;
    let position = LineIndex::new(source).position(source, offset).unwrap();
    let list = view.at(position, false).unwrap();
    let item = values(&list).into_iter().find(|i| i.label == "stone").unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else { panic!() };
    assert_eq!(edit.range.end, position);
    assert_eq!(edit.new_text, "st");
    assert_eq!(&source[offset..], "one red\n");
}

#[test]
fn literals_require_a_current_literal_capable_slot_and_finite_data() {
    let source = "on load:\n    consume s\n";
    let (early, mut view, _, _) = parse(source);
    drop(early);
    let start = source.rfind('s').unwrap();
    let position = LineIndex::new(source).position(source, start + 1).unwrap();
    for (pattern, wanted) in [
        ("consume %~itemtype%", None),
        ("consume %*itemtype%", Some("stone")),
        ("consume %*gamemode%", Some("survival")),
        ("consume %*slot%", None), // A parser without enumerable values.
    ] {
        let index = Arc::get_mut(&mut view.index).unwrap();
        let parsed = syntax_pattern_parser::syntax::parse(pattern, index.typing.as_ref().unwrap().catalog.plural_rules()).unwrap();
        index.patterns = vec![Pattern {
            documentation: None, kind: SyntaxKind::Effect, label: pattern.into(),
            detail: "public generic slot fixture".into(), identity: "public:literal-slot".into(),
            graph: continuation::Graph::new(&parsed.elements).unwrap(),
        }];
        let list = view.at(position, false).unwrap();
        if let Some(wanted) = wanted {
            assert!(values(&list).iter().any(|i| i.label == wanted), "{pattern}");
        } else {
            assert!(values(&list).is_empty(), "{pattern}");
        }
    }
    Arc::get_mut(&mut view.index).unwrap().typing = None;
    assert!(values(&view.at(position, false).unwrap()).is_empty());
    assert!(!supported("(red|blue)", TypeLiteralSource::ParserPattern));
    assert!(!supported("%number%", TypeLiteralSource::ParserPattern));
    assert!(supported("dark grey", TypeLiteralSource::ParserPattern));
    assert!(!supported("red.*", TypeLiteralSource::ParserPattern));
}

#[test]
fn narrowed_prefix_reaches_values_beyond_the_menu_cap_and_plural_is_spelling() {
    let source = "on load:\n    consume \n    consume sto\n";
    let (early, mut view, _, _) = parse(source);
    drop(early);
    let index = Arc::get_mut(&mut view.index).unwrap();
    let parsed = syntax_pattern_parser::syntax::parse("consume %*itemtype%", index.typing.as_ref().unwrap().catalog.plural_rules()).unwrap();
    index.patterns = vec![Pattern {
        documentation: None, kind: SyntaxKind::Effect, label: "public single literal slot".into(),
        detail: "public fixture".into(), identity: "public:single-slot".into(),
        graph: continuation::Graph::new(&parsed.elements).unwrap(),
    }];
    for (needle, broad) in [("consume ", true), ("consume sto", false)] {
        let offset = if broad { source.find(needle).unwrap() } else { source.rfind(needle).unwrap() } + needle.len();
        let position = LineIndex::new(source).position(source, offset).unwrap();
        let list = view.at(position, false).unwrap();
        assert!(!values(&list).is_empty());
        assert!(list.items.len() <= MAX_ITEMS);
        if broad {
            assert!(list.is_incomplete);
        } else {
            assert!(values(&list).iter().any(|i| i.label == "stone"));
            assert!(values(&list).iter().any(|i| i.label == "stones" && i.data.as_ref().unwrap()["typeValue"]["pluralSpelling"] == true));
        }
    }
}

#[test]
fn stale_provider_certificate_withdraws_value_proposals() {
    let source = "on load:\n    dye sto red\n";
    let (mut early, mut view, _, _) = parse(source);
    let catalog = parameters::tests::catalog();
    let mut index = CatalogIndex::with_documentation_format(&catalog,
        skript_parser::RawTreeOptions::for_skript_version(2, 15), lsp_types::MarkupKind::PlainText);
    let mut host = crate::new_parser_host(parser_wasm::HostConfig {
        syntax_catalog: Some(catalog), ..Default::default()
    }).unwrap();
    index.certify_types(&host);
    let index = Arc::new(index);
    early.document.index = Arc::clone(&index);
    view.index = index;
    let position = LineIndex::new(source).position(source, source.find("sto").unwrap() + 3).unwrap();
    assert!(!values(&view.at(position, false).unwrap()).is_empty());
    assert!(host.load_addon(&[]).is_err());
    assert!(values(&view.at(position, false).unwrap()).is_empty());
    if let PreparedQuery::Ready(list) = early.at(position, false).unwrap() {
        assert!(values(&list).is_empty());
    }
}

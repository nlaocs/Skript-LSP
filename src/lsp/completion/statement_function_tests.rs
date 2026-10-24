use super::*;
use super::parameters::tests::parse;

const DECLARATIONS: &str = "function publicNotice(p: player):\n    send \"Public notice\" to {_p}\n\nfunction publicCount(n: number) :: number:\n    return {_n} + 1\n\n";

fn function<'a>(list: &'a CompletionList, name: &str) -> Option<&'a CompletionItem> {
    list.items.iter().find(|item| item.kind == Some(CompletionItemKind::FUNCTION)
        && item.label.starts_with(&format!("{name}(")))
}

#[test]
fn void_function_proposals_follow_native_statement_applicability() {
    for (body, needle, wanted) in [
        ("on join:\n    publicNo", "publicNo", true),
        ("command /public_probe:\n    trigger:\n        publicNo", "publicNo", true),
        ("on join:\n    if player is op:\n        publicNo", "publicNo", true),
        ("on join:\n    set {_n} to publicNo", "publicNo", false),
        ("on join:\n    send publicNo", "publicNo", false),
        ("on join:\n    publicCount(publicNo)", "publicNo", false),
        ("on join:\n    if publicNo:\n        stop", "publicNo", false),
        ("publicNo", "publicNo", false),
    ] {
        let source = format!("{DECLARATIONS}{body}\n");
        let (early, view, _, _) = parse(&source);
        let offset = source.rfind(needle).unwrap() + needle.len();
        let position = LineIndex::new(&source).position(&source, offset).unwrap();
        let list = view.at(position, true).unwrap();
        assert_eq!(function(&list, "publicNotice").is_some(), wanted, "{body}");
        if wanted {
            let item = function(&list, "publicNotice").unwrap();
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else { panic!() };
            assert_eq!(edit.new_text, "publicNotice(${1:p})");
            assert_eq!(edit.range.start, LineIndex::new(&source).position(&source, offset - needle.len()).unwrap());
            assert_eq!(edit.range.end, position);
            assert!(item.detail.as_ref().unwrap().starts_with("return: none (declared);"));
        }
        // Prepared syntax deliberately has no frozen document function namespace.
        if let PreparedQuery::Ready(list) = early.at(position, true).unwrap() {
            assert!(function(&list, "publicNotice").is_none());
        }
    }
}

#[test]
fn statement_function_edits_keep_arguments_and_ignore_keyword_prefix_collisions() {
    for (line, snippets, expected) in [
        ("    publicNo", true, "publicNotice(${1:p})"),
        ("    publicNo", false, "publicNotice("),
        ("    publicNo(player)", true, "publicNotice"),
        ("    publicNo  (\"🧪\")", true, "publicNotice"),
    ] {
        let source = format!("{DECLARATIONS}on join:\n{line}\n");
        let (_, view, _, _) = parse(&source);
        let offset = source.rfind("publicNo").unwrap() + "publicNo".len();
        let position = LineIndex::new(&source).position(&source, offset).unwrap();
        let list = view.at(position, snippets).unwrap();
        let item = function(&list, "publicNotice").unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else { panic!() };
        assert_eq!(edit.new_text, expected);
        assert_eq!(edit.range.end, position);
        if line.contains('(') {
            assert_eq!(item.insert_text_format, Some(InsertTextFormat::PLAIN_TEXT));
            assert_eq!(&source[offset..source.len()-1], &line["    publicNo".len()..]);
            assert!(item.data.as_ref().unwrap()["nlaocs.lsp.completion-edit"].is_object());
        }
    }
    let source = format!("# 🧪\r\n{DECLARATIONS}on join:\r\n    publicNo(player) # 保持\r\n");
    let (_, mut view, _, _) = parse(&source);
    let offset = source.rfind("publicNo").unwrap() + 8;
    let lines = LineIndex::new(&source);
    let position = lines.position(&source, offset).unwrap();
    let list = view.at(position, true).unwrap();
    let item = function(&list, "publicNotice").unwrap();
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else { panic!() };
    assert_eq!(edit.range, lsp_types::Range::new(lines.position(&source, offset - 8).unwrap(), position));
    assert_eq!(edit.new_text, "publicNotice");
    view.context.get_mut(&position.line).unwrap().scope = Scope::Conditions;
    assert!(function(&view.at(position, true).unwrap(), "publicNotice").is_none());
    let source = "function stopWork():\n    stop\non join:\n    sto\n";
    let (_, view, _, _) = parse(source);
    let position = LineIndex::new(source).position(source, source.rfind("sto\n").unwrap()+3).unwrap();
    let list = view.at(position, true).unwrap();
    let item = function(&list, "stopWork").expect("statement call also competes with keyword templates");
    let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else { panic!() };
    assert_eq!(edit.new_text, "stopWork()");
    assert!(list.items.iter().any(|item| item.label.starts_with("stop") && item.kind != Some(CompletionItemKind::FUNCTION)));
}

#[test]
fn completed_void_statement_is_valid_and_current_namespace_is_required() {
    let source = format!("{DECLARATIONS}on join:\n    publicNotice(player)\n    set {{_n}} to publicCo\n");
    let (_, mut view, result, _) = parse(&source);
    assert!(result.component_failures.is_empty());
    // The unfinished RHS is the only incomplete statement, not the void call.
    let position = LineIndex::new(&source).position(&source, source.rfind("publicCo").unwrap()+8).unwrap();
    assert!(function(&view.at(position, false).unwrap(), "publicCount").is_some());
    view.function_namespace_complete = false;
    assert!(function(&view.at(position, false).unwrap(), "publicCount").is_none());
    let complete = format!("{DECLARATIONS}on join:\n    publicNotice(player)\n");
    let (_, _, result, _) = parse(&complete);
    assert!(result.syntax.diagnostics.is_empty(), "{:?}", result.syntax.diagnostics);
}

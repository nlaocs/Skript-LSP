use std::collections::HashMap;

use crate::nlaocs::skript_parser_addon::types::{
    Diagnostic, DiagnosticSeverity, GeneratedRawNode, GeneratedRawNodeKind, GeneratedRawTree,
    HookDecision, HookEffects, RawNodeKind, RawTree, RawTreeNode, ReplaceNodeEdit,
    RetainedChildren, RetainedChildrenPlacement, TreeEdit, TreeMacroInput, TreeMacroOutput,
};

pub(crate) fn expand(input: TreeMacroInput) -> TreeMacroOutput {
    let Some(target) = input.tree.nodes.iter().find(|node| node.id == input.target) else {
        return unchanged(Vec::new());
    };

    // An option replacement is one pass in Skript. Generated nodes re-enter the
    // Tree macro pipeline, so do not recursively expand option values.
    if target.syntax_context != 0
        || !matches!(target.kind, RawNodeKind::Simple | RawNodeKind::Section)
        || inside_options(&input.tree, target)
    {
        return unchanged(Vec::new());
    }

    let all_options = collect_options(&input.tree, None);
    let options = if target.parent.is_none() {
        collect_options(&input.tree, Some(target.id))
    } else {
        all_options.clone()
    };
    let first = replace_options(&target.text, &options);
    let mut text = first.text;
    let mut missing = first.missing;

    // StructCommand.load and StructFunction.preLoad call replaceOptions again,
    // after every top-level StructOptions has been initialized.
    if target.parent.is_none() && preload_replaces_options(&text) {
        let second = replace_options(&text, &all_options);
        text = second.text;
        missing.extend(second.missing);
    }

    let diagnostics = missing
        .into_iter()
        .map(|name| Diagnostic {
            code: "core.options.undefined".to_owned(),
            message: format!("undefined option {{@{name}}}"),
            severity: DiagnosticSeverity::Error,
            span: target
                .code_span
                .clone()
                .unwrap_or_else(|| target.span.clone()),
            related: Vec::new(),
        })
        .collect::<Vec<_>>();
    if text == target.text {
        return unchanged(diagnostics);
    }

    let (kind, retained_children) = match target.kind {
        RawNodeKind::Simple => (GeneratedRawNodeKind::Simple, None),
        RawNodeKind::Section => (
            GeneratedRawNodeKind::Section,
            Some(RetainedChildren {
                target: 0,
                placement: RetainedChildrenPlacement::Append,
            }),
        ),
        _ => return unchanged(diagnostics),
    };
    TreeMacroOutput {
        decision: HookDecision::ContinueProcessing,
        edit: Some(TreeEdit::ReplaceNode(ReplaceNodeEdit {
            replacement: GeneratedRawTree {
                roots: vec![0],
                nodes: vec![GeneratedRawNode {
                    id: 0,
                    kind,
                    text,
                    children: Vec::new(),
                }],
            },
            retained_children,
        })),
        effects: effects(diagnostics),
    }
}

fn inside_options(tree: &RawTree, target: &RawTreeNode) -> bool {
    let mut current = Some(target.id);
    while let Some(id) = current {
        let Some(node) = tree.nodes.iter().find(|node| node.id == id) else {
            return false;
        };
        if node.parent.is_none() && is_options_root(node) {
            return true;
        }
        current = node.parent;
    }
    false
}

fn collect_options(tree: &RawTree, stop_before: Option<u64>) -> HashMap<String, String> {
    let mut options = HashMap::new();
    for root_id in &tree.roots {
        if Some(*root_id) == stop_before {
            break;
        }
        let Some(root) = tree.nodes.iter().find(|node| node.id == *root_id) else {
            continue;
        };
        if is_options_root(root) {
            collect_section(tree, root, "", &mut options);
        }
    }
    options
}

fn collect_section(
    tree: &RawTree,
    section: &RawTreeNode,
    prefix: &str,
    options: &mut HashMap<String, String>,
) {
    for child_id in &section.children {
        let Some(child) = tree.nodes.iter().find(|node| node.id == *child_id) else {
            continue;
        };
        match child.kind {
            RawNodeKind::Simple => {
                if let Some((key, value)) = child.text.split_once(':') {
                    options.insert(format!("{prefix}{}", key.trim()), value.trim().to_owned());
                }
            }
            RawNodeKind::Section => {
                let child_prefix = format!("{prefix}{}.", child.text.trim());
                collect_section(tree, child, &child_prefix, options);
            }
            RawNodeKind::Blank | RawNodeKind::Comment | RawNodeKind::Invalid => {}
        }
    }
}

fn is_options_root(node: &RawTreeNode) -> bool {
    node.parent.is_none()
        && matches!(node.kind, RawNodeKind::Section)
        && node.text.trim().eq_ignore_ascii_case("options")
}

fn preload_replaces_options(text: &str) -> bool {
    let text = text.trim_start();
    text.get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("command "))
        || text
            .get(..9)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("function "))
}

struct Replacement {
    text: String,
    missing: Vec<String>,
}

fn replace_options(input: &str, options: &HashMap<String, String>) -> Replacement {
    let mut text = String::with_capacity(input.len());
    let mut missing = Vec::new();
    let mut rest = input;
    while let Some(start) = rest.find("{@") {
        text.push_str(&rest[..start]);
        let marker = &rest[start..];
        let Some(end) = marker[2..].find('}') else {
            text.push_str(marker);
            rest = "";
            break;
        };
        let end = end + 2;
        let name = &marker[2..end];
        if let Some(value) = options.get(name) {
            text.push_str(value);
        } else {
            text.push_str(&marker[..=end]);
            missing.push(name.to_owned());
        }
        rest = &marker[end + 1..];
    }
    text.push_str(rest);
    Replacement { text, missing }
}

fn unchanged(diagnostics: Vec<Diagnostic>) -> TreeMacroOutput {
    TreeMacroOutput {
        decision: HookDecision::ContinueProcessing,
        edit: None,
        effects: effects(diagnostics),
    }
}

fn effects(diagnostics: Vec<Diagnostic>) -> HookEffects {
    HookEffects {
        diagnostics,
        context_updates: Vec::new(),
        parse_requests: Vec::new(),
        parse_results: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_is_single_pass_and_preserves_missing_markers() {
        let options = HashMap::from([
            ("first".to_owned(), "{@second}".to_owned()),
            ("second".to_owned(), "value".to_owned()),
        ]);
        let result = replace_options("{@first} {@missing}", &options);

        assert_eq!(result.text, "{@second} {@missing}");
        assert_eq!(result.missing, ["missing"]);
    }

    #[test]
    fn only_command_and_function_repeat_replacement_after_preload() {
        assert!(preload_replaces_options("command /test"));
        assert!(preload_replaces_options("FUNCTION test()"));
        assert!(!preload_replaces_options("on load"));
    }
}

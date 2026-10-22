//! Consume the call parser's selected parameter-to-child mapping. Neither the
//! function grammar nor overload/scope resolution is repeated for display.
use super::*;
use lsp_types::InlayHintKind;
use parser_wasm::host::semantic_proof::SemanticWitnesses;
use skript_parser::{FunctionArgument, FunctionCall, TextRange};

pub(super) fn hints(
    node: &AstNode,
    nodes: &HashMap<u64, &AstNode>,
    witnesses: &SemanticWitnesses<'_>,
    source: &str,
    lines: &LineIndex,
    blocked: &[(u64, u64)],
) -> Vec<InlayHint> {
    let Some(call) = &node.function else {
        return Vec::new();
    };
    if !witnesses.native_valid(node) {
        return Vec::new();
    }
    let Some(parent) = exact(node, source, blocked) else {
        return Vec::new();
    };
    let arguments = call
        .arguments
        .iter()
        .map(|argument| {
            Some(FunctionArgument {
                parameter_name: argument.parameter_name.clone(),
                supplied_name: argument.supplied_name.clone(),
                child_start: usize::try_from(argument.child_start).ok()?,
                child_count: usize::try_from(argument.child_count).ok()?,
                omitted: argument.omitted,
            })
        })
        .collect::<Option<Vec<_>>>();
    let Some(arguments) = arguments else {
        return Vec::new();
    };
    let binding = FunctionCall {
        name: call.name.clone(),
        definition_id: call.definition_id.clone(),
        registration_id: call.registration_id.clone(),
        arguments,
    };
    if binding.validate_children(node.children.len()).is_err()
        || node.children.iter().collect::<HashSet<_>>().len() != node.children.len()
    {
        return Vec::new();
    }

    let mut hints = Vec::new();
    for argument in &binding.arguments {
        // Omitted defaults have no explicit source argument. Named arguments
        // already display their name. Plural bindings retain every owned child.
        if argument.omitted
            || argument.supplied_name.is_some()
            || argument.parameter_name.chars().any(char::is_control)
        {
            continue;
        }
        for id in &node.children[argument.child_start..argument.child_start + argument.child_count]
        {
            let Some(child) = nodes.get(id) else { continue };
            if !witnesses.native_valid(child) {
                continue;
            }
            let Some(range) = exact(child, source, blocked) else {
                continue;
            };
            if range.start < parent.start || range.end > parent.end {
                continue;
            }
            let Ok(position) = lines.position(source, range.start) else {
                continue;
            };
            hints.push(InlayHint {
                position,
                label: InlayHintLabel::String(format!("{}:", argument.parameter_name)),
                kind: Some(InlayHintKind::PARAMETER),
                text_edits: None,
                tooltip: Some(InlayHintTooltip::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: super::super::hover_text::code_block(
                        &format!("{}: parameter of {}", argument.parameter_name, call.name),
                        "text",
                    ),
                })),
                padding_left: Some(false),
                padding_right: Some(true),
                data: None,
            });
        }
    }
    hints
}

fn exact(node: &AstNode, source: &str, blocked: &[(u64, u64)]) -> Option<TextRange> {
    use parser_wasm::bindings::nlaocs::skript_parser_addon::types::OriginKind;
    let [origin] = node.span.origins.as_slice() else {
        return None;
    };
    if !matches!(origin.kind, OriginKind::Exact)
        || blocked.iter().any(|(start, end)| {
            *start < origin.original_range.end && origin.original_range.start < *end
        })
    {
        return None;
    }
    let range = TextRange::new(
        usize::try_from(origin.original_range.start).ok()?,
        usize::try_from(origin.original_range.end).ok()?,
    );
    (range.start < range.end && source.get(range.start..range.end)? == node.text).then_some(range)
}

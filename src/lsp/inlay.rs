//! Read-only hints from adopted argument bindings and initialized defaults.
mod function;
use super::positions::LineIndex;
use lsp_types::{InlayHint, InlayHintLabel, InlayHintTooltip, MarkupContent, MarkupKind, Range};
use parser_wasm::{
    DocumentParseResult,
    host::{
        AstNode, CaptureValue, DefaultExpressionInfo, MappedSpan, PatternOmissionKind,
        source_geometry,
    },
};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use syntax_pattern_parser::syntax::{PatternElement, Span, SpannedPatternElement};
use syntaxes::Catalog;

const DEFAULT_SOURCE_SCHEMA: &str = "nlaocs.skript.default-source";

pub(super) struct Document {
    source: Arc<str>,
    lines: LineIndex,
    hints: Vec<InlayHint>,
    variable_hints:Vec<InlayHint>,
    transfer_certificate:Option<parser_wasm::host::semantic_transfer::Certificate>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DefaultSource {
    source_kind: String,
    implementation_class: String,
    target_class: String,
    time: i32,
    event_values: Vec<EventValueSource>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventValueSource {
    event_class: String,
    value_class: String,
    time: i32,
    registration_id: String,
}

impl Document {
    pub fn from_result(
        result: &DocumentParseResult,
        source: Arc<str>,
        catalog: Arc<Catalog>,
    ) -> Result<Self, String> {
        if source.as_ref() != result.source.original() {
            return Err("inlay source does not match the parsed original revision".into());
        }
        let witnesses = result.semantic_witnesses();
        let lines = LineIndex::new(&source);
        let mut nodes = HashMap::new();
        for node in &result.ast.nodes {
            if nodes.insert(node.id, node).is_some() {
                return Err("duplicate inlay AST node identity".into());
            }
        }
        let mut reachable = HashSet::new();
        let mut parents: HashMap<u64, Vec<&AstNode>> = HashMap::new();
        let mut pending = result.ast.roots.clone();
        while let Some(id) = pending.pop() {
            if !reachable.insert(id) {
                continue;
            }
            let node = nodes.get(&id).ok_or("missing inlay AST node")?;
            for child in &node.children {
                if !nodes.contains_key(child) {
                    return Err("missing inlay AST child".into());
                }
                parents.entry(*child).or_default().push(node);
                pending.push(*child);
            }
        }
        let blocked = result
            .unresolved_spans()
            .flat_map(|span| &span.origins)
            .map(|origin| (origin.original_range.start, origin.original_range.end))
            .collect::<Vec<_>>();
        let mut hints = Vec::new();
        let mut seen = HashSet::new();
        for id in reachable {
            let node = nodes[&id];
            hints.extend(function::hints(
                node, &nodes, &witnesses, &source, &lines, &blocked,
            ));
            let Some(summary) = &node.summary else {
                continue;
            };
            let Some(info) = &summary.default_expression else {
                continue;
            };
            if !witnesses.provider_valid(node, &info.component_id) {
                continue;
            }
            let Some(offset) = anchor(&node.span).filter(|_| same_span(&node.span, &info.span))
            else {
                continue;
            };
            if blocked
                .iter()
                .any(|(start, end)| *start <= offset as u64 && offset as u64 <= *end)
            {
                continue;
            }
            let Some(omission) = &info.omission else {
                continue;
            };
            if !matches!(omission.kind, PatternOmissionKind::SkippedOption) {
                continue;
            }
            let Some(data) = summary
                .public_data
                .iter()
                .find(|data| data.schema_id == DEFAULT_SOURCE_SCHEMA && data.schema_version == 1)
                .and_then(|data| serde_json::from_str::<DefaultSource>(&data.json).ok())
            else {
                continue;
            };
            if data.target_class != summary.return_type.as_deref().unwrap_or_default()
                || data.time != info.time
                || data.implementation_class.is_empty()
                || info.provider_id.is_empty()
                || info.component_id.is_empty()
            {
                continue;
            }
            let replacement = match data.source_kind.as_str() {
                "event-value" if !info.is_literal && !data.event_values.is_empty() => {
                    if data.event_values.iter().any(|value| {
                        !info.catalog_references.iter().any(|reference| {
                            reference.role == "event-value"
                                && reference.registration_id.as_deref()
                                    == Some(value.registration_id.as_str())
                        })
                    }) {
                        continue;
                    }
                    let mut names = data
                        .event_values
                        .iter()
                        .map(|value| {
                            catalog
                                .type_by_class_name(&value.value_class)
                                .map(|value| value.code_name.as_str().to_owned())
                                .unwrap_or_else(|| value.value_class.clone())
                        })
                        .collect::<Vec<_>>();
                    names.sort();
                    names.dedup();
                    format!("<event {}>", names.join(" / "))
                }
                "literal" if info.is_literal && data.event_values.is_empty() => {
                    let name = catalog
                        .type_by_class_name(&info.requested_type.class_name)
                        .map(|value| value.code_name.as_str())
                        .unwrap_or(&info.requested_type.class_name);
                    format!("<registered default: {name}>")
                }
                _ => continue,
            };
            let explanation = if info.is_literal {
                format!(
                    "{}\n\nThe stored literal value is unavailable in this snapshot. This hint identifies its registered default type and does not assert a value.",
                    info.reason
                )
            } else {
                info.reason.clone()
            };
            for parent in parents.get(&id).into_iter().flatten() {
                if !witnesses.native_valid(parent) || !owns_capture(parent, node, info, &nodes) {
                    continue;
                }
                let Some(parent_summary) = &parent.summary else {
                    continue;
                };
                let Some(registration) = &parent_summary.registration_id else {
                    continue;
                };
                if registration != &parent.syntax_id {
                    continue;
                }
                let Some(pattern_index) = parent_summary
                    .pattern_index
                    .and_then(|index| usize::try_from(index).ok())
                else {
                    continue;
                };
                let syntaxes = catalog.syntax_by_registration_id(registration);
                if syntaxes.len() != 1 {
                    continue;
                }
                let Some(common) = syntaxes[0].common() else {
                    continue;
                };
                let Some(pattern) = common.patterns.get(pattern_index) else {
                    continue;
                };
                let Ok(start) = usize::try_from(info.pattern_span.start) else {
                    continue;
                };
                let Ok(end) = usize::try_from(info.pattern_span.end) else {
                    continue;
                };
                if pattern.source.get(start..end) != Some(info.expression.as_str()) {
                    continue;
                }
                let Ok(option_start) = usize::try_from(omission.pattern_span.start) else {
                    continue;
                };
                let Ok(option_end) = usize::try_from(omission.pattern_span.end) else {
                    continue;
                };
                let Some(option) = skipped_option(
                    &pattern.parsed.elements,
                    Span::new(option_start, option_end),
                ) else {
                    continue;
                };
                let Some(label) = connective(option, Span::new(start, end), &replacement) else {
                    continue;
                };
                let position = lines.position(&source, offset)?;
                if !seen.insert((position.line, position.character, label.clone())) {
                    continue;
                }
                let evidence = data
                    .event_values
                    .iter()
                    .map(|value| {
                        format!(
                            "{} -> {} (time {}, {})",
                            value.event_class, value.value_class, value.time, value.registration_id
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                hints.push(InlayHint {
                    position,
                    label: InlayHintLabel::String(label),
                    kind: None,
                    text_edits: None,
                    tooltip: Some(InlayHintTooltip::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: render_tooltip(
                            &format!("Implicit argument: {}\nInitialized by: {} ({})\n\n{}\n\nTarget: {}\nContext: {}\n\n{}", info.expression, info.provider_id, info.component_id, explanation, data.target_class, info.event_classes.join(", "), evidence),
                            &pattern.source,
                        ),
                    })),
                    padding_left: Some(true), padding_right: Some(true), data: None,
                });
            }
        }
        hints.sort_by_key(|hint| (hint.position.line, hint.position.character));
        Ok(Self {
            source,
            lines,
            hints,
            variable_hints:Vec::new(),
            transfer_certificate:None,
        })
    }

    pub fn add_variable_types(
        &mut self,
        semantic: &super::semantic::Document,
    ) -> Result<(), String> {
        if !semantic.source_matches(&self.source) {
            return Err("inlay and semantic sources differ".into());
        }
        self.variable_hints=semantic.write_type_hints();
        self.transfer_certificate=semantic.transfer_certificate();
        Ok(())
    }

    pub fn at(&self, range: Range) -> Vec<InlayHint> {
        let Some(start) = self.lines.offset(&self.source, range.start) else {
            return Vec::new();
        };
        let Some(end) = self.lines.offset(&self.source, range.end) else {
            return Vec::new();
        };
        if start > end {
            return Vec::new();
        }
        let current=self.transfer_certificate.as_ref().is_none_or(|c|c.is_current());
        let mut hints=self.hints.iter().chain(self.variable_hints.iter().filter(|_|current))
            .filter(|hint| {
                self.lines
                    .offset(&self.source, hint.position)
                    .is_some_and(|offset| start <= offset && offset <= end)
            })
            .cloned()
            .collect::<Vec<_>>();
        hints.sort_by_key(|hint|(hint.position.line,hint.position.character));
        hints
    }
}

/// Catalog/provider strings remain literal text, including reasons and IDs.
/// Shared dynamic fences prevent any embedded backticks from ending a block.
fn render_tooltip(details: &str, pattern: &str) -> String {
    format!(
        "{}\n\nRegistered pattern:\n\n{}",
        super::hover_text::code_block(details, "text"),
        super::hover_text::code_block(pattern, "skript-pattern")
    )
}

fn anchor(span: &MappedSpan) -> Option<usize> {
    use parser_wasm::bindings::nlaocs::skript_parser_addon::types::OriginKind;
    if span.virtual_range.start != span.virtual_range.end || span.origins.is_empty() {
        return None;
    }
    let point = span.origins[0].original_range.start;
    span.origins
        .iter()
        .all(|origin| {
            // Native identity source keeps Exact point origins; expanded
            // omission source uses Anchored. Both must name one original point.
            matches!(origin.kind, OriginKind::Exact | OriginKind::Anchored)
                && origin.original_range.start == point
                && origin.original_range.end == point
        })
        .then(|| usize::try_from(point).ok())
        .flatten()
}
fn same_span(left: &MappedSpan, right: &MappedSpan) -> bool {
    left.virtual_range.start == right.virtual_range.start
        && left.virtual_range.end == right.virtual_range.end
        && left.origins.len() == right.origins.len()
        && left.origins.iter().zip(&right.origins).all(|(a, b)| {
            a.original_range.start == b.original_range.start
                && a.original_range.end == b.original_range.end
                && std::mem::discriminant(&a.kind) == std::mem::discriminant(&b.kind)
                && a.expansion == b.expansion
        })
}
fn owns_capture(
    parent: &AstNode,
    node: &AstNode,
    info: &DefaultExpressionInfo,
    nodes: &HashMap<u64, &AstNode>,
) -> bool {
    // Reuse the native sealed slot/binding contract, including routed parser IDs.
    // AST arena child names are {index}:{binding_parser_id}, not expression:{index}.
    let geometry = format!(
        "{}{}",
        source_geometry::PATTERN_CAPTURE_PREFIX,
        info.capture_index
    );
    super::semantic::capture_node(parent, info.capture_index, nodes) == Some(node.id)
        && parent.captures.iter().any(|capture| {
            capture.name == geometry
                && matches!(&capture.value, CaptureValue::Span(span) if same_span(span, &info.span))
        })
}

fn skipped_option(
    elements: &[SpannedPatternElement],
    target: Span,
) -> Option<&[SpannedPatternElement]> {
    for element in elements {
        match &element.value {
            PatternElement::Option(children) if element.span == target => return Some(children),
            PatternElement::Option(children) | PatternElement::Group(children) => {
                if let Some(found) = skipped_option(children, target) {
                    return Some(found);
                }
            }
            PatternElement::Choice(branches) => {
                for branch in branches {
                    if let Some(found) = skipped_option(branch, target) {
                        return Some(found);
                    }
                }
            }
            _ => {}
        }
    }
    None
}
fn connective(elements: &[SpannedPatternElement], slot: Span, replacement: &str) -> Option<String> {
    fn render(
        elements: &[SpannedPatternElement],
        slot: Span,
        replacement: &str,
        output: &mut String,
        found: &mut bool,
    ) -> Option<()> {
        for element in elements {
            match &element.value {
                PatternElement::Literal(text) => output.push_str(text),
                PatternElement::TypeExpr(_) if element.span == slot && !*found => {
                    output.push_str(replacement);
                    *found = true;
                }
                PatternElement::Group(children) => {
                    render(children, slot, replacement, output, found)?
                }
                PatternElement::ParseTag(_)
                | PatternElement::ParseMark(_)
                | PatternElement::Empty => {}
                // Never choose words from nested optional/choice/regex branches for display.
                _ => return None,
            }
        }
        Some(())
    }
    let mut output = String::new();
    let mut found = false;
    render(elements, slot, replacement, &mut output, &mut found)?;
    found.then(|| output.split_whitespace().collect::<Vec<_>>().join(" "))
}

#[cfg(test)]
mod tests;

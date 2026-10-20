//! Semantic colors for confirmed parts of the final, source-mapped AST.
//!
//! A parent range is not a keyword range: captures and children always take
//! their own space, including children whose classification is unsupported.
use super::positions::LineIndex;
use lsp_types::{
    SemanticToken, SemanticTokenType, SemanticTokens, SemanticTokensClientCapabilities,
    SemanticTokensFullOptions, SemanticTokensLegend, TokenFormat,
};
use parser_wasm::{
    DocumentParseResult,
    bindings::nlaocs::skript_parser_addon::types::{CaptureValue, OriginKind, SyntaxKind},
    host::{AstNode, AstTree, MappedSpan, source_geometry},
};
use skript_parser::{RawNodeKind, RawTriviaKind, TextRange};
use std::collections::{BTreeMap, HashMap, HashSet};
use syntax_pattern_parser::syntax::{PatternElement, SpannedPatternElement};
use syntaxes::Catalog;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Event,
    Condition,
    Effect,
    Expression,
    Section,
    Structure,
    Literal,
    Function,
    Variable,
    InterpolationDelimiter,
}

impl Role {
    const ALL: [Self; 10] = [
        Self::Event,
        Self::Condition,
        Self::Effect,
        Self::Expression,
        Self::Section,
        Self::Structure,
        Self::Literal,
        Self::Function,
        Self::Variable,
        Self::InterpolationDelimiter,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Event => "skriptEvent",
            Self::Condition => "skriptCondition",
            Self::Effect => "skriptEffect",
            Self::Expression => "skriptExpression",
            Self::Section => "skriptSection",
            Self::Structure => "skriptStructure",
            Self::Literal => "skriptLiteral",
            Self::Function => "function",
            Self::Variable => "variable",
            Self::InterpolationDelimiter => "skriptInterpolationDelimiter",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Legend {
    roles: Vec<Role>,
}

impl Legend {
    pub fn negotiate(capabilities: Option<&SemanticTokensClientCapabilities>) -> Option<Self> {
        let capabilities = capabilities?;
        if !capabilities.formats.contains(&TokenFormat::RELATIVE)
            || !matches!(
                capabilities.requests.full,
                Some(
                    SemanticTokensFullOptions::Bool(true) | SemanticTokensFullOptions::Delta { .. }
                )
            )
        {
            return None;
        }
        let roles = Role::ALL
            .into_iter()
            .filter(|role| {
                capabilities
                    .token_types
                    .iter()
                    .any(|kind| kind.as_str() == role.name())
            })
            .collect::<Vec<_>>();
        (!roles.is_empty()).then_some(Self { roles })
    }

    pub fn wire(&self) -> SemanticTokensLegend {
        SemanticTokensLegend {
            token_types: self
                .roles
                .iter()
                .map(|role| SemanticTokenType::new(role.name()))
                .collect(),
            token_modifiers: Vec::new(),
        }
    }
}

pub(super) struct Document {
    tokens: Vec<(lsp_types::Range, Role)>,
}

impl Document {
    pub fn from_result(
        result: &DocumentParseResult,
        source: &str,
        catalog: &Catalog,
    ) -> Result<Self, String> {
        if source != result.source.original() {
            return Err("semantic token source does not match the parsed revision".into());
        }
        let mut blocked = Vec::new();
        for span in result.unresolved_spans() {
            blocked.extend(origins(span)?);
        }
        for node in result.raw_tree.iter() {
            if matches!(node.kind, RawNodeKind::Comment | RawNodeKind::Invalid) {
                blocked.extend(
                    node.line
                        .content_span
                        .origins
                        .iter()
                        .map(|origin| origin.original_range),
                );
            }
            for trivia in &node.line.trailing_trivia {
                if matches!(
                    trivia.kind,
                    RawTriviaKind::LineComment | RawTriviaKind::BlockComment
                ) {
                    blocked.extend(
                        trivia
                            .span
                            .origins
                            .iter()
                            .map(|origin| origin.original_range),
                    );
                }
            }
        }
        let fixed = result
            .ast
            .nodes
            .iter()
            .filter(|node| fixed_parts_are_known(node, catalog))
            .map(|node| node.id)
            .collect();
        Self::from_ast(&result.ast, source, blocked, fixed)
    }

    fn from_ast(
        ast: &AstTree,
        source: &str,
        mut blocked: Vec<TextRange>,
        fixed: HashSet<u64>,
    ) -> Result<Self, String> {
        let lines = LineIndex::new(source);
        let mut nodes = BTreeMap::new();
        for node in &ast.nodes {
            if nodes.insert(node.id, node).is_some() {
                return Err("duplicate semantic token AST node ID".into());
            }
        }
        let mut depths = HashMap::new();
        let mut pending = ast.roots.iter().map(|id| (*id, 0)).collect::<Vec<_>>();
        while let Some((id, depth)) = pending.pop() {
            if depth > nodes.len() {
                return Err("cyclic semantic token AST".into());
            }
            if depths.get(&id).is_some_and(|previous| *previous >= depth) {
                continue;
            }
            let node = nodes.get(&id).ok_or("missing semantic token AST node")?;
            depths.insert(id, depth);
            pending.extend(node.children.iter().map(|child| (*child, depth + 1)));
        }
        let mut invalid_geometry = HashSet::new();
        for id in depths.keys() {
            let node = nodes[id];
            let geometry_current = source_geometry::pattern_capture_geometry_is_current(node);
            if source_geometry::has_pattern_capture_geometry(node) && !geometry_current {
                invalid_geometry.insert(*id);
            }
            for (range, origin) in origins(&node.span)?.into_iter().zip(&node.span.origins) {
                lines.range(source, range)?;
                if origin.kind != OriginKind::Exact {
                    blocked.push(range);
                }
            }
            if invalid_geometry.contains(id) {
                // All capture Span/Text evidence participates in the seal,
                // including non-reserved spans supplied by a binding.
                continue;
            }
            for capture in &node.captures {
                if let CaptureValue::Span(span) = &capture.value {
                    if capture
                        .name
                        .starts_with(source_geometry::PATTERN_CAPTURE_PREFIX)
                    {
                        // A stale/malformed parent proof is not a failure of its
                        // independently valid children or siblings.
                        if !geometry_current {
                            continue;
                        }
                        if origins(span).is_err()
                            || origins(span)?
                                .iter()
                                .any(|range| lines.range(source, *range).is_err())
                        {
                            invalid_geometry.insert(*id);
                            continue;
                        }
                    }
                    for (range, origin) in origins(span)?.into_iter().zip(&span.origins) {
                        lines.range(source, range)?;
                        if origin.kind != OriginKind::Exact {
                            blocked.push(range);
                        }
                    }
                }
            }
        }
        for range in &blocked {
            lines.range(source, *range)?;
        }
        let mut pieces = Vec::new();
        for id in depths.keys() {
            let node = nodes[id];
            if invalid_geometry.contains(id) {
                continue;
            }
            let Some(role) = classify(node) else {
                continue;
            };
            if !matches!(role, Role::Literal | Role::Variable | Role::Function)
                && !fixed.contains(id)
            {
                continue;
            }
            if !text_captures_are_located(node, source) {
                continue;
            }
            let ranges = origins(&node.span)?;
            if node
                .span
                .origins
                .iter()
                .any(|origin| origin.kind != OriginKind::Exact)
                || ranges
                    .iter()
                    .map(|range| &source[range.start..range.end])
                    .collect::<String>()
                    != node.text
            {
                continue;
            }
            let mut captures = Vec::new();
            for child in &node.children {
                captures.extend(origins(
                    &nodes.get(child).ok_or("missing semantic token child")?.span,
                )?);
            }
            let mut child_ranges = captures.clone();
            for capture in &node.captures {
                match &capture.value {
                    CaptureValue::Span(span) => captures.extend(origins(span)?),
                    CaptureValue::Node(id) => {
                        let ranges =
                            origins(&nodes.get(id).ok_or("missing semantic token capture")?.span)?;
                        child_ranges.extend(ranges.iter().copied());
                        captures.extend(ranges);
                    }
                    CaptureValue::Nodes(ids) => {
                        for id in ids {
                            let ranges = origins(
                                &nodes.get(id).ok_or("missing semantic token capture")?.span,
                            )?;
                            child_ranges.extend(ranges.iter().copied());
                            captures.extend(ranges);
                        }
                    }
                    // Its complete native slot was checked above and is also
                    // present as a sealed Span capture. Do not search the text.
                    CaptureValue::Text(_) => {}
                }
            }
            for range in &captures {
                lines.range(source, *range)?;
            }
            // A zero-width default child owns no source. Empty fault regions
            // remain in `blocked` and deliberately retain anchored exclusion.
            child_ranges.retain(|range| !range.is_empty());
            if role == Role::Expression {
                if let Some(ranges) = semantic_capture_ranges(node) {
                    for range in ranges {
                        for fragment in subtract(range, &child_ranges) {
                            for fragment in subtract(fragment, &blocked) {
                                push_piece(source, fragment, role, *id, &mut pieces);
                            }
                        }
                    }
                }
            }
            if matches!(role, Role::Literal | Role::Variable) && ranges.len() == 1 {
                let parent = ranges[0];
                if let Some(parts) = super::public_data::interpolation_parts(node) {
                    for part in parts {
                        // The parser located the complete interpolation slot.
                        // A routed/macro-edited child may cover less source;
                        // never repaint the rest with the container's role.
                        captures.push(TextRange::new(
                            parent.start + part.opening,
                            parent.start + part.closing + 1,
                        ));
                        let child = nodes
                            .get(&node.children[part.child_index])
                            .ok_or("missing interpolation child")?;
                        let inner = TextRange::new(
                            parent.start + part.opening + 1,
                            parent.start + part.closing,
                        );
                        let child_ranges = origins(&child.span)?;
                        if child.span.origins.len() != 1
                            || child.span.origins[0].kind != OriginKind::Exact
                            || child_ranges
                                .iter()
                                .any(|range| range.start < inner.start || range.end > inner.end)
                        {
                            continue;
                        }
                        for offset in [part.opening, part.closing] {
                            let delimiter =
                                TextRange::new(parent.start + offset, parent.start + offset + 1);
                            captures.push(delimiter);
                            for fragment in subtract(delimiter, &blocked) {
                                pieces.push((fragment, Role::InterpolationDelimiter, *id));
                            }
                        }
                    }
                }
            }
            for mut range in ranges {
                if role == Role::Function {
                    // Function argument values keep their own classifications.
                    // A typed call identifies its name, not its entire call span.
                    if node.span.origins.len() != 1 {
                        continue;
                    }
                    let Some(call) = &node.function else {
                        continue;
                    };
                    let text = &source[range.start..range.end];
                    let trimmed = text.trim_start();
                    let Some(tail) = trimmed.strip_prefix(&call.name) else {
                        continue;
                    };
                    if !tail.trim_start().starts_with('(') {
                        continue;
                    }
                    range.start += text.len() - trimmed.len();
                    range.end = range.start + call.name.len();
                }
                captures.retain(|capture| !capture.is_empty());
                for fragment in subtract(range, &captures) {
                    for fragment in subtract(fragment, &blocked) {
                        push_piece(source, fragment, role, *id, &mut pieces);
                    }
                }
            }
        }
        pieces.sort_by_key(|(range, _, id)| (range.start, range.end, *id));
        pieces.dedup();
        let mut tokens = Vec::new();
        let mut index = 0;
        while index < pieces.len() {
            let mut end = index + 1;
            let mut cluster_end = pieces[index].0.end;
            while end < pieces.len() && pieces[end].0.start < cluster_end {
                cluster_end = cluster_end.max(pieces[end].0.end);
                end += 1;
            }
            // Normal parent/child overlap was already removed by captures.
            // Conflicting source ownership is uncertain, independent of arena order.
            if end == index + 1 {
                tokens.push((lines.range(source, pieces[index].0)?, pieces[index].1));
            }
            index = end;
        }
        Ok(Self { tokens })
    }

    pub fn full(&self, legend: &Legend) -> SemanticTokens {
        let mut previous = lsp_types::Position::new(0, 0);
        let data = self
            .tokens
            .iter()
            .filter_map(|(range, role)| {
                let token_type = u32::try_from(
                    legend
                        .roles
                        .iter()
                        .position(|candidate| candidate == role)?,
                )
                .ok()?;
                let token = SemanticToken {
                    delta_line: range.start.line - previous.line,
                    delta_start: if range.start.line == previous.line {
                        range.start.character - previous.character
                    } else {
                        range.start.character
                    },
                    length: range.end.character - range.start.character,
                    token_type,
                    token_modifiers_bitset: 0,
                };
                previous = range.start;
                Some(token)
            })
            .collect();
        SemanticTokens {
            result_id: None,
            data,
        }
    }

    #[cfg(test)]
    pub fn test_fixture(source: &str) -> Self {
        let length = u32::try_from(source.encode_utf16().count()).unwrap();
        Self {
            tokens: if length == 0 {
                Vec::new()
            } else {
                vec![(
                    lsp_types::Range::new(
                        lsp_types::Position::new(0, 0),
                        lsp_types::Position::new(0, length),
                    ),
                    Role::Literal,
                )]
            },
        }
    }
}

fn push_piece(
    source: &str,
    range: TextRange,
    role: Role,
    id: u64,
    pieces: &mut Vec<(TextRange, Role, u64)>,
) {
    let text = &source[range.start..range.end];
    let mut start = 0;
    for (offset, character) in text.char_indices() {
        if character.is_whitespace() {
            if start < offset {
                pieces.push((
                    TextRange::new(range.start + start, range.start + offset),
                    role,
                    id,
                ));
            }
            start = offset + character.len_utf8();
        }
    }
    if start < text.len() {
        pieces.push((TextRange::new(range.start + start, range.end), role, id));
    }
}

fn semantic_capture_ranges(node: &AstNode) -> Option<Vec<TextRange>> {
    if !complete_capture_geometry(node) {
        return None;
    }
    let indices = super::public_data::expression_capture_roles(node)?;
    let mut ranges = Vec::new();
    for index in indices {
        let name = format!("{}{index}", source_geometry::PATTERN_CAPTURE_PREFIX);
        let CaptureValue::Span(span) = &node
            .captures
            .iter()
            .find(|capture| capture.name == name)?
            .value
        else {
            return None;
        };
        if span.origins.len() != 1 || span.origins[0].kind != OriginKind::Exact {
            return None;
        }
        ranges.extend(origins(span).ok()?);
    }
    Some(ranges)
}

fn classify(node: &AstNode) -> Option<Role> {
    let summary = node.summary.as_ref()?;
    match summary.kind.as_str() {
        "literal" if matches!(node.kind, SyntaxKind::Type | SyntaxKind::Expression) => {
            Some(Role::Literal)
        }
        "variable"
            if node.kind == SyntaxKind::Expression
                && super::public_data::variable_details(node).is_some() =>
        {
            Some(Role::Variable)
        }
        "function" if node.kind == SyntaxKind::Function && node.function.is_some() => {
            Some(Role::Function)
        }
        "event" if node.kind == SyntaxKind::Event => Some(Role::Event),
        "condition" if node.kind == SyntaxKind::Condition => Some(Role::Condition),
        "effect" if node.kind == SyntaxKind::Effect => Some(Role::Effect),
        "registered-expression" if node.kind == SyntaxKind::Expression => Some(Role::Expression),
        "section" if node.kind == SyntaxKind::Section => Some(Role::Section),
        "structure" if node.kind == SyntaxKind::Structure => Some(Role::Structure),
        _ => None,
    }
}

/// Match the adopted registration and trust native capture spans only while
/// their owner-protected source/identity seal still matches the final AST.
fn fixed_parts_are_known(node: &AstNode, catalog: &Catalog) -> bool {
    let Some((registration, definition, index)) = registered_pattern_identity(node) else {
        return false;
    };
    let mut matching = catalog
        .syntax_by_registration_id(registration)
        .into_iter()
        .filter(|syntax| {
            syntax.definition_id().as_str() == definition
                && super::hover::same_kind(syntax.kind(), node.kind)
        });
    let Some(syntax) = matching.next() else {
        return false;
    };
    if matching.next().is_some() {
        return false;
    }
    let Some(pattern) = usize::try_from(index)
        .ok()
        .and_then(|index| syntax.common()?.patterns.get(index))
    else {
        return false;
    };
    if source_geometry::has_pattern_capture_geometry(node) {
        return complete_capture_geometry(node);
    }
    // Older or addon-created nodes without a native proof stay conservative.
    slots_are_projected(&pattern.parsed.elements, node.kind == SyntaxKind::Event)
}

fn complete_capture_geometry(node: &AstNode) -> bool {
    if !source_geometry::pattern_capture_geometry_is_current(node) {
        return false;
    }
    let Ok(parents) = origins(&node.span) else {
        return false;
    };
    node.captures.iter().all(|capture| {
        if !capture
            .name
            .starts_with(source_geometry::PATTERN_CAPTURE_PREFIX)
        {
            return true;
        }
        let CaptureValue::Span(span) = &capture.value else {
            return false;
        };
        span.virtual_range.start >= node.span.virtual_range.start
            && span.virtual_range.end <= node.span.virtual_range.end
            && origins(span).is_ok_and(|ranges| {
                ranges.iter().all(|range| {
                    parents
                        .iter()
                        .any(|parent| parent.start <= range.start && range.end <= parent.end)
                })
            })
    })
}

fn text_captures_are_located(node: &AstNode, source: &str) -> bool {
    node.captures.iter().all(|capture| {
        let CaptureValue::Text(value) = &capture.value else {
            return true;
        };
        if !complete_capture_geometry(node) {
            return false;
        }
        // This signed structural payload describes the typed slot; it is not
        // source text from a routed parser. Every other Text retains the exact
        // native-slot/source equality check below.
        if source_geometry::pattern_expected_types(node, capture).is_some() {
            return true;
        }
        let Some((index, _)) = capture.name.split_once(':') else {
            return false;
        };
        let Ok(index) = index.parse::<u64>() else {
            return false;
        };
        let name = format!("{}{index}", source_geometry::PATTERN_CAPTURE_PREFIX);
        let Some(CaptureValue::Span(span)) = node
            .captures
            .iter()
            .find(|capture| capture.name == name)
            .map(|capture| &capture.value)
        else {
            return false;
        };
        origins(span).is_ok_and(|ranges| {
            ranges
                .iter()
                .all(|range| source.get(range.start..range.end).is_some())
                && ranges
                    .iter()
                    .map(|range| &source[range.start..range.end])
                    .collect::<String>()
                    == *value
        })
    })
}

fn registered_pattern_identity(node: &AstNode) -> Option<(&str, &str, u64)> {
    let summary = node.summary.as_ref()?;
    let registration = summary.registration_id.as_deref()?;
    if node.syntax_id != registration {
        return None;
    }
    Some((
        registration,
        summary.definition_id.as_deref()?,
        summary.pattern_index?,
    ))
}

fn slots_are_projected(elements: &[SpannedPatternElement], event: bool) -> bool {
    elements.iter().all(|element| match &element.value {
        PatternElement::Regex(_) => false,
        PatternElement::TypeExpr(_) => !event,
        PatternElement::Choice(branches) => branches
            .iter()
            .all(|branch| slots_are_projected(branch, event)),
        PatternElement::Group(elements) | PatternElement::Option(elements) => {
            slots_are_projected(elements, event)
        }
        _ => true,
    })
}

fn origins(span: &MappedSpan) -> Result<Vec<TextRange>, String> {
    if span.origins.is_empty() {
        return Err("semantic token has no original source origin".into());
    }
    span.origins
        .iter()
        .map(|origin| {
            Ok(TextRange::new(
                usize::try_from(origin.original_range.start)
                    .map_err(|_| "semantic token source offset overflow")?,
                usize::try_from(origin.original_range.end)
                    .map_err(|_| "semantic token source offset overflow")?,
            ))
        })
        .collect()
}

fn subtract(range: TextRange, cuts: &[TextRange]) -> Vec<TextRange> {
    let mut fragments = vec![range];
    for cut in cuts {
        fragments = fragments
            .into_iter()
            .flat_map(|fragment| {
                if cut.is_empty() {
                    // An anchored fault has no trustworthy source character to color.
                    return if fragment.start <= cut.start && cut.start < fragment.end {
                        Vec::new()
                    } else {
                        vec![fragment]
                    };
                }
                if cut.end <= fragment.start || cut.start >= fragment.end {
                    return vec![fragment];
                }
                let mut remaining = Vec::new();
                if fragment.start < cut.start {
                    remaining.push(TextRange::new(fragment.start, cut.start));
                }
                if cut.end < fragment.end {
                    remaining.push(TextRange::new(cut.end, fragment.end));
                }
                remaining
            })
            .collect();
    }
    fragments
        .into_iter()
        .filter(|fragment| !fragment.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use parser_wasm::bindings::nlaocs::skript_parser_addon::types::{
        AstContextOrigin, Capture, ExpressionPossibleReturnTypesState, ExpressionPublicData,
        FunctionCall, ParseSummary, SourceOrigin, TextRange as WitRange,
    };

    fn node(
        id: u64,
        kind: SyntaxKind,
        semantic: &str,
        source: &str,
        start: usize,
        end: usize,
    ) -> AstNode {
        AstNode {
            id,
            kind,
            syntax_id: format!("fixture:{id}"),
            text: source[start..end].to_owned(),
            span: MappedSpan {
                virtual_range: WitRange {
                    start: start as u64,
                    end: end as u64,
                },
                origins: vec![SourceOrigin {
                    original_range: WitRange {
                        start: start as u64,
                        end: end as u64,
                    },
                    kind: OriginKind::Exact,
                    expansion: None,
                }],
            },
            syntax_context: 0,
            context_origin: AstContextOrigin::Preserved,
            summary: Some(ParseSummary {
                kind: semantic.into(),
                definition_id: None,
                registration_id: None,
                element_class: None,
                pattern_index: None,
                return_type: None,
                possible_return_types: Vec::new(),
                possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
                multiplicity: None,
                default_expression: None,
                public_data: Vec::new(),
                metadata: Vec::new(),
            }),
            captures: Vec::new(),
            children: Vec::new(),
            function: None,
            metadata: Vec::new(),
        }
    }

    fn document(
        source: &str,
        roots: Vec<u64>,
        nodes: Vec<AstNode>,
        blocked: Vec<TextRange>,
    ) -> Document {
        let fixed = nodes.iter().map(|node| node.id).collect();
        Document::from_ast(&AstTree { roots, nodes }, source, blocked, fixed).unwrap()
    }

    fn decoded(document: &Document, legend: &Legend) -> Vec<(u32, u32, u32, String)> {
        let mut line = 0;
        let mut start = 0;
        document
            .full(legend)
            .data
            .into_iter()
            .map(|token| {
                if token.delta_line == 0 {
                    start += token.delta_start;
                } else {
                    line += token.delta_line;
                    start = token.delta_start;
                }
                assert!(token.length > 0);
                (
                    line,
                    start,
                    token.length,
                    legend.roles[token.token_type as usize].name().to_owned(),
                )
            })
            .collect()
    }

    fn all() -> Legend {
        Legend {
            roles: Role::ALL.to_vec(),
        }
    }

    #[test]
    fn negotiation_requires_full_relative_and_filters_custom_and_standard_types() {
        let capabilities = |types: Vec<&str>, full: bool, formats: Vec<&str>| {
            serde_json::from_value(
            serde_json::json!({"requests":{"full":full},"tokenTypes":types,"tokenModifiers":[],"formats":formats})
        ).unwrap()
        };
        assert!(Legend::negotiate(None).is_none());
        assert!(
            Legend::negotiate(Some(&capabilities(
                vec!["function"],
                false,
                vec!["relative"]
            )))
            .is_none()
        );
        assert!(Legend::negotiate(Some(&capabilities(vec!["function"], true, vec![]))).is_none());
        assert!(
            Legend::negotiate(Some(&capabilities(
                vec!["keyword", "unknown"],
                true,
                vec!["relative"]
            )))
            .is_none()
        );
        let legend = Legend::negotiate(Some(&capabilities(
            vec!["function", "skriptEffect", "unknown", "function"],
            true,
            vec!["relative"],
        )))
        .unwrap();
        assert_eq!(legend.roles, vec![Role::Effect, Role::Function]);
        assert!(legend.wire().token_modifiers.is_empty());
    }

    #[test]
    fn fixed_words_exclude_unsupported_children_and_zero_width_defaults() {
        let source = "send \"日本😀\" to {_x}";
        let split = source.find(" to").unwrap();
        let variable = source.find('{').unwrap();
        let mut parent = node(9001, SyntaxKind::Effect, "effect", source, 0, source.len());
        parent.children = vec![51, 89, 71];
        let literal = node(51, SyntaxKind::Type, "literal", source, 5, split);
        let unknown = node(
            89,
            SyntaxKind::Expression,
            "custom",
            source,
            variable,
            source.len(),
        );
        let implicit = node(71, SyntaxKind::Type, "default-expression", source, 4, 4);
        let document = document(
            source,
            vec![9001],
            vec![unknown, literal, implicit, parent],
            Vec::new(),
        );
        assert_eq!(
            decoded(&document, &all()),
            vec![
                (0, 0, 4, "skriptEffect".into()),
                (0, 5, 6, "skriptLiteral".into()),
                (0, 12, 2, "skriptEffect".into()),
            ]
        );
        assert!(
            document
                .full(&Legend {
                    roles: vec![Role::Function]
                })
                .data
                .is_empty()
        );
    }

    #[test]
    fn text_capture_parent_is_omitted_while_confirmed_children_survive() {
        let source = "say value";
        let mut parent = node(11, SyntaxKind::Effect, "effect", source, 0, source.len());
        parent.children = vec![99];
        parent.captures = vec![Capture {
            name: "raw".into(),
            value: CaptureValue::Text("say".into()),
        }];
        let child = node(99, SyntaxKind::Type, "literal", source, 4, source.len());
        let document = document(source, vec![11], vec![parent, child], Vec::new());
        assert_eq!(
            decoded(&document, &all()),
            vec![(0, 4, 5, "skriptLiteral".into())]
        );
    }

    #[test]
    fn unprojected_regex_and_event_type_captures_are_not_fixed_words() {
        use syntax_pattern_parser::syntax::{Span, Spanned};
        let wrap = |value| Spanned::new(value, Span::new(0, 1));
        let regex = vec![wrap(PatternElement::Option(vec![wrap(
            PatternElement::Regex(".+".into()),
        )]))];
        assert!(!slots_are_projected(&regex, false));
        assert!(!slots_are_projected(&regex, true));
        let slot = PatternElement::TypeExpr(syntax_pattern_parser::syntax::PatternTypeExpr {
            alternatives: Vec::new(),
            nullable: false,
            allow_literals: true,
            allow_expressions: true,
            time: 0,
        });
        let typed = vec![wrap(slot)];
        assert!(slots_are_projected(&typed, false));
        assert!(!slots_are_projected(&typed, true));
        let fixed = vec![wrap(PatternElement::Literal("load".into()))];
        assert!(slots_are_projected(&fixed, true));
        let source = "on damage victim";
        let event = node(8, SyntaxKind::Event, "event", source, 0, source.len());
        let ast = AstTree {
            roots: vec![8],
            nodes: vec![event],
        };
        let result = Document::from_ast(&ast, source, Vec::new(), HashSet::new()).unwrap();
        assert!(result.full(&all()).data.is_empty());
    }

    #[test]
    fn utf16_crlf_faults_and_unreachable_nodes_keep_original_coordinates() {
        let source = "日本😀 value\r\nnext";
        let first = node(40, SyntaxKind::Type, "literal", source, 0, source.len());
        let unreachable = node(99, SyntaxKind::Type, "literal", source, 0, 3);
        let document = document(
            source,
            vec![40],
            vec![first, unreachable],
            vec![TextRange::new(11, 16)],
        );
        assert_eq!(
            decoded(&document, &all()),
            vec![
                (0, 0, 4, "skriptLiteral".into()),
                (1, 0, 4, "skriptLiteral".into())
            ]
        );
    }

    #[test]
    fn nonexact_and_zero_width_faults_block_parent_colors_without_fallback() {
        let source = "alpha beta";
        let mut parent = node(1, SyntaxKind::Effect, "effect", source, 0, source.len());
        parent.children = vec![2];
        let mut child = node(2, SyntaxKind::Type, "literal", source, 6, 10);
        child.span.origins[0].kind = OriginKind::Replaced;
        let document = document(
            source,
            vec![1],
            vec![parent, child],
            vec![TextRange::new(2, 2)],
        );
        assert!(document.full(&all()).data.is_empty());
    }

    #[test]
    fn competing_original_ranges_are_omitted_independently_of_arena_order() {
        for reverse in [false, true] {
            let source = "value";
            let mut nodes = vec![
                node(30, SyntaxKind::Type, "literal", source, 0, 5),
                node(91, SyntaxKind::Effect, "effect", source, 0, 5),
            ];
            if reverse {
                nodes.reverse();
            }
            assert!(
                document(source, vec![30, 91], nodes, Vec::new())
                    .full(&all())
                    .data
                    .is_empty()
            );
        }
    }

    #[test]
    fn function_colors_only_its_typed_name_and_rejects_multi_origin_prefixes() {
        let source = "f(日本😀)";
        let mut call = node(
            80,
            SyntaxKind::Function,
            "function",
            source,
            0,
            source.len(),
        );
        call.function = Some(FunctionCall {
            name: "f".into(),
            definition_id: "fixture:f".into(),
            registration_id: "fixture:f#0".into(),
            arguments: Vec::new(),
        });
        call.children = vec![2];
        let child = node(2, SyntaxKind::Type, "literal", source, 2, source.len() - 1);
        let result = document(
            source,
            vec![80],
            vec![call.clone(), child.clone()],
            Vec::new(),
        );
        assert_eq!(
            decoded(&result, &all()),
            vec![
                (0, 0, 1, "function".into()),
                (0, 2, 4, "skriptLiteral".into())
            ]
        );
        let mut origin = call.span.origins[0].clone();
        origin.original_range.end = 2;
        call.span.origins[0].original_range.start = 2;
        call.span.origins.insert(0, origin);
        let result = document(source, vec![80], vec![call, child], Vec::new());
        assert_eq!(
            decoded(&result, &all()),
            vec![(0, 2, 4, "skriptLiteral".into())]
        );
    }

    fn typed_native_capture(
        source: &str,
        capture_index: usize,
        start: usize,
        end: usize,
    ) -> skript_parser::PatternCapture {
        use syntax_pattern_parser::syntax::{PatternTypeAlternative, PatternTypeExpr, Span};
        let mapped = skript_parser::MappedSource::identity(source);
        skript_parser::PatternCapture::TypeExpression {
            capture_index,
            state: skript_parser::TypeCaptureState::Explicit,
            omission: None,
            pattern_span: Span::new(0, 8),
            expression: PatternTypeExpr {
                alternatives: vec![PatternTypeAlternative {
                    name: "number".into(),
                    plural: false,
                }],
                nullable: false,
                allow_literals: true,
                allow_expressions: true,
                time: 0,
            },
            value: source[start..end].into(),
            span: skript_parser::MatchSpan {
                local_range: TextRange::new(start, end),
                mapped: mapped.map_range(TextRange::new(start, end)).unwrap(),
            },
            alternative_index: Some(0),
            resolution_id: Some(format!("actual:{capture_index}")),
        }
    }

    #[test]
    fn native_type_evidence_is_structural_and_keeps_effect_condition_and_literal_colors() {
        let source = "say 1\n1 is 2";
        let mut effect = node(1, SyntaxKind::Effect, "effect", source, 0, 5);
        effect.children = vec![2];
        let literal = node(2, SyntaxKind::Type, "literal", source, 4, 5);
        source_geometry::retain_pattern_captures(
            &mut effect,
            &[typed_native_capture(source, 0, 4, 5)],
        );
        let mut condition = node(3, SyntaxKind::Condition, "condition", source, 6, 12);
        condition.children = vec![4, 5];
        let left = node(4, SyntaxKind::Type, "literal", source, 6, 7);
        let right = node(5, SyntaxKind::Type, "literal", source, 11, 12);
        source_geometry::retain_pattern_captures(
            &mut condition,
            &[
                typed_native_capture(source, 0, 6, 7),
                typed_native_capture(source, 1, 11, 12),
            ],
        );
        for parent in [&effect, &condition] {
            assert!(source_geometry::pattern_capture_geometry_is_current(parent));
            for capture in parent.captures.iter().filter(|capture| {
                capture
                    .name
                    .starts_with(source_geometry::PATTERN_EXPECTED_TYPES_PREFIX)
            }) {
                let evidence = source_geometry::pattern_expected_types(parent, capture).unwrap();
                assert_eq!(evidence.type_codes, ["number"]);
                let mut detached = capture.clone();
                detached.value = CaptureValue::Text("[\"string\"]".into());
                assert!(source_geometry::pattern_expected_types(parent, &detached).is_none());
            }
        }
        assert_eq!(
            decoded(
                &document(
                    source,
                    vec![1, 3],
                    vec![effect, literal, condition, left, right],
                    vec![]
                ),
                &all()
            ),
            vec![
                (0, 0, 3, "skriptEffect".into()),
                (0, 4, 1, "skriptLiteral".into()),
                (1, 0, 1, "skriptLiteral".into()),
                (1, 2, 2, "skriptCondition".into()),
                (1, 5, 1, "skriptLiteral".into()),
            ]
        );
    }

    #[test]
    fn malformed_or_changed_native_type_evidence_never_colors_parent_but_keeps_healthy_nodes() {
        let source = "say 1\nhealthy";
        let mut parent = node(1, SyntaxKind::Effect, "effect", source, 0, 5);
        parent.children = vec![2];
        source_geometry::retain_pattern_captures(
            &mut parent,
            &[typed_native_capture(source, 0, 4, 5)],
        );
        let child = node(2, SyntaxKind::Type, "literal", source, 4, 5);
        let sibling = node(3, SyntaxKind::Effect, "effect", source, 6, 13);
        for mutation in [
            "unknown-reserved",
            "json",
            "empty",
            "blank-code",
            "bad-index",
            "leading-zero",
            "duplicate",
            "missing-slot",
            "wrong-kind",
            "edited-value",
            "edited-name",
            "removed",
            "duplicate-span",
        ] {
            let mut invalid = parent.clone();
            let expected = invalid
                .captures
                .iter()
                .position(|capture| {
                    capture
                        .name
                        .starts_with(source_geometry::PATTERN_EXPECTED_TYPES_PREFIX)
                })
                .unwrap();
            match mutation {
                "unknown-reserved" => invalid.captures.push(Capture {
                    name: "parser.unknown-evidence:0".into(),
                    value: CaptureValue::Text("opaque".into()),
                }),
                "json" => {
                    invalid.captures[expected].value =
                        CaptureValue::Text("{\"codes\":[\"number\"]}".into())
                }
                "empty" => invalid.captures[expected].value = CaptureValue::Text("[]".into()),
                "blank-code" => {
                    invalid.captures[expected].value = CaptureValue::Text("[\" \" ]".into())
                }
                "bad-index" => {
                    invalid.captures[expected].name =
                        format!("{}x", source_geometry::PATTERN_EXPECTED_TYPES_PREFIX)
                }
                "leading-zero" => {
                    invalid.captures[expected].name =
                        format!("{}00", source_geometry::PATTERN_EXPECTED_TYPES_PREFIX)
                }
                "duplicate" => invalid.captures.push(invalid.captures[expected].clone()),
                "missing-slot" => invalid.captures.retain(|capture| {
                    !capture
                        .name
                        .starts_with(source_geometry::PATTERN_CAPTURE_PREFIX)
                }),
                "wrong-kind" => {
                    invalid.captures[expected].value = CaptureValue::Span(parent.span.clone())
                }
                "edited-value" => {
                    invalid.captures[expected].value = CaptureValue::Text("[\"string\"]".into())
                }
                "edited-name" => {
                    invalid.captures[expected].name =
                        format!("{}1", source_geometry::PATTERN_EXPECTED_TYPES_PREFIX)
                }
                "removed" => {
                    invalid.captures.remove(expected);
                }
                "duplicate-span" => {
                    let span = invalid
                        .captures
                        .iter()
                        .find(|capture| {
                            capture
                                .name
                                .starts_with(source_geometry::PATTERN_CAPTURE_PREFIX)
                        })
                        .unwrap()
                        .clone();
                    invalid.captures.push(span);
                }
                _ => unreachable!(),
            }
            if !matches!(mutation, "edited-value" | "edited-name" | "removed") {
                // An adversarial native-input fixture exercises schema checks
                // even with a newly calculated seal. Production never reseals
                // an AST macro's edited output.
                source_geometry::retain_pattern_captures(&mut invalid, &[]);
            }
            assert_eq!(
                decoded(
                    &document(
                        source,
                        vec![1, 3],
                        vec![invalid, child.clone(), sibling.clone()],
                        vec![]
                    ),
                    &all()
                ),
                vec![
                    (0, 4, 1, "skriptLiteral".into()),
                    (1, 0, 7, "skriptEffect".into()),
                ],
                "{mutation}"
            );
        }
    }

    #[test]
    fn numeric_routed_text_still_requires_exact_source_with_native_type_evidence() {
        let source = "say 1";
        for (value, valid) in [("1", true), ("other", false)] {
            let mut parent = node(1, SyntaxKind::Effect, "effect", source, 0, 5);
            parent.children = vec![2];
            parent.captures.push(Capture {
                name: "0:fixture.raw".into(),
                value: CaptureValue::Text(value.into()),
            });
            source_geometry::retain_pattern_captures(
                &mut parent,
                &[typed_native_capture(source, 0, 4, 5)],
            );
            assert!(source_geometry::pattern_capture_geometry_is_current(
                &parent
            ));
            let child = node(2, SyntaxKind::Type, "literal", source, 4, 5);
            let actual = decoded(
                &document(source, vec![1], vec![parent, child], vec![]),
                &all(),
            );
            let expected = if valid {
                vec![
                    (0, 0, 3, "skriptEffect".into()),
                    (0, 4, 1, "skriptLiteral".into()),
                ]
            } else {
                vec![(0, 4, 1, "skriptLiteral".into())]
            };
            assert_eq!(actual, expected, "{value}");
        }
    }

    #[test]
    fn located_raw_text_uses_the_entire_native_capture_without_hiding_its_child() {
        let source = "if value";
        let mapped = skript_parser::MappedSource::identity(source);
        let mut parent = node(1, SyntaxKind::Section, "section", source, 0, source.len());
        parent.children = vec![2];
        parent.captures.push(Capture {
            name: "0:fixture.raw".into(),
            value: CaptureValue::Text("value".into()),
        });
        source_geometry::retain_pattern_captures(
            &mut parent,
            &[skript_parser::PatternCapture::Regex {
                capture_index: 0,
                pattern_span: syntax_pattern_parser::syntax::Span::new(3, 7),
                value: "value".into(),
                span: skript_parser::MatchSpan {
                    local_range: TextRange::new(3, 8),
                    mapped: mapped.map_range(TextRange::new(3, 8)).unwrap(),
                },
                groups: vec![],
            }],
        );
        // The routed child deliberately covers only part of the native slot.
        let child = node(2, SyntaxKind::Expression, "literal", source, 3, 6);
        let result = document(source, vec![1], vec![parent.clone(), child.clone()], vec![]);
        assert_eq!(
            decoded(&result, &all()),
            vec![
                (0, 0, 2, "skriptSection".into()),
                (0, 3, 3, "skriptLiteral".into())
            ]
        );
        let CaptureValue::Span(span) = &mut parent.captures.last_mut().unwrap().value else {
            panic!()
        };
        span.origins[0].original_range.end = 6;
        let result = document(source, vec![1], vec![parent, child], vec![]);
        assert_eq!(
            decoded(&result, &all()),
            vec![(0, 3, 3, "skriptLiteral".into())]
        );
    }

    #[test]
    fn parser_confirmed_delimiters_do_not_depend_on_trimmed_child_boundaries() {
        let source = "{_x% value %}";
        let mut parent = node(
            1,
            SyntaxKind::Expression,
            "variable",
            source,
            0,
            source.len(),
        );
        parent.children = vec![2];
        parent.summary.as_mut().unwrap().public_data = vec![
            ExpressionPublicData { schema_id: "nlaocs.skript.variable".into(), schema_version: 1,
                json: r#"{"scope":"local","name":[{"kind":"text","text":"x"},{"kind":"expression","childIndex":0}]}"#.into() },
            ExpressionPublicData { schema_id: "nlaocs.skript.interpolation".into(), schema_version: 1,
                json: r#"{"expressions":[{"childIndex":0,"opening":3,"closing":11}]}"#.into() },
        ];
        let child = node(
            2,
            SyntaxKind::Expression,
            "registered-expression",
            source,
            5,
            10,
        );
        let result = document(source, vec![1], vec![parent.clone(), child.clone()], vec![]);
        assert_eq!(
            decoded(&result, &all()),
            vec![
                (0, 0, 3, "variable".into()),
                (0, 3, 1, "skriptInterpolationDelimiter".into()),
                (0, 5, 5, "skriptExpression".into()),
                (0, 11, 1, "skriptInterpolationDelimiter".into()),
                (0, 12, 1, "variable".into()),
            ]
        );
        let narrowed = node(
            2,
            SyntaxKind::Expression,
            "registered-expression",
            source,
            5,
            6,
        );
        let result = document(source, vec![1], vec![parent.clone(), narrowed], vec![]);
        assert_eq!(
            decoded(&result, &all()),
            vec![
                (0, 0, 3, "variable".into()),
                (0, 3, 1, "skriptInterpolationDelimiter".into()),
                (0, 5, 1, "skriptExpression".into()),
                (0, 11, 1, "skriptInterpolationDelimiter".into()),
                (0, 12, 1, "variable".into()),
            ],
            "the unclassified remainder of a known interpolation slot has no parent fallback"
        );
        for mutation in 0..4 {
            let mut malformed = parent.clone();
            let record = &mut malformed.summary.as_mut().unwrap().public_data[1];
            match mutation {
                0 => record.schema_version = 2,
                1 => {
                    record.json =
                        r#"{"expressions":[{"childIndex":1,"opening":3,"closing":11}]}"#.into()
                }
                2 => {
                    record.json =
                        r#"{"expressions":[{"childIndex":0,"opening":3,"closing":999}]}"#.into()
                }
                _ => {
                    let duplicate = record.clone();
                    malformed
                        .summary
                        .as_mut()
                        .unwrap()
                        .public_data
                        .push(duplicate);
                }
            }
            let result = document(source, vec![1], vec![malformed, child.clone()], vec![]);
            assert!(
                result
                    .tokens
                    .iter()
                    .all(|(_, role)| *role != Role::InterpolationDelimiter)
            );
            assert!(
                result
                    .tokens
                    .iter()
                    .any(|(_, role)| *role == Role::Expression)
            );
        }
    }

    #[test]
    fn malformed_reserved_geometry_omits_the_parent_but_preserves_healthy_children() {
        let source = "if value\nsend";
        let mut parent = node(1, SyntaxKind::Section, "section", source, 0, 8);
        parent.children = vec![2];
        let mut broken = parent.span.clone();
        broken.origins[0].original_range.end = 999;
        parent.captures.push(Capture {
            name: "parser.pattern-capture:0".into(),
            value: CaptureValue::Span(broken),
        });
        let mut missing_origin = parent.span.clone();
        missing_origin.origins.clear();
        parent.captures.push(Capture {
            name: "0:binding".into(),
            value: CaptureValue::Span(missing_origin),
        });
        let mut anchored = parent.span.clone();
        anchored.origins[0].kind = OriginKind::Anchored;
        parent.captures.push(Capture {
            name: "1:binding".into(),
            value: CaptureValue::Span(anchored),
        });
        let child = node(2, SyntaxKind::Expression, "literal", source, 3, 8);
        let sibling = node(3, SyntaxKind::Effect, "effect", source, 9, 13);
        let result = document(source, vec![1, 3], vec![parent, child, sibling], vec![]);
        assert_eq!(
            decoded(&result, &all()),
            vec![
                (0, 3, 5, "skriptLiteral".into()),
                (1, 0, 4, "skriptEffect".into())
            ]
        );
    }

    #[test]
    fn variable_schema_is_required_and_interpolated_child_keeps_its_role() {
        let source = "{_x%value%}";
        let mut variable = node(
            10,
            SyntaxKind::Expression,
            "variable",
            source,
            0,
            source.len(),
        );
        variable.children = vec![18];
        let child = node(
            18,
            SyntaxKind::Expression,
            "registered-expression",
            source,
            4,
            9,
        );
        for version in [1, 2] {
            variable.summary.as_mut().unwrap().public_data = vec![ExpressionPublicData {
                schema_id: "nlaocs.skript.variable".into(), schema_version: version,
                json: r#"{"scope":"local","name":[{"kind":"text","text":"x"},{"kind":"expression","childIndex":0}]}"#.into(),
            }];
            let result = document(
                source,
                vec![10],
                vec![variable.clone(), child.clone()],
                Vec::new(),
            );
            let types = result
                .tokens
                .iter()
                .map(|(_, role)| *role)
                .collect::<Vec<_>>();
            assert_eq!(
                types,
                if version == 1 {
                    vec![Role::Variable, Role::Expression, Role::Variable]
                } else {
                    vec![Role::Expression]
                }
            );
        }
    }

    #[test]
    fn conflicting_macro_summary_and_node_kind_have_no_inferred_classification() {
        let source = "value";
        let mut malformed = node(19, SyntaxKind::Effect, "literal", source, 0, source.len());
        assert!(classify(&malformed).is_none());
        malformed.summary.as_mut().unwrap().kind = "variable".into();
        malformed.summary.as_mut().unwrap().public_data = vec![ExpressionPublicData {
            schema_id: "nlaocs.skript.variable".into(),
            schema_version: 1,
            json: r#"{"scope":"local","name":[{"kind":"text","text":"value"}]}"#.into(),
        }];
        assert!(classify(&malformed).is_none());
        malformed.kind = SyntaxKind::Expression;
        assert_eq!(classify(&malformed), Some(Role::Variable));
        let duplicate = malformed.summary.as_ref().unwrap().public_data[0].clone();
        malformed
            .summary
            .as_mut()
            .unwrap()
            .public_data
            .push(duplicate);
        assert!(classify(&malformed).is_none());
    }

    #[test]
    fn conflicting_macro_pattern_identity_is_not_used_for_fixed_parts() {
        let mut value = node(73, SyntaxKind::Effect, "effect", "value", 0, 5);
        let summary = value.summary.as_mut().unwrap();
        summary.definition_id = Some("fixture:definition".into());
        summary.registration_id = Some("fixture:registration".into());
        summary.pattern_index = Some(0);
        assert!(registered_pattern_identity(&value).is_none());
        value.syntax_id = "fixture:registration".into();
        assert_eq!(
            registered_pattern_identity(&value),
            Some(("fixture:registration", "fixture:definition", 0))
        );
    }

    fn role_capture_fixture(source: &str) -> AstNode {
        let mut parent = node(
            1,
            SyntaxKind::Expression,
            "registered-expression",
            source,
            0,
            source.len(),
        );
        let start = source.find('-').unwrap() + 1;
        parent.captures.push(Capture {
            name: "0:fixture.semantic".into(),
            value: CaptureValue::Text(source[start..].into()),
        });
        let mapped = skript_parser::MappedSource::identity(source);
        source_geometry::retain_pattern_captures(
            &mut parent,
            &[skript_parser::PatternCapture::Regex {
                capture_index: 0,
                pattern_span: syntax_pattern_parser::syntax::Span::new(0, 4),
                value: source[start..].into(),
                span: skript_parser::MatchSpan {
                    local_range: TextRange::new(start, source.len()),
                    mapped: mapped
                        .map_range(TextRange::new(start, source.len()))
                        .unwrap(),
                },
                groups: vec![],
            }],
        );
        parent
            .summary
            .as_mut()
            .unwrap()
            .public_data
            .push(ExpressionPublicData {
                schema_id: "nlaocs.skript.capture-roles".into(),
                schema_version: 1,
                json: r#"{"captures":[{"captureIndex":0,"role":"expression"}]}"#.into(),
            });
        parent
    }

    #[test]
    fn declared_capture_role_uses_native_geometry_without_a_spelling_exception() {
        for source in ["event-player", "event-entity", "event-日本😀"] {
            let parent = role_capture_fixture(source);
            let got = decoded(&document(source, vec![1], vec![parent], vec![]), &all());
            assert_eq!(
                got,
                vec![
                    (0, 0, 6, "skriptExpression".into()),
                    (
                        0,
                        6,
                        source[6..].encode_utf16().count() as u32,
                        "skriptExpression".into()
                    )
                ]
            );
        }
    }

    #[test]
    fn unknown_capture_contracts_do_not_color_the_slot_or_hide_healthy_fixed_words() {
        let source = "event-value";
        let good = role_capture_fixture(source);
        for (version, json) in [
            (
                2,
                r#"{"captures":[{"captureIndex":0,"role":"expression"}]}"#,
            ),
            (1, r#"{"captures":[{"captureIndex":0,"role":"unknown"}]}"#),
            (
                1,
                r#"{"captures":[{"captureIndex":7,"role":"expression"}]}"#,
            ),
            (
                1,
                r#"{"captures":[{"captureIndex":0,"role":"expression"},{"captureIndex":0,"role":"expression"}]}"#,
            ),
        ] {
            let mut parent = good.clone();
            let record = &mut parent.summary.as_mut().unwrap().public_data[0];
            record.schema_version = version;
            record.json = json.into();
            assert_eq!(
                decoded(&document(source, vec![1], vec![parent], vec![]), &all()),
                vec![(0, 0, 6, "skriptExpression".into())]
            );
        }
    }

    #[test]
    fn capture_role_keeps_nested_unknown_children_faults_and_competing_owners_excluded() {
        let source = "event-value";
        let mut parent = role_capture_fixture(source);
        parent.children = vec![2];
        let default_child = node(2, SyntaxKind::Type, "unsupported", source, 8, 8);
        assert_eq!(
            decoded(
                &document(
                    source,
                    vec![1],
                    vec![parent.clone(), default_child.clone()],
                    vec![]
                ),
                &all()
            ),
            vec![
                (0, 0, 6, "skriptExpression".into()),
                (0, 6, 5, "skriptExpression".into())
            ]
        );
        assert_eq!(
            decoded(
                &document(
                    source,
                    vec![1],
                    vec![parent.clone(), default_child],
                    vec![TextRange::new(8, 8)]
                ),
                &all()
            ),
            vec![(0, 0, 6, "skriptExpression".into())]
        );
        let child = node(2, SyntaxKind::Type, "unsupported", source, 8, 11);
        assert_eq!(
            decoded(
                &document(
                    source,
                    vec![1],
                    vec![parent.clone(), child],
                    vec![TextRange::new(7, 8)]
                ),
                &all()
            ),
            vec![
                (0, 0, 6, "skriptExpression".into()),
                (0, 6, 1, "skriptExpression".into())
            ]
        );
        let child = node(2, SyntaxKind::Type, "literal", source, 8, 11);
        let got = decoded(
            &document(source, vec![1], vec![parent.clone(), child], vec![]),
            &all(),
        );
        assert_eq!(
            got,
            vec![
                (0, 0, 6, "skriptExpression".into()),
                (0, 6, 2, "skriptExpression".into()),
                (0, 8, 3, "skriptLiteral".into())
            ]
        );
        parent.children.clear();
        let competing = node(3, SyntaxKind::Type, "literal", source, 6, 11);
        assert_eq!(
            decoded(
                &document(source, vec![1, 3], vec![parent.clone(), competing], vec![]),
                &all()
            ),
            vec![(0, 0, 6, "skriptExpression".into())]
        );
        parent.text = "changed".into();
        assert!(
            document(source, vec![1], vec![parent], vec![])
                .full(&all())
                .data
                .is_empty()
        );
    }
}

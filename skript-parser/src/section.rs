//! Recursive Section parsing over RawTree headers and bodies.
#![allow(missing_docs)] // Aggregate contracts are documented on their owning types.

use crate::{
    CandidateMatch, ConditionParseError, EffectMatches, EffectParseError, ExpressionParseContext,
    ExpressionParseEnvironment, ExpressionParseError, ExpressionParserConfig, ExpressionSession,
    FailureTrace, HOST_CONDITION_PARSER_ID, HOST_EFFECT_PARSER_ID, MappedSource, MatchPattern,
    MatchSpan, MatchSyntaxKind, ParsedCapture, ParsedCaptureResult, ParsedCaptureStatus,
    ParsedCaptureValue, PatternCandidate, PatternCapture, RawNode, RawNodeId, RawNodeKind, RawTree,
    RegisteredSyntaxIdentity, SectionChildrenDecision, SectionChildrenRequest, TextRange,
};
use std::collections::BTreeMap;
use syntaxes::{
    Catalog, ClassName, DynamicSyntaxSnapshot, Syntax, SyntaxCandidateSource, SyntaxKind,
};
use thiserror::Error;

pub struct SectionParseRequest<'a> {
    pub source: &'a MappedSource,
    pub tree: &'a RawTree,
    pub node: &'a RawNode,
    pub context: ExpressionParseContext,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SectionParserConfig {
    pub expression: ExpressionParserConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionCandidate {
    pub raw_node_id: RawNodeId,
    pub matched: CandidateMatch,
    pub element_class: Option<ClassName>,
    /// All recursively parsed captures in pattern order.
    pub parsed_captures: Vec<ParsedCapture>,
    pub loop_section: bool,
    pub effect_section: bool,
    pub section_expression: bool,
    pub body: Vec<SectionBodyNode>,
    pub handler: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

impl SectionCandidate {
    /// Iterates Condition captures resolved through generic parser bindings.
    pub fn conditions(&self) -> impl Iterator<Item = &crate::ConditionNode> {
        self.parsed_captures.iter().filter_map(|capture| {
            if let Some(crate::ParsedCaptureValue::Condition(condition)) =
                capture.result.value.as_ref()
            {
                Some(condition)
            } else {
                None
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionBodyNode {
    Section(Box<SectionMatches>),
    Effect(Box<EffectMatches>),
    Trivia(RawNodeId),
    Unclaimed(RawNodeId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionDiagnosticKind {
    Unclaimed,
    MultipleClaims,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionDiagnostic {
    pub raw_node_id: RawNodeId,
    pub kind: SectionDiagnosticKind,
    pub span: MatchSpan,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownSectionNode {
    pub raw_node_id: RawNodeId,
    pub source: String,
    pub span: MatchSpan,
    pub failure: Option<FailureTrace>,
    pub body: Vec<SectionBodyNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionMatches {
    pub selected: Option<SectionCandidate>,
    pub alternatives: Vec<SectionCandidate>,
    pub unknown: Option<UnknownSectionNode>,
    pub diagnostics: Vec<SectionDiagnostic>,
}

#[derive(Debug, Error)]
pub enum SectionParseError {
    #[error("Section parsing requires a Section RawTree node, got {actual:?}")]
    UnsupportedNodeKind { actual: RawNodeKind },
    #[error("Section RawTree node {node_id} has no code span")]
    MissingCodeSpan { node_id: RawNodeId },
    #[error("Section header does not end with a colon")]
    MissingHeaderColon,
    #[error("Section syntax context {node_context} does not match parser context {parser_context}")]
    SyntaxContextMismatch {
        node_context: u64,
        parser_context: u64,
    },
    #[error("Section range {range} is invalid for the mapped source")]
    InvalidRange { range: TextRange },
    #[error(transparent)]
    Expression(#[from] ExpressionParseError),
    #[error(transparent)]
    Condition(#[from] ConditionParseError),
    #[error(transparent)]
    Effect(#[from] EffectParseError),
    #[error("Section parser extension failed: {message}")]
    Environment { message: String },
}

pub fn parse_section<E: ExpressionParseEnvironment>(
    catalog: &Catalog,
    request: SectionParseRequest<'_>,
    environment: &mut E,
    config: SectionParserConfig,
) -> Result<SectionMatches, SectionParseError> {
    parse_section_with_snapshot(catalog, None, request, environment, config)
}

pub fn parse_section_with_snapshot<E: ExpressionParseEnvironment>(
    catalog: &Catalog,
    dynamic_snapshot: Option<&DynamicSyntaxSnapshot>,
    request: SectionParseRequest<'_>,
    environment: &mut E,
    config: SectionParserConfig,
) -> Result<SectionMatches, SectionParseError> {
    validate_section_node(request.node, &request.context)?;
    let mut session = ExpressionSession::new(
        catalog,
        dynamic_snapshot,
        request.source,
        environment,
        request.context,
        config.expression,
    );
    parse_section_with_session(&mut session, request.tree, request.node, 0)
}

fn validate_section_node(
    node: &RawNode,
    context: &ExpressionParseContext,
) -> Result<(), SectionParseError> {
    if node.kind != RawNodeKind::Section {
        return Err(SectionParseError::UnsupportedNodeKind { actual: node.kind });
    }
    let node_context = u64::from(node.syntax_context.get());
    if node_context != context.syntax_context {
        return Err(SectionParseError::SyntaxContextMismatch {
            node_context,
            parser_context: context.syntax_context,
        });
    }
    Ok(())
}

fn parse_section_with_session<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    tree: &RawTree,
    node: &RawNode,
    depth: usize,
) -> Result<SectionMatches, SectionParseError> {
    session.ensure_depth(depth)?;
    let range = section_header_range(session.source(), node)?;
    let mut candidates = section_pattern_candidates(session);
    session.retain_viable_patterns(range, &mut candidates)?;
    let matched = session.match_candidates_at_depth(range, &candidates, depth)?;
    let failure = matched.primary_failure().cloned();
    let mut ranked = matched
        .selected
        .into_iter()
        .chain(matched.alternatives)
        .collect::<Vec<_>>();
    let mut accepted = Vec::new();
    for matched in ranked.drain(..) {
        session
            .begin_semantic_candidate()
            .map_err(|message| SectionParseError::Environment { message })?;
        let candidate = section_candidate(session, node.id, matched, range.start, depth);
        let keep = candidate
            .as_ref()
            .is_ok_and(|candidate| candidate.is_some() && accepted.is_empty());
        session
            .finish_semantic_candidate(keep)
            .map_err(|message| SectionParseError::Environment { message })?;
        if let Some(candidate) = candidate? {
            accepted.push(candidate);
        }
    }
    let mut diagnostics = Vec::new();
    let mut selected = (!accepted.is_empty()).then(|| accepted.remove(0));
    let alternatives = accepted;
    if selected.is_some() && !alternatives.is_empty() {
        diagnostics.push(section_diagnostic(
            session,
            node,
            SectionDiagnosticKind::MultipleClaims,
        )?);
    }

    if let Some(candidate) = selected.as_mut() {
        let parent_context = session.context().clone();
        let request = section_children_request(
            session.source().virtual_source(),
            candidate,
            &parent_context,
        );
        let decision = session
            .environment_mut()
            .enter_section_children(request)
            .map_err(|message| SectionParseError::Environment { message })?;
        let SectionChildrenDecision::Accept(child_context) = decision else {
            let (body, child_diagnostics) = parse_section_body(session, tree, node, depth + 1)?;
            diagnostics.extend(child_diagnostics);
            diagnostics.push(section_diagnostic(
                session,
                node,
                SectionDiagnosticKind::Unclaimed,
            )?);
            let source = range
                .slice(session.source().virtual_source())
                .ok_or(SectionParseError::InvalidRange { range })?
                .to_owned();
            return Ok(SectionMatches {
                selected: None,
                alternatives,
                unknown: Some(UnknownSectionNode {
                    raw_node_id: node.id,
                    source,
                    span: session.map_range(range)?,
                    failure: failure.clone(),
                    body,
                }),
                diagnostics,
            });
        };
        let saved_context = session.replace_context(child_context);
        let body = parse_section_body(session, tree, node, depth + 1);
        let child_context = session.replace_context(saved_context);
        let request =
            section_children_request(session.source().virtual_source(), candidate, &child_context);
        let exit = session
            .environment_mut()
            .exit_section_children(request)
            .map_err(|message| SectionParseError::Environment { message });
        let (body, child_diagnostics) = body?;
        exit?;
        candidate.body = body;
        diagnostics.extend(child_diagnostics);
        return Ok(SectionMatches {
            selected,
            alternatives,
            unknown: None,
            diagnostics,
        });
    }

    let (body, child_diagnostics) = parse_section_body(session, tree, node, depth + 1)?;
    diagnostics.push(section_diagnostic(
        session,
        node,
        SectionDiagnosticKind::Unclaimed,
    )?);
    diagnostics.extend(child_diagnostics);
    let source = range
        .slice(session.source().virtual_source())
        .ok_or(SectionParseError::InvalidRange { range })?
        .to_owned();
    Ok(SectionMatches {
        selected: None,
        alternatives,
        unknown: Some(UnknownSectionNode {
            raw_node_id: node.id,
            source,
            span: session.map_range(range)?,
            failure,
            body,
        }),
        diagnostics,
    })
}

fn section_candidate<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    raw_node_id: RawNodeId,
    matched: CandidateMatch,
    frame_start: usize,
    depth: usize,
) -> Result<Option<SectionCandidate>, SectionParseError> {
    let section = session
        .catalog()
        .sections()
        .find(|section| section.common.registration_id.as_str() == matched.registration_id);
    let section_expression = session.catalog().expressions().find(|expression| {
        expression.section_expression
            && expression.common.registration_id.as_str() == matched.registration_id
    });
    let element_class = section
        .map(|section| section.common.element_class.clone())
        .or_else(|| section_expression.map(|expression| expression.common.element_class.clone()));
    let capture_bindings = session
        .environment()
        .registered_capture_bindings(RegisteredSyntaxIdentity {
            kind: SyntaxKind::Section,
            definition_id: &matched.definition_id,
            registration_id: &matched.registration_id,
            pattern_index: Some(matched.pattern_index),
        })
        .map_err(|message| SectionParseError::Environment { message })?;
    let mut parsed_captures = Vec::new();
    for (capture_index, capture) in matched.matched.captures.iter().enumerate() {
        if let PatternCapture::TypeExpression {
            resolution_id: Some(id),
            ..
        } = capture
            && let Some(node) = session.resolved_node(id).cloned()
        {
            parsed_captures.push(crate::expression_parsed_capture(capture_index, node));
            continue;
        }
        let PatternCapture::Regex { span, .. } = capture else {
            continue;
        };
        let Some(binding) = capture_bindings
            .iter()
            .find(|binding| binding.capture_index == capture_index)
        else {
            continue;
        };
        let local = span.local_range;
        let range = TextRange::new(frame_start + local.start, frame_start + local.end);
        let parsed = match binding.parser_id.as_str() {
            HOST_CONDITION_PARSER_ID => {
                let parsed =
                    crate::condition::parse_condition_with_session(session, range, depth + 1)?;
                match parsed.selected {
                    Some(selected) => {
                        let mut capture =
                            crate::condition_parsed_capture(capture_index, selected.node);
                        capture.binding = binding.clone();
                        capture
                    }
                    None if binding.required => return Ok(None),
                    None => ParsedCapture {
                        capture_index,
                        binding: binding.clone(),
                        result: ParsedCaptureResult::failure(
                            binding.parser_id.clone(),
                            span.clone(),
                            "condition capture did not match",
                        ),
                    },
                }
            }
            HOST_EFFECT_PARSER_ID => {
                let parsed = crate::effect::parse_effect_range_with_session(
                    session,
                    range,
                    raw_node_id,
                    depth + 1,
                )?;
                match parsed.selected {
                    Some(selected) => ParsedCapture {
                        capture_index,
                        binding: binding.clone(),
                        result: ParsedCaptureResult::success(
                            binding.parser_id.clone(),
                            span.clone(),
                            Some(crate::effect::effect_semantic_summary(
                                &selected,
                                session.catalog(),
                            )),
                            ParsedCaptureValue::Effect(Box::new(selected)),
                        ),
                    },
                    None if binding.required => return Ok(None),
                    None => ParsedCapture {
                        capture_index,
                        binding: binding.clone(),
                        result: ParsedCaptureResult::failure(
                            binding.parser_id.clone(),
                            span.clone(),
                            "effect capture did not match",
                        ),
                    },
                }
            }
            _ if binding.required => return Ok(None),
            _ => ParsedCapture {
                capture_index,
                binding: binding.clone(),
                result: ParsedCaptureResult::failure(
                    binding.parser_id.clone(),
                    span.clone(),
                    format!("no native parser route for {}", binding.parser_id),
                ),
            },
        };
        if parsed.result.status != ParsedCaptureStatus::Failed || !binding.required {
            parsed_captures.push(parsed);
        }
    }
    let dynamic = session.dynamic_snapshot().and_then(|snapshot| {
        snapshot
            .definitions
            .values()
            .find(|definition| definition.id.qualified() == matched.registration_id)
    });
    let loop_section = section.is_some_and(|section| section.loop_section);
    let effect_section = section.is_some_and(|section| section.effect_section);
    let section_expression = section_expression.is_some();
    Ok(Some(SectionCandidate {
        raw_node_id,
        matched,
        element_class,
        parsed_captures,
        loop_section,
        effect_section,
        section_expression,
        body: Vec::new(),
        handler: dynamic.map(|definition| definition.handler.clone()),
        metadata: dynamic.map_or_else(BTreeMap::new, |definition| definition.metadata.clone()),
    }))
}

fn section_pattern_candidates<'a, E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'a, E>,
) -> Vec<PatternCandidate<'a>> {
    let mut candidates = session.syntax_candidates(SyntaxKind::Section);
    candidates.extend(section_expression_pattern_candidates(
        session.catalog(),
        session.dynamic_snapshot(),
    ));
    candidates.sort_by_key(|candidate| {
        (
            candidate.resolved_order.unwrap_or(usize::MAX),
            candidate.registration_order,
        )
    });
    candidates
}

fn section_expression_pattern_candidates<'a>(
    catalog: &'a Catalog,
    snapshot: Option<&'a DynamicSyntaxSnapshot>,
) -> Vec<PatternCandidate<'a>> {
    match snapshot {
        Some(snapshot) => snapshot
            .candidates
            .iter()
            .enumerate()
            .filter_map(|(resolved_order, candidate)| {
                if candidate.kind != SyntaxKind::Expression {
                    return None;
                }
                let SyntaxCandidateSource::Static(index) = &candidate.source else {
                    return None;
                };
                let Some(Syntax::Expression(expression)) = catalog.syntax_at(*index) else {
                    return None;
                };
                expression
                    .section_expression
                    .then(|| section_expression_pattern_candidate(expression, Some(resolved_order)))
            })
            .collect(),
        None => catalog
            .syntaxes()
            .iter()
            .filter_map(|syntax| {
                let Syntax::Expression(expression) = syntax else {
                    return None;
                };
                expression
                    .section_expression
                    .then(|| section_expression_pattern_candidate(expression, None))
            })
            .collect(),
    }
}

fn section_expression_pattern_candidate<'a>(
    expression: &'a syntaxes::Expression,
    resolved_order: Option<usize>,
) -> PatternCandidate<'a> {
    PatternCandidate {
        kind: MatchSyntaxKind::Section,
        definition_id: expression.common.definition_id.as_str().to_owned(),
        registration_id: expression.common.registration_id.as_str().to_owned(),
        priority: 0,
        registration_order: expression.common.registration_order,
        resolved_order,
        patterns: expression
            .common
            .patterns
            .iter()
            .enumerate()
            .map(|(pattern_index, pattern)| MatchPattern {
                pattern_index,
                source: &pattern.source,
                parsed: &pattern.parsed,
            })
            .collect(),
    }
}

pub(crate) fn parse_section_body<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    tree: &RawTree,
    node: &RawNode,
    depth: usize,
) -> Result<(Vec<SectionBodyNode>, Vec<SectionDiagnostic>), SectionParseError> {
    let mut body = Vec::new();
    let mut diagnostics = Vec::new();
    for id in &node.children {
        let Some(child) = tree.get(*id) else {
            continue;
        };
        match child.kind {
            RawNodeKind::Blank | RawNodeKind::Comment => {
                body.push(SectionBodyNode::Trivia(child.id));
            }
            RawNodeKind::Simple => {
                let Some(code_span) = child.code_span.as_ref() else {
                    body.push(SectionBodyNode::Unclaimed(child.id));
                    diagnostics.push(section_diagnostic(
                        session,
                        child,
                        SectionDiagnosticKind::Unclaimed,
                    )?);
                    continue;
                };
                let matches = crate::effect::parse_effect_range_with_session(
                    session,
                    code_span.virtual_range,
                    child.id,
                    depth,
                )?;
                if matches.selected.is_none() {
                    diagnostics.push(section_diagnostic(
                        session,
                        child,
                        SectionDiagnosticKind::Unclaimed,
                    )?);
                } else if !matches.alternatives.is_empty() {
                    diagnostics.push(section_diagnostic(
                        session,
                        child,
                        SectionDiagnosticKind::MultipleClaims,
                    )?);
                }
                body.push(SectionBodyNode::Effect(Box::new(matches)));
            }
            RawNodeKind::Section => {
                let matches = parse_section_with_session(session, tree, child, depth)?;
                diagnostics.extend(matches.diagnostics.iter().cloned());
                body.push(SectionBodyNode::Section(Box::new(matches)));
            }
            RawNodeKind::Invalid => {
                body.push(SectionBodyNode::Unclaimed(child.id));
                diagnostics.push(section_diagnostic(
                    session,
                    child,
                    SectionDiagnosticKind::Unclaimed,
                )?);
            }
        }
    }
    Ok((body, diagnostics))
}

fn section_children_request<'a>(
    input: &'a str,
    candidate: &'a SectionCandidate,
    context: &'a ExpressionParseContext,
) -> SectionChildrenRequest<'a> {
    SectionChildrenRequest {
        input,
        raw_node_id: candidate.raw_node_id,
        definition_id: &candidate.matched.definition_id,
        registration_id: &candidate.matched.registration_id,
        element_class: candidate.element_class.as_ref(),
        pattern_index: candidate.matched.pattern_index,
        span: &candidate.matched.matched.span,
        loop_section: candidate.loop_section,
        effect_section: candidate.effect_section,
        section_expression: candidate.section_expression,
        captures: &candidate.matched.matched.captures,
        parsed_captures: &candidate.parsed_captures,
        context,
    }
}

fn section_header_range(
    source: &MappedSource,
    node: &RawNode,
) -> Result<TextRange, SectionParseError> {
    let range = node
        .code_span
        .as_ref()
        .ok_or(SectionParseError::MissingCodeSpan { node_id: node.id })?
        .virtual_range;
    let text = range
        .slice(source.virtual_source())
        .ok_or(SectionParseError::InvalidRange { range })?;
    let Some(header) = text.strip_suffix(':') else {
        return Err(SectionParseError::MissingHeaderColon);
    };
    let trimmed = crate::pattern_match::java_trim_range(header);
    Ok(TextRange::new(
        range.start + trimmed.start,
        range.start + trimmed.end,
    ))
}

fn section_diagnostic<E: ExpressionParseEnvironment>(
    session: &ExpressionSession<'_, E>,
    node: &RawNode,
    kind: SectionDiagnosticKind,
) -> Result<SectionDiagnostic, SectionParseError> {
    let range = node
        .code_span
        .as_ref()
        .map_or(node.span.virtual_range, |span| span.virtual_range);
    Ok(SectionDiagnostic {
        raw_node_id: node.id,
        kind,
        span: session.map_range(range)?,
    })
}

//! Recursive Section parsing over RawTree headers and bodies.
#![allow(missing_docs)] // Aggregate contracts are documented on their owning types.

use crate::{
    CandidateMatch, ConditionParseError, EffectMatches, EffectParseError, ExpressionParseContext,
    ExpressionParseEnvironment, ExpressionParseError, ExpressionParserConfig, ExpressionSession,
    FailureFrame, FailureFrameRole, FailureTrace, HOST_CONDITION_PARSER_ID, HOST_EFFECT_PARSER_ID,
    MappedSource, MatchPattern, MatchSpan, MatchSyntaxKind, ParsedCapture, ParsedCaptureResult,
    ParsedCaptureStatus, ParsedCaptureValue, PatternCandidate, PatternCapture, PatternFailure,
    PatternFailureReason, RawNode, RawNodeId, RawNodeKind, RawTree, RegisteredSyntaxIdentity,
    SectionBodyMode, SectionChildrenDecision, SectionChildrenRequest, SectionExitDecision,
    SectionRawNodeSummary, SectionSiblingSummary, TextRange,
};
use std::collections::BTreeMap;
use syntaxes::{
    Catalog, ClassName, DynamicSyntaxSnapshot, Multiplicity, PossibleReturnTypesState, Syntax,
    SyntaxCandidateSource, SyntaxKind,
};
use thiserror::Error;

pub struct SectionParseRequest<'a> {
    pub source: &'a MappedSource,
    pub tree: &'a RawTree,
    pub node: &'a RawNode,
    pub context: ExpressionParseContext,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SectionRootLifecycle {
    /// Parse the body and run both enter and exit hooks.
    #[default]
    Complete,
    /// Keep the requested root Section's enter state active after parsing.
    ///
    /// Nested Sections still complete their normal lifecycle. Callers are
    /// responsible for restoring the transaction and parser context later.
    RetainBody,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SectionParserConfig {
    pub expression: ExpressionParserConfig,
    pub root_lifecycle: SectionRootLifecycle,
}

/// Primary role of an active Section scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SectionScopeKind {
    Section,
    Loop,
    EffectSection,
    SectionExpression,
}

/// Semantic summary of one parsed Section pattern capture.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SectionScopeCapture {
    /// Implicit capture provenance available to later hooks inside this scope.
    pub default_expression: Option<crate::DefaultExpressionInfo>,
    pub capture_index: usize,
    pub parser_id: String,
    pub status: ParsedCaptureStatus,
    pub source: String,
    pub kind: Option<String>,
    pub definition_id: Option<String>,
    pub registration_id: Option<String>,
    pub element_class: Option<ClassName>,
    pub pattern_index: Option<usize>,
    pub return_type: Option<ClassName>,
    pub possible_return_types: Vec<ClassName>,
    pub possible_return_types_state: PossibleReturnTypesState,
    pub multiplicity: Option<Multiplicity>,
    pub public_data: Vec<crate::ExpressionPublicData>,
    pub metadata: BTreeMap<String, String>,
}

/// Immutable identity and semantics of one active Section body.
///
/// The parser owns this stack. Addon hooks can inspect it through their parse
/// context, but context updates cannot add, remove, or reorder frames.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SectionScopeFrame {
    pub scope_id: u64,
    pub parent_scope_id: Option<u64>,
    pub raw_node_id: RawNodeId,
    pub kind: SectionScopeKind,
    pub definition_id: String,
    pub registration_id: String,
    pub element_class: Option<ClassName>,
    pub pattern_index: usize,
    pub pattern: String,
    pub source: String,
    pub addon_name: Option<String>,
    pub addon_version: Option<String>,
    pub owner_component_id: Option<String>,
    pub loop_section: bool,
    pub effect_section: bool,
    pub section_expression: bool,
    pub captures: Vec<SectionScopeCapture>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionCandidate {
    pub raw_node_id: RawNodeId,
    pub matched: CandidateMatch,
    pub element_class: Option<ClassName>,
    pub addon_name: Option<String>,
    pub addon_version: Option<String>,
    pub owner_component_id: Option<String>,
    /// All recursively parsed captures in pattern order.
    pub parsed_captures: Vec<ParsedCapture>,
    pub loop_section: bool,
    pub effect_section: bool,
    pub section_expression: bool,
    pub body_mode: SectionBodyMode,
    pub body: Vec<SectionBodyNode>,
    pub handler: Option<String>,
    pub metadata: BTreeMap<String, String>,
    /// Context inherited by statements inside this Section body.
    pub body_context: ExpressionParseContext,
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
    Condition {
        raw_node_id: RawNodeId,
        matches: Box<crate::ConditionMatches>,
    },
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

type ParsedSectionCandidate = Result<(SectionCandidate, Vec<SectionDiagnostic>), FailureTrace>;
type SectionCandidateAttempt = Result<Option<ParsedSectionCandidate>, SectionParseError>;

#[derive(Debug, Clone, Error)]
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
    let SectionParserConfig {
        expression,
        root_lifecycle,
    } = config;
    let mut session = ExpressionSession::new(
        catalog,
        dynamic_snapshot,
        request.source,
        environment,
        request.context,
        expression,
    );
    parse_section_with_session(
        &mut session,
        request.tree,
        request.node,
        0,
        &[],
        None,
        root_lifecycle,
    )
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
    preceding_siblings: &[SectionSiblingSummary],
    next_sibling: Option<&SectionRawNodeSummary>,
    lifecycle: SectionRootLifecycle,
) -> Result<SectionMatches, SectionParseError> {
    session.ensure_depth(depth)?;
    let range = section_header_range(session.source(), node)?;
    let mut candidates = section_pattern_candidates(session);
    session.retain_viable_patterns(range, &mut candidates)?;
    let matched = session.match_candidates_at_depth(range, &candidates, depth)?;
    let mut failure = matched.primary_failure().cloned();
    let ranked = matched
        .selected
        .into_iter()
        .chain(matched.alternatives)
        .collect::<Vec<_>>();
    let initial_context = session.context().clone();
    let raw_children = raw_child_summaries(session, tree, node)?;
    let mut diagnostics = Vec::new();
    for matched in ranked {
        session.replace_context(initial_context.clone());
        let local = matched.matched.span.local_range;
        let span = session.map_range(TextRange::new(
            range.start + local.start,
            range.start + local.end,
        ))?;
        if let Some(restricted) = session.event_restriction_failure(&matched.registration_id, span)
        {
            failure = crate::choose_failure_trace(failure, Some(restricted));
            continue;
        }
        session
            .begin_semantic_candidate()
            .map_err(|message| SectionParseError::Environment { message })?;
        session
            .activate_pattern_candidate(&matched)
            .map_err(|message| SectionParseError::Environment { message })?;
        let attempt: SectionCandidateAttempt = (|| {
            let Some(mut candidate) =
                section_candidate(session, node.id, matched, range.start, depth)?
            else {
                return Ok(None);
            };
            let parent_context = session.context().clone();
            let request = section_children_request(
                session.source().virtual_source(),
                &candidate,
                &parent_context,
                preceding_siblings,
                next_sibling,
                &raw_children,
            );
            let decision = session
                .environment_mut()
                .enter_section_children(request)
                .map_err(|message| SectionParseError::Environment { message })?;
            let (mut child_context, body_mode, metadata) = match decision {
                SectionChildrenDecision::Accept {
                    context,
                    body_mode,
                    metadata,
                } => (context, body_mode, metadata),
                SectionChildrenDecision::Reject {
                    reason,
                    diagnostics,
                } => {
                    return Ok(Some(Err(section_semantic_rejection(
                        &candidate,
                        reason,
                        diagnostics,
                    ))));
                }
            };
            candidate.body_mode = body_mode;
            candidate.metadata = metadata;
            child_context.section_stack = parent_context.section_stack.clone();
            let header_source = range
                .slice(session.source().virtual_source())
                .ok_or(SectionParseError::InvalidRange { range })?;
            child_context.section_stack.push(section_scope_frame(
                &candidate,
                &parent_context,
                header_source,
                session.source().virtual_source(),
            ));
            candidate.body_context = child_context.clone();
            let saved_context = session.replace_context(child_context);
            let body = parse_section_body(session, tree, node, body_mode, depth + 1);
            let child_context = session.replace_context(saved_context);
            let (body, child_diagnostics) = body?;
            candidate.body = body;
            if lifecycle == SectionRootLifecycle::RetainBody {
                candidate.body_context = child_context.clone();
                session.replace_context(child_context);
                return Ok(Some(Ok((candidate, child_diagnostics))));
            }
            let request = section_children_request(
                session.source().virtual_source(),
                &candidate,
                &child_context,
                preceding_siblings,
                next_sibling,
                &raw_children,
            );
            match session
                .environment_mut()
                .exit_section_children(request)
                .map_err(|message| SectionParseError::Environment { message })?
            {
                SectionExitDecision::Accept {
                    mut context,
                    metadata,
                } => {
                    // Exit hooks may publish ordinary context updates to siblings,
                    // but only the native parser owns control-flow ancestry.
                    context.section_stack = parent_context.section_stack;
                    session.replace_context(context);
                    candidate.metadata = metadata;
                    Ok(Some(Ok((candidate, child_diagnostics))))
                }
                SectionExitDecision::Reject {
                    reason,
                    diagnostics,
                } => Ok(Some(Err(section_semantic_rejection(
                    &candidate,
                    reason,
                    diagnostics,
                )))),
            }
        })();
        let accepted = attempt
            .as_ref()
            .is_ok_and(|attempt| matches!(attempt, Some(Ok((_candidate, _diagnostics)))));
        session
            .finish_semantic_candidate(accepted)
            .map_err(|message| SectionParseError::Environment { message })?;
        match attempt? {
            Some(Ok((candidate, child_diagnostics))) => {
                diagnostics.extend(child_diagnostics);
                return Ok(SectionMatches {
                    selected: Some(candidate),
                    alternatives: Vec::new(),
                    unknown: None,
                    diagnostics,
                });
            }
            Some(Err(rejected)) => {
                failure = crate::choose_failure_trace(failure, Some(rejected));
            }
            None => {}
        }
    }

    session.replace_context(initial_context);
    let (body, child_diagnostics) =
        parse_section_body(session, tree, node, SectionBodyMode::Trigger, depth + 1)?;
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
        alternatives: Vec::new(),
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

fn section_semantic_rejection(
    candidate: &SectionCandidate,
    reason: String,
    diagnostics: Vec<crate::SemanticDiagnostic>,
) -> FailureTrace {
    let candidate_span = candidate.matched.matched.span.clone();
    let span = crate::failure::semantic_failure_span(&candidate_span, &diagnostics);
    FailureTrace::leaf(PatternFailure {
        span: span.clone(),
        reasons: vec![PatternFailureReason::HookRejected { reason }],
    })
    .with_parent(FailureFrame {
        kind: candidate.matched.kind,
        definition_id: candidate.matched.definition_id.clone(),
        registration_id: candidate.matched.registration_id.clone(),
        pattern_index: candidate.matched.pattern_index,
        pattern: candidate.matched.pattern.clone(),
        element_path: Vec::new(),
        pattern_span: None,
        input_span: candidate_span,
        role: FailureFrameRole::SemanticCandidate,
    })
    .with_semantic_diagnostics(diagnostics)
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
    let addon = section
        .map(|section| section.common.addon.clone())
        .or_else(|| section_expression.map(|expression| expression.common.addon.clone()));
    let dynamic_handler = session.dynamic_handler_for_registration(&matched.registration_id);
    let capture_bindings = session
        .environment()
        .registered_capture_bindings(RegisteredSyntaxIdentity {
            kind: SyntaxKind::Section,
            definition_id: &matched.definition_id,
            registration_id: &matched.registration_id,
            pattern_index: Some(matched.pattern_index),
            pattern_source: Some(&matched.pattern),
            tags: Some(&matched.matched.tags),
            mark: Some(matched.matched.mark),
            dynamic_handler,
        })
        .map_err(|message| SectionParseError::Environment { message })?;
    let mut parsed_captures = Vec::new();
    for capture in &matched.matched.captures {
        let capture_index = capture.capture_index();
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
                let context = session
                    .capture_context(binding)
                    .map_err(|message| SectionParseError::Environment { message })?;
                let previous = context.map(|context| session.replace_context(context));
                let parsed =
                    crate::condition::parse_condition_with_session(session, range, depth + 1);
                if let Some(previous) = previous {
                    session.replace_context(previous);
                }
                let parsed = parsed?;
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
                let context = session
                    .capture_context(binding)
                    .map_err(|message| SectionParseError::Environment { message })?;
                let previous = context.map(|context| session.replace_context(context));
                let parsed = crate::effect::parse_effect_range_with_session(
                    session,
                    range,
                    raw_node_id,
                    depth + 1,
                );
                if let Some(previous) = previous {
                    session.replace_context(previous);
                }
                let parsed = parsed?;
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
    let addon_name = addon.as_ref().map(|addon| addon.name.clone());
    let addon_version = addon.map(|addon| addon.version);
    let owner_component_id = dynamic.map(|definition| definition.id.component_id.clone());
    Ok(Some(SectionCandidate {
        raw_node_id,
        matched,
        element_class,
        addon_name,
        addon_version,
        owner_component_id,
        parsed_captures,
        loop_section,
        effect_section,
        section_expression,
        body_mode: SectionBodyMode::Trigger,
        body: Vec::new(),
        handler: dynamic.map(|definition| definition.handler.clone()),
        metadata: dynamic.map_or_else(BTreeMap::new, |definition| definition.metadata.clone()),
        body_context: session.context().clone(),
    }))
}

fn section_scope_frame(
    candidate: &SectionCandidate,
    parent_context: &ExpressionParseContext,
    source: &str,
    complete_source: &str,
) -> SectionScopeFrame {
    let raw_scope_id = candidate.raw_node_id.get();
    let scope_id = if parent_context
        .section_stack
        .iter()
        .any(|frame| frame.scope_id == raw_scope_id)
    {
        parent_context
            .section_stack
            .iter()
            .map(|frame| frame.scope_id)
            .max()
            .unwrap_or(raw_scope_id)
            .saturating_add(1)
    } else {
        raw_scope_id
    };
    SectionScopeFrame {
        scope_id,
        parent_scope_id: parent_context
            .section_stack
            .last()
            .map(|frame| frame.scope_id),
        raw_node_id: candidate.raw_node_id,
        kind: if candidate.section_expression {
            SectionScopeKind::SectionExpression
        } else if candidate.loop_section {
            SectionScopeKind::Loop
        } else if candidate.effect_section {
            SectionScopeKind::EffectSection
        } else {
            SectionScopeKind::Section
        },
        definition_id: candidate.matched.definition_id.clone(),
        registration_id: candidate.matched.registration_id.clone(),
        element_class: candidate.element_class.clone(),
        pattern_index: candidate.matched.pattern_index,
        pattern: candidate.matched.pattern.clone(),
        source: source.to_owned(),
        addon_name: candidate.addon_name.clone(),
        addon_version: candidate.addon_version.clone(),
        owner_component_id: candidate.owner_component_id.clone(),
        loop_section: candidate.loop_section,
        effect_section: candidate.effect_section,
        section_expression: candidate.section_expression,
        captures: candidate
            .parsed_captures
            .iter()
            .map(|capture| section_scope_capture(capture, complete_source))
            .collect(),
        metadata: candidate.metadata.clone(),
    }
}

fn section_scope_capture(capture: &ParsedCapture, source: &str) -> SectionScopeCapture {
    let summary = capture.result.summary.as_ref();
    let local = capture.result.span.local_range;
    SectionScopeCapture {
        default_expression: summary.and_then(|summary| summary.default_expression.clone()),
        capture_index: capture.capture_index,
        parser_id: capture.result.parser_id.clone(),
        status: capture.result.status.clone(),
        source: local.slice(source).unwrap_or_default().to_owned(),
        kind: summary.map(|summary| summary.kind.clone()),
        definition_id: summary.and_then(|summary| summary.definition_id.clone()),
        registration_id: summary.and_then(|summary| summary.registration_id.clone()),
        element_class: summary.and_then(|summary| summary.element_class.clone()),
        pattern_index: summary.and_then(|summary| summary.pattern_index),
        return_type: summary.and_then(|summary| summary.return_type.clone()),
        possible_return_types: summary
            .map(|summary| summary.possible_return_types.clone())
            .unwrap_or_default(),
        possible_return_types_state: summary
            .map_or(PossibleReturnTypesState::Unresolved, |summary| {
                summary.possible_return_types_state
            }),
        multiplicity: summary.and_then(|summary| summary.multiplicity),
        public_data: summary
            .map(|summary| summary.public_data.clone())
            .unwrap_or_default(),
        metadata: summary
            .map(|summary| summary.metadata.clone())
            .unwrap_or_default(),
    }
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
    body_mode: SectionBodyMode,
    depth: usize,
) -> Result<(Vec<SectionBodyNode>, Vec<SectionDiagnostic>), SectionParseError> {
    let mut body = Vec::new();
    let mut diagnostics = Vec::new();
    let mut preceding_sections = Vec::new();
    for (child_index, id) in node.children.iter().enumerate() {
        let Some(child) = tree.get(*id) else {
            continue;
        };
        match child.kind {
            RawNodeKind::Blank | RawNodeKind::Comment => {
                body.push(SectionBodyNode::Trivia(child.id));
            }
            RawNodeKind::Simple => {
                preceding_sections.clear();
                let Some(code_span) = child.code_span.as_ref() else {
                    body.push(SectionBodyNode::Unclaimed(child.id));
                    diagnostics.push(section_diagnostic(
                        session,
                        child,
                        SectionDiagnosticKind::Unclaimed,
                    )?);
                    continue;
                };
                match body_mode {
                    SectionBodyMode::Trigger => {
                        let matches = crate::effect::parse_effect_range_with_session(
                            session,
                            code_span.virtual_range,
                            child.id,
                            depth,
                        )?;
                        extend_match_diagnostics(
                            session,
                            child,
                            matches.selected.is_some(),
                            !matches.alternatives.is_empty(),
                            &mut diagnostics,
                        )?;
                        body.push(SectionBodyNode::Effect(Box::new(matches)));
                    }
                    SectionBodyMode::Conditions => {
                        let matches = crate::condition::parse_condition_with_session(
                            session,
                            code_span.virtual_range,
                            depth,
                        )?;
                        extend_match_diagnostics(
                            session,
                            child,
                            matches.selected.is_some(),
                            !matches.alternatives.is_empty(),
                            &mut diagnostics,
                        )?;
                        body.push(SectionBodyNode::Condition {
                            raw_node_id: child.id,
                            matches: Box::new(matches),
                        });
                    }
                }
            }
            RawNodeKind::Section => {
                if body_mode == SectionBodyMode::Conditions {
                    preceding_sections.clear();
                    body.push(SectionBodyNode::Unclaimed(child.id));
                    diagnostics.push(section_diagnostic(
                        session,
                        child,
                        SectionDiagnosticKind::Unclaimed,
                    )?);
                    continue;
                }
                let next_sibling = node.children[child_index + 1..]
                    .iter()
                    .filter_map(|id| tree.get(*id))
                    .find(|node| !matches!(node.kind, RawNodeKind::Blank | RawNodeKind::Comment))
                    .map(|node| raw_node_summary(session, node))
                    .transpose()?;
                let matches = parse_section_with_session(
                    session,
                    tree,
                    child,
                    depth,
                    &preceding_sections,
                    next_sibling.as_ref(),
                    SectionRootLifecycle::Complete,
                )?;
                diagnostics.extend(matches.diagnostics.iter().cloned());
                if let Some(selected) = matches.selected.as_ref() {
                    preceding_sections.push(section_sibling_summary(session, selected)?);
                } else {
                    preceding_sections.clear();
                }
                body.push(SectionBodyNode::Section(Box::new(matches)));
            }
            RawNodeKind::Invalid => {
                preceding_sections.clear();
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

fn extend_match_diagnostics<E: ExpressionParseEnvironment>(
    session: &ExpressionSession<'_, E>,
    node: &RawNode,
    selected: bool,
    has_alternatives: bool,
    diagnostics: &mut Vec<SectionDiagnostic>,
) -> Result<(), SectionParseError> {
    let kind = if !selected {
        Some(SectionDiagnosticKind::Unclaimed)
    } else if has_alternatives {
        Some(SectionDiagnosticKind::MultipleClaims)
    } else {
        None
    };
    if let Some(kind) = kind {
        diagnostics.push(section_diagnostic(session, node, kind)?);
    }
    Ok(())
}

fn raw_child_summaries<E: ExpressionParseEnvironment>(
    session: &ExpressionSession<'_, E>,
    tree: &RawTree,
    parent: &RawNode,
) -> Result<Vec<SectionRawNodeSummary>, SectionParseError> {
    parent
        .children
        .iter()
        .filter_map(|id| tree.get(*id))
        .map(|node| raw_node_summary(session, node))
        .collect()
}

fn raw_node_summary<E: ExpressionParseEnvironment>(
    session: &ExpressionSession<'_, E>,
    node: &RawNode,
) -> Result<SectionRawNodeSummary, SectionParseError> {
    let range = node
        .code_span
        .as_ref()
        .map_or(node.span.virtual_range, |span| span.virtual_range);
    Ok(SectionRawNodeSummary {
        raw_node_id: node.id,
        kind: node.kind,
        source: range
            .slice(session.source().virtual_source())
            .ok_or(SectionParseError::InvalidRange { range })?
            .to_owned(),
        span: session.map_range(range)?,
    })
}

fn section_sibling_summary<E: ExpressionParseEnvironment>(
    session: &ExpressionSession<'_, E>,
    candidate: &SectionCandidate,
) -> Result<SectionSiblingSummary, SectionParseError> {
    let range = candidate.matched.matched.span.mapped.virtual_range;
    Ok(SectionSiblingSummary {
        raw_node_id: candidate.raw_node_id,
        definition_id: candidate.matched.definition_id.clone(),
        registration_id: candidate.matched.registration_id.clone(),
        element_class: candidate.element_class.clone(),
        pattern_index: candidate.matched.pattern_index,
        source: range
            .slice(session.source().virtual_source())
            .ok_or(SectionParseError::InvalidRange { range })?
            .to_owned(),
        span: candidate.matched.matched.span.clone(),
        handler: candidate.handler.clone(),
        metadata: candidate.metadata.clone(),
    })
}

fn section_children_request<'a>(
    input: &'a str,
    candidate: &'a SectionCandidate,
    context: &'a ExpressionParseContext,
    preceding_siblings: &'a [SectionSiblingSummary],
    next_sibling: Option<&'a SectionRawNodeSummary>,
    raw_children: &'a [SectionRawNodeSummary],
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
        tags: &candidate.matched.matched.tags,
        mark: candidate.matched.matched.mark,
        marks: &candidate.matched.matched.marks,
        parsed_captures: &candidate.parsed_captures,
        body_mode: candidate.body_mode,
        preceding_siblings,
        next_sibling,
        raw_children,
        metadata: &candidate.metadata,
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

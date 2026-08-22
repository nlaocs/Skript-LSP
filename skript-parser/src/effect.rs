//! Effect parsing over lossless RawTree nodes and SSG registrations.
//!
//! The parser keeps source provenance and all ranked alternatives while sharing
//! the recursive Expression session used by standalone Expression parsing.
#![allow(missing_docs)] // Aggregate contracts are documented on their owning types.

use crate::{
    CandidateFailure, CandidateMatch, CandidateMatches, ConditionParseError,
    ExpressionParseContext, ExpressionParseEnvironment, ExpressionParseError,
    ExpressionParserConfig, ExpressionSession, FailureFrame, FailureFrameRole, FailureTrace,
    HOST_CONDITION_PARSER_ID, HOST_EFFECT_PARSER_ID, MappedSource, MatchSpan, ParsedCapture,
    ParsedCaptureResult, ParsedCaptureStatus, ParsedCaptureValue, PatternCapture, PatternFailure,
    RankedFailures, RawNode, RawNodeId, RawNodeKind, RegisteredSyntaxIdentity, TextRange,
};
use std::collections::{BTreeMap, HashSet};
use syntaxes::{Catalog, DynamicSyntaxSnapshot, SyntaxKind};
use thiserror::Error;

/// Input required to parse one lossless Simple node as an Effect.
pub struct EffectParseRequest<'a> {
    pub source: &'a MappedSource,
    pub node: &'a RawNode,
    pub context: ExpressionParseContext,
}

/// Resource budgets shared by Effect matching and nested Expression parsing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectParserConfig {
    pub expression: ExpressionParserConfig,
}

/// One valid Effect candidate in deterministic registration order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectCandidate {
    pub raw_node_id: RawNodeId,
    pub matched: CandidateMatch,
    /// All recursively parsed captures in pattern order.
    pub parsed_captures: Vec<ParsedCapture>,
    /// Opaque handler name for dynamically registered Effects.
    pub handler: Option<String>,
    /// Addon-owned metadata attached to a dynamic registration.
    pub metadata: BTreeMap<String, String>,
}

impl EffectCandidate {
    /// Iterates typed Expression captures without exposing parser-specific storage.
    pub fn expressions(&self) -> impl Iterator<Item = &crate::ExpressionNode> {
        self.parsed_captures.iter().filter_map(|capture| {
            if let Some(ParsedCaptureValue::Expression(expression)) = capture.result.value.as_ref()
            {
                Some(expression)
            } else {
                None
            }
        })
    }

    /// Iterates Condition captures resolved by any registered parser binding.
    pub fn conditions(&self) -> impl Iterator<Item = &crate::ConditionNode> {
        self.parsed_captures.iter().filter_map(|capture| {
            if let Some(ParsedCaptureValue::Condition(condition)) = capture.result.value.as_ref() {
                Some(condition)
            } else {
                None
            }
        })
    }

    /// Iterates nested Effect captures resolved by any registered parser binding.
    pub fn effects(&self) -> impl Iterator<Item = &EffectCandidate> {
        self.parsed_captures.iter().filter_map(|capture| {
            if let Some(ParsedCaptureValue::Effect(effect)) = capture.result.value.as_ref() {
                Some(effect.as_ref())
            } else {
                None
            }
        })
    }
}

/// A Simple node that no registered Effect accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownEffectNode {
    pub raw_node_id: RawNodeId,
    pub source: String,
    pub span: MatchSpan,
    /// Ranked rejected registrations and the aggregate matcher fallback.
    pub failures: RankedFailures<EffectCandidateFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Effect registration that matched a meaningful prefix but failed semantically.
pub struct EffectCandidateFailure {
    pub matched: CandidateFailure,
    pub element_class: Option<syntaxes::ClassName>,
    pub handler: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

/// Selected Effect, later alternatives, or a source-preserving unknown node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectMatches {
    pub selected: Option<EffectCandidate>,
    pub alternatives: Vec<EffectCandidate>,
    pub unknown: Option<UnknownEffectNode>,
}

/// Failure while validating a RawTree node or parsing its Effect contents.
#[derive(Debug, Error)]
pub enum EffectParseError {
    #[error("Effect parsing requires a Simple RawTree node, got {actual:?}")]
    UnsupportedNodeKind { actual: RawNodeKind },
    #[error("Simple RawTree node {node_id} has no code span")]
    MissingCodeSpan { node_id: RawNodeId },
    #[error("Effect syntax context {node_context} does not match parser context {parser_context}")]
    SyntaxContextMismatch {
        node_context: u64,
        parser_context: u64,
    },
    #[error("Effect code range {range} is invalid for the mapped source")]
    InvalidCodeRange { range: TextRange },
    #[error("failed to map Effect code: {message}")]
    SourceMap { message: String },
    #[error(transparent)]
    Expression(#[from] ExpressionParseError),
    #[error(transparent)]
    Condition(#[from] ConditionParseError),
}

/// Parses one Simple node with static Effect registrations.
///
/// # Examples
///
/// ```no_run
/// use skript_parser::{
///     EffectParseRequest, EffectParserConfig, ExpressionParseContext, MappedSource,
///     NoopExpressionEnvironment, RawTreeOptions, parse_effect, parse_raw_tree,
/// };
/// use syntaxes::Catalog;
///
/// fn parse_line(catalog: &Catalog) -> Result<(), Box<dyn std::error::Error>> {
///     let source = MappedSource::identity("broadcast \"hello\"");
///     let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
///     let node = tree.get(tree.roots[0]).expect("one source line");
///     let result = parse_effect(
///         catalog,
///         EffectParseRequest {
///             source: &source,
///             node,
///             context: ExpressionParseContext::default(),
///         },
///         &mut NoopExpressionEnvironment,
///         EffectParserConfig::default(),
///     )?;
///     assert!(result.selected.is_some() || result.unknown.is_some());
///     Ok(())
/// }
/// ```
///
/// # Errors
///
/// Returns [EffectParseError] when the node is not Simple, its source/context
/// does not match the request, or nested pattern/Expression parsing fails.
pub fn parse_effect<E: ExpressionParseEnvironment>(
    catalog: &Catalog,
    request: EffectParseRequest<'_>,
    environment: &mut E,
    config: EffectParserConfig,
) -> Result<EffectMatches, EffectParseError> {
    parse_effect_with_snapshot(catalog, None, request, environment, config)
}

/// Parses one Simple node with static and frozen dynamic Effect registrations.
///
/// The complete code span must match one candidate. `%type%` captures recurse
/// through the same Expression session, so nested candidates share memoization,
/// resource limits, hook ordering, and transactional selection.
pub fn parse_effect_with_snapshot<E: ExpressionParseEnvironment>(
    catalog: &Catalog,
    dynamic_snapshot: Option<&DynamicSyntaxSnapshot>,
    request: EffectParseRequest<'_>,
    environment: &mut E,
    config: EffectParserConfig,
) -> Result<EffectMatches, EffectParseError> {
    if request.node.kind != RawNodeKind::Simple {
        return Err(EffectParseError::UnsupportedNodeKind {
            actual: request.node.kind,
        });
    }
    let node_context = u64::from(request.node.syntax_context.get());
    if node_context != request.context.syntax_context {
        return Err(EffectParseError::SyntaxContextMismatch {
            node_context,
            parser_context: request.context.syntax_context,
        });
    }
    let code_span = request
        .node
        .code_span
        .as_ref()
        .ok_or(EffectParseError::MissingCodeSpan {
            node_id: request.node.id,
        })?;
    let range = code_span.virtual_range;
    if !range.is_valid_for(request.source.virtual_source()) {
        return Err(EffectParseError::InvalidCodeRange { range });
    }

    let mut session = ExpressionSession::new(
        catalog,
        dynamic_snapshot,
        request.source,
        environment,
        request.context,
        config.expression,
    );
    parse_effect_range_with_session(&mut session, range, request.node.id, 0)
}

enum EffectCandidateResolution {
    Accepted(EffectCandidate),
    Rejected(EffectCandidateFailure),
}

pub(crate) fn parse_effect_range_with_session<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    range: TextRange,
    raw_node_id: RawNodeId,
    depth: usize,
) -> Result<EffectMatches, EffectParseError> {
    session.ensure_depth(depth)?;
    let mut candidates = session.syntax_candidates(SyntaxKind::Effect);
    session.retain_viable_patterns(range, &mut candidates)?;
    let matches = session.match_candidates_at_depth(range, &candidates, depth)?;
    let CandidateMatches {
        selected: matched_selected,
        alternatives: matched_alternatives,
        failures: mut matcher_failures,
    } = matches;
    let mut ranked = matched_selected
        .into_iter()
        .chain(matched_alternatives)
        .collect::<Vec<_>>();
    let mut accepted = Vec::new();
    let mut candidate_failures = Vec::new();
    for matched in ranked.drain(..) {
        let resolved_order = candidates
            .iter()
            .find(|candidate| candidate.registration_id == matched.registration_id)
            .and_then(|candidate| candidate.resolved_order);
        session
            .begin_semantic_candidate()
            .map_err(|message| ExpressionParseError::Environment { message })?;
        let candidate = effect_candidate(
            raw_node_id,
            matched,
            resolved_order,
            session,
            range.start,
            depth,
        );
        let keep = candidate.as_ref().is_ok_and(|candidate| {
            matches!(candidate, EffectCandidateResolution::Accepted(_)) && accepted.is_empty()
        });
        session
            .finish_semantic_candidate(keep)
            .map_err(|message| ExpressionParseError::Environment { message })?;
        match candidate? {
            EffectCandidateResolution::Accepted(candidate) => accepted.push(candidate),
            EffectCandidateResolution::Rejected(candidate) => {
                candidate_failures.push(candidate);
            }
        }
    }
    let selected = (!accepted.is_empty()).then(|| accepted.remove(0));
    let alternatives = accepted;
    if selected.is_none()
        && let Some(primary) = matcher_failures.primary().cloned()
        && let Some(mut candidate) = candidates
            .iter()
            .find(|candidate| candidate.registration_id == primary.registration_id)
            .cloned()
    {
        if let Some(pattern_index) = primary.pattern_index {
            candidate
                .patterns
                .retain(|pattern| pattern.pattern_index == pattern_index);
        }
        if let Some(recovered) =
            session.recover_candidate_failures_at_depth(range, &[candidate], depth)?
            && let Some(target) = matcher_failures.candidates.iter_mut().find(|candidate| {
                candidate.registration_id == recovered.registration_id
                    && candidate.pattern_index == recovered.pattern_index
            })
        {
            for trace in std::iter::once(recovered.trace).chain(recovered.related) {
                if trace != target.trace && !target.related.contains(&trace) {
                    target.related.push(trace);
                }
            }
            target.related.sort_by_key(|trace| {
                let range = trace.root_cause().failure.span.mapped.virtual_range;
                (range.start, range.end)
            });
        }
    }
    let unknown = if selected.is_none() {
        let source = range
            .slice(session.source().virtual_source())
            .ok_or(EffectParseError::InvalidCodeRange { range })?
            .to_owned();
        let mapped =
            session
                .source()
                .map_range(range)
                .map_err(|error| EffectParseError::SourceMap {
                    message: error.to_string(),
                })?;
        candidate_failures.extend(
            matcher_failures
                .candidates
                .into_iter()
                .map(|matched| effect_candidate_failure(session, matched)),
        );
        candidate_failures.sort_by_key(|candidate| {
            let trace = &candidate.matched.trace;
            let root = trace.root_cause();
            let range = root.failure.span.mapped.virtual_range;
            (
                std::cmp::Reverse(trace.specificity()),
                std::cmp::Reverse(range.end),
                std::cmp::Reverse(range.end.saturating_sub(range.start)),
                std::cmp::Reverse(candidate.matched.literal_anchor),
                candidate.matched.resolved_order.is_none(),
                candidate.matched.resolved_order.unwrap_or(usize::MAX),
                candidate.matched.priority,
                candidate.matched.registration_order,
            )
        });
        let mut seen = HashSet::new();
        candidate_failures.retain(|candidate| {
            seen.insert((
                candidate.matched.registration_id.clone(),
                candidate.matched.pattern_index,
            ))
        });
        Some(UnknownEffectNode {
            raw_node_id,
            source,
            span: MatchSpan {
                local_range: range,
                mapped,
            },
            failures: RankedFailures {
                fallback: matcher_failures.fallback,
                candidates: candidate_failures,
            },
        })
    } else {
        None
    };

    Ok(EffectMatches {
        selected,
        alternatives,
        unknown,
    })
}

fn effect_candidate_failure<E: ExpressionParseEnvironment>(
    session: &ExpressionSession<'_, E>,
    matched: CandidateFailure,
) -> EffectCandidateFailure {
    let dynamic = session.dynamic_snapshot().and_then(|snapshot| {
        snapshot
            .definitions
            .values()
            .find(|definition| definition.id.qualified() == matched.registration_id)
    });
    let element_class = session
        .catalog()
        .effects()
        .find(|effect| effect.common.registration_id.as_str() == matched.registration_id)
        .map(|effect| effect.common.element_class.clone());
    EffectCandidateFailure {
        matched,
        element_class,
        handler: dynamic.map(|definition| definition.handler.clone()),
        metadata: dynamic.map_or_else(BTreeMap::new, |definition| definition.metadata.clone()),
    }
}

pub(crate) fn effect_semantic_summary(
    candidate: &EffectCandidate,
    catalog: &Catalog,
) -> crate::ParsedCaptureSemanticSummary {
    crate::ParsedCaptureSemanticSummary {
        kind: "effect".to_owned(),
        definition_id: Some(candidate.matched.definition_id.clone()),
        registration_id: Some(candidate.matched.registration_id.clone()),
        element_class: catalog
            .effects()
            .find(|effect| {
                effect.common.registration_id.as_str() == candidate.matched.registration_id
            })
            .map(|effect| effect.common.element_class.clone()),
        pattern_index: Some(candidate.matched.pattern_index),
        return_type: None,
        multiplicity: None,
        metadata: candidate.metadata.clone(),
    }
}

fn effect_candidate<E: ExpressionParseEnvironment>(
    raw_node_id: RawNodeId,
    matched: CandidateMatch,
    resolved_order: Option<usize>,
    session: &mut ExpressionSession<'_, E>,
    frame_start: usize,
    depth: usize,
) -> Result<EffectCandidateResolution, EffectParseError> {
    let dynamic = session.dynamic_snapshot().and_then(|snapshot| {
        snapshot
            .definitions
            .values()
            .find(|definition| definition.id.qualified() == matched.registration_id)
    });
    let element_class = session
        .catalog()
        .effects()
        .find(|effect| effect.common.registration_id.as_str() == matched.registration_id)
        .map(|effect| effect.common.element_class.clone());
    let handler = dynamic.map(|definition| definition.handler.clone());
    let metadata = dynamic.map_or_else(BTreeMap::new, |definition| definition.metadata.clone());
    let capture_bindings = session
        .environment()
        .registered_capture_bindings(RegisteredSyntaxIdentity {
            kind: SyntaxKind::Effect,
            definition_id: &matched.definition_id,
            registration_id: &matched.registration_id,
            pattern_index: Some(matched.pattern_index),
        })
        .map_err(|message| {
            EffectParseError::Expression(ExpressionParseError::Environment { message })
        })?;
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
        let PatternCapture::Regex {
            pattern_span, span, ..
        } = capture
        else {
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
                    None => {
                        let cause = parsed
                            .unknown
                            .map(condition_failure_trace)
                            .unwrap_or_else(|| nested_failure_trace(span.clone()));
                        let trace = cause.with_parent(semantic_failure_frame(
                            &matched,
                            *pattern_span,
                            span.clone(),
                            FailureFrameRole::ConditionCapture {
                                index: capture_index,
                            },
                        ));
                        if binding.required {
                            return Ok(EffectCandidateResolution::Rejected(
                                semantic_effect_candidate_failure(
                                    &matched,
                                    resolved_order,
                                    trace,
                                    element_class.clone(),
                                    handler.clone(),
                                    metadata.clone(),
                                ),
                            ));
                        }
                        ParsedCapture {
                            capture_index,
                            binding: binding.clone(),
                            result: ParsedCaptureResult::failure(
                                binding.parser_id.clone(),
                                span.clone(),
                                "condition capture did not match",
                            ),
                        }
                    }
                }
            }
            HOST_EFFECT_PARSER_ID => {
                let parsed =
                    parse_effect_range_with_session(session, range, raw_node_id, depth + 1)?;
                match parsed.selected {
                    Some(selected) => ParsedCapture {
                        capture_index,
                        binding: binding.clone(),
                        result: ParsedCaptureResult::success(
                            binding.parser_id.clone(),
                            span.clone(),
                            Some(effect_semantic_summary(&selected, session.catalog())),
                            ParsedCaptureValue::Effect(Box::new(selected)),
                        ),
                    },
                    None => {
                        let cause = parsed
                            .unknown
                            .map(effect_failure_trace)
                            .unwrap_or_else(|| nested_failure_trace(span.clone()));
                        let trace = cause.with_parent(semantic_failure_frame(
                            &matched,
                            *pattern_span,
                            span.clone(),
                            FailureFrameRole::EffectCapture {
                                index: capture_index,
                            },
                        ));
                        if binding.required {
                            return Ok(EffectCandidateResolution::Rejected(
                                semantic_effect_candidate_failure(
                                    &matched,
                                    resolved_order,
                                    trace,
                                    element_class.clone(),
                                    handler.clone(),
                                    metadata.clone(),
                                ),
                            ));
                        }
                        ParsedCapture {
                            capture_index,
                            binding: binding.clone(),
                            result: ParsedCaptureResult::failure(
                                binding.parser_id.clone(),
                                span.clone(),
                                "effect capture did not match",
                            ),
                        }
                    }
                }
            }
            _ => {
                let result = ParsedCaptureResult::failure(
                    binding.parser_id.clone(),
                    span.clone(),
                    format!("no native parser route for {}", binding.parser_id),
                );
                if binding.required {
                    let trace =
                        nested_failure_trace(span.clone()).with_parent(semantic_failure_frame(
                            &matched,
                            *pattern_span,
                            span.clone(),
                            FailureFrameRole::EffectCapture {
                                index: capture_index,
                            },
                        ));
                    return Ok(EffectCandidateResolution::Rejected(
                        semantic_effect_candidate_failure(
                            &matched,
                            resolved_order,
                            trace,
                            element_class.clone(),
                            handler.clone(),
                            metadata.clone(),
                        ),
                    ));
                }
                ParsedCapture {
                    capture_index,
                    binding: binding.clone(),
                    result,
                }
            }
        };
        if parsed.result.status != ParsedCaptureStatus::Failed || !binding.required {
            parsed_captures.push(parsed);
        }
    }
    Ok(EffectCandidateResolution::Accepted(EffectCandidate {
        raw_node_id,
        matched,
        parsed_captures,
        handler,
        metadata,
    }))
}

fn semantic_effect_candidate_failure(
    matched: &CandidateMatch,
    resolved_order: Option<usize>,
    trace: FailureTrace,
    element_class: Option<syntaxes::ClassName>,
    handler: Option<String>,
    metadata: BTreeMap<String, String>,
) -> EffectCandidateFailure {
    EffectCandidateFailure {
        matched: CandidateFailure {
            kind: matched.kind,
            definition_id: matched.definition_id.clone(),
            registration_id: matched.registration_id.clone(),
            priority: matched.priority,
            registration_order: matched.registration_order,
            resolved_order,
            literal_anchor: matched.literal_anchor,
            pattern_index: Some(matched.pattern_index),
            pattern: Some(matched.pattern.clone()),
            trace,
            related: Vec::new(),
        },
        element_class,
        handler,
        metadata,
    }
}

fn semantic_failure_frame(
    matched: &CandidateMatch,
    pattern_span: syntax_pattern_parser::syntax::Span,
    input_span: MatchSpan,
    role: FailureFrameRole,
) -> FailureFrame {
    FailureFrame {
        kind: matched.kind,
        definition_id: matched.definition_id.clone(),
        registration_id: matched.registration_id.clone(),
        pattern_index: matched.pattern_index,
        pattern: matched.pattern.clone(),
        element_path: Vec::new(),
        pattern_span: Some(pattern_span),
        input_span,
        role,
    }
}

fn condition_failure_trace(unknown: crate::UnknownCondition) -> FailureTrace {
    unknown
        .failure
        .unwrap_or_else(|| nested_failure_trace(unknown.span))
}

fn effect_failure_trace(unknown: UnknownEffectNode) -> FailureTrace {
    let RankedFailures {
        fallback,
        candidates,
    } = unknown.failures;
    candidates
        .into_iter()
        .next()
        .map(|candidate| candidate.matched.trace)
        .or(fallback)
        .unwrap_or_else(|| nested_failure_trace(unknown.span))
}

pub(crate) fn nested_failure_trace(span: MatchSpan) -> FailureTrace {
    FailureTrace::leaf(PatternFailure {
        span,
        reasons: vec![crate::PatternFailureReason::TrailingInput],
    })
}

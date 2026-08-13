//! Effect parsing over lossless RawTree nodes and SSG registrations.
//!
//! The parser keeps source provenance and all ranked alternatives while sharing
//! the recursive Expression session used by standalone Expression parsing.
#![allow(missing_docs)] // Aggregate contracts are documented on their owning types.

use crate::{
    CandidateFailure, CandidateMatch, ConditionParseError, ExpressionNode, ExpressionParseContext,
    ExpressionParseEnvironment, ExpressionParseError, ExpressionParserConfig, ExpressionSession,
    MappedSource, MatchSpan, PatternCapture, PatternFailure, RawNode, RawNodeId, RawNodeKind,
    RegisteredCaptureKind, RegisteredConditionCapture, TextRange,
};
use std::collections::BTreeMap;
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
    pub expressions: Vec<ExpressionNode>,
    pub conditions: Vec<RegisteredConditionCapture>,
    pub effects: Vec<RegisteredEffectCapture>,
    /// Opaque handler name for dynamically registered Effects.
    pub handler: Option<String>,
    /// Addon-owned metadata attached to a dynamic registration.
    pub metadata: BTreeMap<String, String>,
}

/// A Simple node that no registered Effect accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownEffectNode {
    pub raw_node_id: RawNodeId,
    pub source: String,
    pub span: MatchSpan,
    pub failure: Option<PatternFailure>,
    /// Registration that matched a meaningful prefix but failed in one of its captures.
    pub best_candidate: Option<EffectCandidateFailure>,
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

/// One regex capture that was recursively accepted as an Effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredEffectCapture {
    pub capture_index: usize,
    pub value: String,
    pub span: MatchSpan,
    pub candidate: Box<EffectCandidate>,
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
    let mut ranked = matches
        .selected
        .into_iter()
        .chain(matches.alternatives)
        .collect::<Vec<_>>();
    let mut accepted = Vec::new();
    let mut semantic_best_failure = None;
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
                semantic_best_failure.get_or_insert(candidate);
            }
        }
    }
    let selected = (!accepted.is_empty()).then(|| accepted.remove(0));
    let alternatives = accepted;
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
        let best_candidate = semantic_best_failure.or_else(|| {
            matches
                .best_failure
                .map(|matched| effect_candidate_failure(session, matched))
        });
        let failure = best_candidate
            .as_ref()
            .map(|candidate| candidate.matched.failure.clone())
            .or(matches.failure);
        Some(UnknownEffectNode {
            raw_node_id,
            source,
            span: MatchSpan {
                local_range: range,
                mapped,
            },
            failure,
            best_candidate,
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

fn effect_candidate<E: ExpressionParseEnvironment>(
    raw_node_id: RawNodeId,
    matched: CandidateMatch,
    resolved_order: Option<usize>,
    session: &mut ExpressionSession<'_, E>,
    frame_start: usize,
    depth: usize,
) -> Result<EffectCandidateResolution, EffectParseError> {
    let expressions = matched
        .matched
        .captures
        .iter()
        .filter_map(|capture| match capture {
            PatternCapture::TypeExpression {
                resolution_id: Some(id),
                ..
            } => session.resolved_node(id).cloned(),
            PatternCapture::Regex { .. }
            | PatternCapture::TypeExpression {
                resolution_id: None,
                ..
            } => None,
        })
        .collect();
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
    let capture_kinds = element_class
        .as_ref()
        .map_or_else(Vec::new, |element_class| {
            session
                .environment()
                .registered_capture_kinds(SyntaxKind::Effect, element_class)
        });
    let mut conditions = Vec::new();
    let mut effects = Vec::new();
    for (capture_index, (capture, kind)) in matched
        .matched
        .captures
        .iter()
        .filter(|capture| matches!(capture, PatternCapture::Regex { .. }))
        .zip(capture_kinds)
        .enumerate()
    {
        let PatternCapture::Regex { value, span, .. } = capture else {
            unreachable!("regex captures were filtered")
        };
        let local = span.local_range;
        let range = TextRange::new(frame_start + local.start, frame_start + local.end);
        match kind {
            RegisteredCaptureKind::Raw => {}
            RegisteredCaptureKind::Condition => {
                let parsed =
                    crate::condition::parse_condition_with_session(session, range, depth + 1)?;
                let Some(selected) = parsed.selected else {
                    let failure = parsed
                        .unknown
                        .map(|unknown| {
                            unknown.failure.map_or_else(
                                || nested_failure_from_span(unknown.span, frame_start),
                                |failure| rebase_nested_failure(failure, range.start - frame_start),
                            )
                        })
                        .unwrap_or_else(|| nested_failure_from_capture(span.clone()));
                    return Ok(EffectCandidateResolution::Rejected(
                        semantic_effect_candidate_failure(
                            &matched,
                            resolved_order,
                            failure,
                            element_class.clone(),
                            handler.clone(),
                            metadata.clone(),
                        ),
                    ));
                };
                conditions.push(RegisteredConditionCapture {
                    capture_index,
                    node: selected.node,
                });
            }
            RegisteredCaptureKind::Effect => {
                let parsed =
                    parse_effect_range_with_session(session, range, raw_node_id, depth + 1)?;
                let Some(selected) = parsed.selected else {
                    let failure = parsed
                        .unknown
                        .map(|unknown| {
                            unknown
                                .best_candidate
                                .map(|candidate| candidate.matched.failure)
                                .or(unknown.failure)
                                .map_or_else(
                                    || nested_failure_from_span(unknown.span, frame_start),
                                    |failure| {
                                        rebase_nested_failure(failure, range.start - frame_start)
                                    },
                                )
                        })
                        .unwrap_or_else(|| nested_failure_from_capture(span.clone()));
                    return Ok(EffectCandidateResolution::Rejected(
                        semantic_effect_candidate_failure(
                            &matched,
                            resolved_order,
                            failure,
                            element_class.clone(),
                            handler.clone(),
                            metadata.clone(),
                        ),
                    ));
                };
                effects.push(RegisteredEffectCapture {
                    capture_index,
                    value: value.clone(),
                    span: span.clone(),
                    candidate: Box::new(selected),
                });
            }
        }
    }
    Ok(EffectCandidateResolution::Accepted(EffectCandidate {
        raw_node_id,
        matched,
        expressions,
        conditions,
        effects,
        handler,
        metadata,
    }))
}

fn semantic_effect_candidate_failure(
    matched: &CandidateMatch,
    resolved_order: Option<usize>,
    failure: PatternFailure,
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
            failure,
        },
        element_class,
        handler,
        metadata,
    }
}

fn rebase_nested_failure(mut failure: PatternFailure, offset: usize) -> PatternFailure {
    failure.offset = failure.offset.saturating_add(offset);
    failure.span.local_range = TextRange::new(
        failure.span.local_range.start.saturating_add(offset),
        failure.span.local_range.end.saturating_add(offset),
    );
    failure
}

fn nested_failure_from_span(mut span: MatchSpan, frame_start: usize) -> PatternFailure {
    span.local_range = TextRange::new(
        span.local_range.start.saturating_sub(frame_start),
        span.local_range.end.saturating_sub(frame_start),
    );
    PatternFailure {
        offset: span.local_range.start,
        span,
        reasons: vec![crate::PatternFailureReason::TrailingInput],
    }
}

fn nested_failure_from_capture(span: MatchSpan) -> PatternFailure {
    PatternFailure {
        offset: span.local_range.start,
        span,
        reasons: vec![crate::PatternFailureReason::TrailingInput],
    }
}

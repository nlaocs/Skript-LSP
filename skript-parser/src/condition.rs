//! Condition parsing over SSG registrations and recursive Expressions.
#![allow(missing_docs)] // Aggregate contracts are documented on their owning types.

use crate::pattern_match::{find_parenthesis_end, java_trim_range};
use crate::{
    CandidateMatch, ExpressionNode, ExpressionParseContext, ExpressionParseEnvironment,
    ExpressionParseError, ExpressionParserConfig, ExpressionSession, FailureFrame,
    FailureFrameRole, FailureTrace, MappedSource, MatchSpan, MatchSyntaxKind, ParseMarkCapture,
    ParseTagCapture, PatternCapture, PatternFailure, PatternFailureReason, TextRange,
};
use std::collections::BTreeMap;
use syntaxes::{Catalog, DynamicSyntaxSnapshot, SyntaxKind};
use thiserror::Error;

/// Input required to parse one complete Condition.
pub struct ConditionParseRequest<'a> {
    pub source: &'a MappedSource,
    pub range: TextRange,
    pub context: ExpressionParseContext,
}

/// Resource budgets shared by Condition matching and nested Expressions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConditionParserConfig {
    pub expression: ExpressionParserConfig,
}

/// Source of a Condition node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionNodeKind {
    /// A complete child Condition wrapped in one pair of parentheses.
    Grouped,
    /// One registered SSG or dynamic Condition pattern.
    Registered {
        definition_id: String,
        registration_id: String,
        pattern_index: usize,
        pattern: String,
        priority: i32,
        registration_order: usize,
    },
}

/// One parsed Condition with its nested Expression and grouping structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionNode {
    pub kind: ConditionNodeKind,
    pub span: MatchSpan,
    pub captures: Vec<PatternCapture>,
    pub tags: Vec<ParseTagCapture>,
    pub mark: i32,
    pub marks: Vec<ParseMarkCapture>,
    pub expressions: Vec<ExpressionNode>,
    pub children: Vec<ConditionNode>,
    pub handler: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

/// One valid Condition candidate in deterministic registration order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionCandidate {
    pub node: ConditionNode,
}

/// Fully parsed Condition candidate offered to CoreLibrary and addon semantics.
pub struct ConditionSemanticRequest<'a> {
    pub input: &'a str,
    pub context: &'a ExpressionParseContext,
    pub candidate: &'a ConditionCandidate,
}

/// Semantic decision returned after a Condition matched structurally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionSemanticDecision {
    UseCandidate,
    Accepted {
        context: ExpressionParseContext,
        handler: Option<String>,
        metadata: BTreeMap<String, String>,
    },
    Reject {
        reason: String,
        diagnostics: Vec<crate::SemanticDiagnostic>,
    },
}

/// Source-preserving information for a Condition that did not match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownCondition {
    pub source: String,
    pub span: MatchSpan,
    pub failure: Option<FailureTrace>,
}

/// Selected Condition, later alternatives, or a source-preserving unknown value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionMatches {
    pub selected: Option<ConditionCandidate>,
    pub alternatives: Vec<ConditionCandidate>,
    pub unknown: Option<UnknownCondition>,
}

/// Failure while validating Condition input or parsing nested Expressions.
#[derive(Debug, Error)]
pub enum ConditionParseError {
    #[error("Condition range {range} is invalid for the mapped source")]
    InvalidInputRange { range: TextRange },
    #[error(transparent)]
    Expression(#[from] ExpressionParseError),
}

/// Parses one complete Condition with static SSG registrations.
pub fn parse_condition<E: ExpressionParseEnvironment>(
    catalog: &Catalog,
    request: ConditionParseRequest<'_>,
    environment: &mut E,
    config: ConditionParserConfig,
) -> Result<ConditionMatches, ConditionParseError> {
    parse_condition_with_snapshot(catalog, None, request, environment, config)
}

/// Parses one complete Condition with static and frozen dynamic registrations.
pub fn parse_condition_with_snapshot<E: ExpressionParseEnvironment>(
    catalog: &Catalog,
    dynamic_snapshot: Option<&DynamicSyntaxSnapshot>,
    request: ConditionParseRequest<'_>,
    environment: &mut E,
    config: ConditionParserConfig,
) -> Result<ConditionMatches, ConditionParseError> {
    if !request.range.is_valid_for(request.source.virtual_source()) {
        return Err(ConditionParseError::InvalidInputRange {
            range: request.range,
        });
    }
    let mut session = ExpressionSession::new(
        catalog,
        dynamic_snapshot,
        request.source,
        environment,
        request.context,
        config.expression,
    );
    parse_condition_with_session(&mut session, request.range, 0)
}

pub(crate) fn parse_condition_with_session<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    range: TextRange,
    depth: usize,
) -> Result<ConditionMatches, ConditionParseError> {
    session.ensure_depth(depth)?;
    if !range.is_valid_for(session.source().virtual_source()) {
        return Err(ConditionParseError::InvalidInputRange { range });
    }
    let text = range
        .slice(session.source().virtual_source())
        .expect("Condition range was validated");
    let local_trimmed = java_trim_range(text);
    let trimmed = TextRange::new(
        range.start + local_trimmed.start,
        range.start + local_trimmed.end,
    );
    if trimmed.is_empty() {
        return unknown_condition(session, trimmed, None);
    }

    if trimmed
        .slice(session.source().virtual_source())
        .is_some_and(|value| value.starts_with('('))
        && let Some(close) = find_parenthesis_end(
            session.source().virtual_source(),
            trimmed.start + '('.len_utf8(),
            trimmed.end,
        )
        && close + ')'.len_utf8() == trimmed.end
    {
        let inner = TextRange::new(trimmed.start + '('.len_utf8(), close);
        let mut matches = parse_condition_with_session(session, inner, depth + 1)?;
        let group_span = session.map_range(trimmed)?;
        matches.selected = matches
            .selected
            .map(|candidate| grouped_candidate(candidate, group_span.clone()));
        matches.alternatives = matches
            .alternatives
            .into_iter()
            .map(|candidate| grouped_candidate(candidate, group_span.clone()))
            .collect();
        if matches.selected.is_some() {
            matches.unknown = None;
        }
        return Ok(matches);
    }

    let mut candidates = session.syntax_candidates(SyntaxKind::Condition);
    session.retain_viable_patterns(trimmed, &mut candidates)?;
    let matched = session.match_candidates_at_depth(trimmed, &candidates, depth)?;
    let mut failure = matched.primary_failure().cloned();
    let mut accepted = Vec::new();
    let initial_context = session.context().clone();
    let mut selected_context = None;
    for matched in matched.selected.into_iter().chain(matched.alternatives) {
        session.replace_context(initial_context.clone());
        if let Some(restricted) =
            session.event_restriction_failure(&matched.registration_id, session.map_range(trimmed)?)
        {
            failure = crate::choose_failure_trace(failure, Some(restricted));
            continue;
        }
        session
            .begin_semantic_candidate()
            .map_err(|message| ExpressionParseError::Environment { message })?;
        let mut candidate = condition_candidate(session, matched, trimmed)?;
        let mut accepted_semantically = true;
        let input = trimmed
            .slice(session.source().virtual_source())
            .ok_or(ConditionParseError::InvalidInputRange { range: trimmed })?;
        let context = session.context().clone();
        match session
            .environment_mut()
            .resolve_condition_candidate(ConditionSemanticRequest {
                input,
                context: &context,
                candidate: &candidate,
            })
            .map_err(|message| ExpressionParseError::Environment { message })?
        {
            ConditionSemanticDecision::UseCandidate => {}
            ConditionSemanticDecision::Accepted {
                context,
                handler,
                metadata,
            } => {
                session.replace_context(context);
                candidate.node.handler = handler;
                candidate.node.metadata = metadata;
            }
            ConditionSemanticDecision::Reject {
                reason,
                diagnostics,
            } => {
                accepted_semantically = false;
                let candidate_span = candidate.node.span.clone();
                let span = crate::failure::semantic_failure_span(&candidate_span, &diagnostics);
                let mut trace = FailureTrace::leaf(PatternFailure {
                    span: span.clone(),
                    reasons: vec![PatternFailureReason::HookRejected { reason }],
                });
                if let ConditionNodeKind::Registered {
                    definition_id,
                    registration_id,
                    pattern_index,
                    pattern,
                    ..
                } = &candidate.node.kind
                {
                    trace = trace.with_parent(FailureFrame {
                        kind: MatchSyntaxKind::Condition,
                        definition_id: definition_id.clone(),
                        registration_id: registration_id.clone(),
                        pattern_index: *pattern_index,
                        pattern: pattern.clone(),
                        element_path: Vec::new(),
                        pattern_span: None,
                        input_span: candidate_span,
                        role: FailureFrameRole::SemanticCandidate,
                    });
                }
                failure = crate::choose_failure_trace(
                    failure,
                    Some(trace.with_semantic_diagnostics(diagnostics)),
                );
            }
        }
        let keep = accepted_semantically && accepted.is_empty();
        if keep {
            selected_context = Some(session.context().clone());
        }
        session
            .finish_semantic_candidate(keep)
            .map_err(|message| ExpressionParseError::Environment { message })?;
        if accepted_semantically {
            accepted.push(candidate);
            break;
        }
    }
    session.replace_context(selected_context.unwrap_or(initial_context));
    let selected = (!accepted.is_empty()).then(|| accepted.remove(0));
    let alternatives = accepted;
    if selected.is_none() {
        unknown_condition(session, trimmed, failure)
    } else {
        Ok(ConditionMatches {
            selected,
            alternatives,
            unknown: None,
        })
    }
}

fn grouped_candidate(candidate: ConditionCandidate, span: MatchSpan) -> ConditionCandidate {
    ConditionCandidate {
        node: ConditionNode {
            kind: ConditionNodeKind::Grouped,
            span,
            captures: Vec::new(),
            tags: Vec::new(),
            mark: 0,
            marks: Vec::new(),
            expressions: Vec::new(),
            children: vec![candidate.node],
            handler: None,
            metadata: BTreeMap::new(),
        },
    }
}

fn condition_candidate<E: ExpressionParseEnvironment>(
    session: &ExpressionSession<'_, E>,
    matched: CandidateMatch,
    range: TextRange,
) -> Result<ConditionCandidate, ConditionParseError> {
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
    Ok(ConditionCandidate {
        node: ConditionNode {
            kind: ConditionNodeKind::Registered {
                definition_id: matched.definition_id,
                registration_id: matched.registration_id,
                pattern_index: matched.pattern_index,
                pattern: matched.pattern,
                priority: matched.priority,
                registration_order: matched.registration_order,
            },
            span: session.map_range(range)?,
            captures: matched.matched.captures,
            tags: matched.matched.tags,
            mark: matched.matched.mark,
            marks: matched.matched.marks,
            expressions,
            children: Vec::new(),
            handler: dynamic.map(|definition| definition.handler.clone()),
            metadata: dynamic.map_or_else(BTreeMap::new, |definition| definition.metadata.clone()),
        },
    })
}

fn unknown_condition<E: ExpressionParseEnvironment>(
    session: &ExpressionSession<'_, E>,
    range: TextRange,
    failure: Option<FailureTrace>,
) -> Result<ConditionMatches, ConditionParseError> {
    let source = range
        .slice(session.source().virtual_source())
        .ok_or(ConditionParseError::InvalidInputRange { range })?
        .to_owned();
    Ok(ConditionMatches {
        selected: None,
        alternatives: Vec::new(),
        unknown: Some(UnknownCondition {
            source,
            span: session.map_range(range)?,
            failure,
        }),
    })
}

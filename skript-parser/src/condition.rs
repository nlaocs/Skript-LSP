//! Condition parsing over SSG registrations and recursive Expressions.
#![allow(missing_docs)] // Aggregate contracts are documented on their owning types.

#[cfg(test)]
mod capture_bindings_tests;

use crate::pattern_match::{find_parenthesis_end, java_trim_range};
use crate::expression::{
    capture_context, capture_expression_expected_types, capture_expression_flags,
    capture_expression_time_state,
};
use crate::{
    CandidateMatch, ExpressionNode, ExpressionParseContext, ExpressionParseEnvironment,
    ExpressionParseError, ExpressionParserConfig, ExpressionSession, FailureFrame,
    FailureFrameRole, FailureTrace, MappedSource, MatchSpan, MatchSyntaxKind, ParseMarkCapture,
    ParseTagCapture, ParsedCaptureRef, PatternCapture, PatternFailure, PatternFailureReason,
    RegisteredSyntaxIdentity, TextRange, HOST_EXPRESSION_PARSER_ID,
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

/// Association retained at the native resolution-ID lookup, never reconstructed
/// from expression order or coincident source spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConditionExpressionBinding {
    pub capture_index: usize,
    pub expression_index: usize,
    pub origin: ConditionExpressionBindingOrigin,
}

/// How the native parser selected an associated expression. A coincident regex
/// span alone never establishes a binding to an expression parsed elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionExpressionBindingOrigin {
    TypeResolution,
    NativeRegex,
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
    pub expression_bindings: Vec<ConditionExpressionBinding>,
    pub children: Vec<ConditionNode>,
    pub handler: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

impl ConditionNode {
    /// A unique native capture association, or no claim for malformed or
    /// unbound children. Regex slots require explicit native selection origin.
    pub fn expression_capture_index(&self, expression_index: usize) -> Option<usize> {
        self.expressions.get(expression_index)?;
        let mut bindings = self
            .expression_bindings
            .iter()
            .filter(|binding| binding.expression_index == expression_index);
        let binding = bindings.next()?;
        if bindings.next().is_some()
            || self
                .expression_bindings
                .iter()
                .filter(|other| other.capture_index == binding.capture_index)
                .count()
                != 1
        {
            return None;
        }
        let mut captures = self
            .captures
            .iter()
            .filter(|capture| capture.capture_index() == binding.capture_index);
        if !matches!(
            (captures.next()?, binding.origin),
            (PatternCapture::TypeExpression { resolution_id: Some(_), .. },
                ConditionExpressionBindingOrigin::TypeResolution)
                | (PatternCapture::Regex { .. }, ConditionExpressionBindingOrigin::NativeRegex)
        ) || captures.next().is_some()
        {
            return None;
        }
        Some(binding.capture_index)
    }

    /// The same borrowed capture contract used by Effects and Sections.
    pub fn parsed_capture_refs(&self) -> impl Iterator<Item = ParsedCaptureRef<'_>> {
        self.expressions
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                self.expression_capture_index(index).map(|capture_index| {
                    ParsedCaptureRef::Expression {
                        capture_index,
                        node,
                    }
                })
            })
    }
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
#[derive(Debug, Clone, Error)]
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
        session.check_cancelled()?;
        session.replace_context(initial_context.clone());
        if let Some(restricted) =
            session.event_restriction_failure(&matched.registration_id, session.map_range(trimmed)?)
        {
            failure = crate::choose_failure_trace(failure, Some(restricted));
            continue;
        }
        session
            .begin_semantic_candidate()
            .map_err(|message| session.environment_error(message))?;
        let attempt: Result<_, ConditionParseError> = (|| {
            session
                .activate_pattern_candidate(&matched)
                .map_err(|message| session.environment_error(message))?;
            let mut candidate = match condition_candidate(session, matched, trimmed, depth)? {
                ConditionCandidateResolution::Accepted(candidate) => candidate,
                ConditionCandidateResolution::Rejected(trace) => {
                    failure = crate::choose_failure_trace(failure.take(), Some(trace));
                    return Ok(None);
                }
            };
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
                .map_err(|message| session.environment_error(message))?
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
                        failure.take(),
                        Some(trace.with_semantic_diagnostics(diagnostics)),
                    );
                }
            }
            Ok(accepted_semantically.then_some(candidate))
        })();
        let keep = attempt
            .as_ref()
            .is_ok_and(|candidate| candidate.is_some() && accepted.is_empty());
        if keep {
            selected_context = Some(session.context().clone());
        }
        session
            .finish_semantic_candidate(keep)
            .map_err(|message| session.environment_error(message))?;
        if let Some(candidate) = attempt? {
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
            expression_bindings: Vec::new(),
            children: vec![candidate.node],
            handler: None,
            metadata: BTreeMap::new(),
        },
    }
}

enum ConditionCandidateResolution {
    Accepted(ConditionCandidate),
    Rejected(FailureTrace),
}

fn condition_candidate<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    matched: CandidateMatch,
    range: TextRange,
    depth: usize,
) -> Result<ConditionCandidateResolution, ConditionParseError> {
    let (mut expressions, mut expression_bindings) =
        resolved_condition_expressions(&matched.matched.captures, |id| session.resolved_node(id));
    let dynamic = session.dynamic_snapshot().and_then(|snapshot| {
        snapshot
            .definitions
            .values()
            .find(|definition| definition.id.qualified() == matched.registration_id)
    });
    let handler = dynamic.map(|definition| definition.handler.clone());
    let metadata = dynamic.map_or_else(BTreeMap::new, |definition| definition.metadata.clone());
    let bindings = session.environment().registered_capture_bindings(RegisteredSyntaxIdentity {
        kind: SyntaxKind::Condition,
        definition_id: &matched.definition_id,
        registration_id: &matched.registration_id,
        pattern_index: Some(matched.pattern_index),
        pattern_source: Some(&matched.pattern),
        tags: Some(&matched.matched.tags),
        mark: Some(matched.matched.mark),
        dynamic_handler: handler.as_deref(),
    }).map_err(|message| session.environment_error(message))?;
    for capture in &matched.matched.captures {
        let PatternCapture::Regex { capture_index, pattern_span, span, .. } = capture else { continue; };
        let Some(binding) = bindings.iter().find(|binding| binding.capture_index == *capture_index) else { continue; };
        let rejected = |cause: Option<FailureTrace>| {
            cause.unwrap_or_else(|| crate::effect::nested_failure_trace(span.clone()))
                .with_parent(FailureFrame {
                    kind: MatchSyntaxKind::Condition,
                    definition_id: matched.definition_id.clone(),
                    registration_id: matched.registration_id.clone(),
                    pattern_index: matched.pattern_index,
                    pattern: matched.pattern.clone(),
                    element_path: Vec::new(),
                    pattern_span: Some(*pattern_span),
                    input_span: span.clone(),
                    role: FailureFrameRole::ExpressionCapture { index: *capture_index },
                })
        };
        if binding.parser_id != HOST_EXPRESSION_PARSER_ID {
            if binding.required { return Ok(ConditionCandidateResolution::Rejected(rejected(None))); }
            continue;
        }
        let input = TextRange::new(range.start + span.local_range.start, range.start + span.local_range.end);
        let context = capture_context(session.context(), binding, &expressions)
            .map_err(|message| session.environment_error(message))?;
        let expected = capture_expression_expected_types(binding)
            .map_err(|message| session.environment_error(message))?;
        let time = capture_expression_time_state(binding, 0)
            .map_err(|message| session.environment_error(message))?;
        let (allow_literals, allow_expressions) = capture_expression_flags(binding)
            .map_err(|message| session.environment_error(message))?;
        let previous = context.map(|context| session.replace_context(context));
        let parsed = session.parse_capture_expression_prefixes(binding, input, &expected,
            allow_literals, allow_expressions, time, depth + 1);
        if let Some(previous) = previous { session.replace_context(previous); }
        let parsed = parsed?;
        let cause = parsed.failure;
        let mut candidates = parsed.candidates.into_iter()
            .filter(|candidate| candidate.node.span.local_range.end == input.end);
        let Some(candidate) = candidates.next() else {
            if binding.required { return Ok(ConditionCandidateResolution::Rejected(rejected(cause))); }
            continue;
        };
        session.select_expression(&candidate.node)?;
        session.check_cancelled()?;
        expression_bindings.push(ConditionExpressionBinding {
            capture_index: *capture_index,
            expression_index: expressions.len(),
            origin: ConditionExpressionBindingOrigin::NativeRegex,
        });
        expressions.push(candidate.node);
    }
    Ok(ConditionCandidateResolution::Accepted(ConditionCandidate {
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
            expression_bindings,
            children: Vec::new(),
            handler,
            metadata,
        },
    }))
}

fn resolved_condition_expressions<'a>(
    captures: &[PatternCapture],
    mut resolve: impl FnMut(&str) -> Option<&'a ExpressionNode>,
) -> (Vec<ExpressionNode>, Vec<ConditionExpressionBinding>) {
    let mut expressions = Vec::new();
    let mut bindings = Vec::new();
    for capture in captures {
        if let PatternCapture::TypeExpression {
            capture_index,
            resolution_id: Some(id),
            ..
        } = capture
            && let Some(node) = resolve(id)
        {
            bindings.push(ConditionExpressionBinding {
                capture_index: *capture_index,
                expression_index: expressions.len(),
                origin: ConditionExpressionBindingOrigin::TypeResolution,
            });
            expressions.push(node.clone());
        }
    }
    (expressions, bindings)
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

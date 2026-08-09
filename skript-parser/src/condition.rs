//! Condition parsing over SSG registrations and recursive Expressions.
#![allow(missing_docs)] // Aggregate contracts are documented on their owning types.

use crate::pattern_match::{find_parenthesis_end, java_trim_range};
use crate::{
    CandidateMatch, ExpressionNode, ExpressionParseContext, ExpressionParseEnvironment,
    ExpressionParseError, ExpressionParserConfig, ExpressionSession, MappedSource, MatchSpan,
    ParseMarkCapture, ParseTagCapture, PatternCapture, PatternFailure, TextRange,
    catalog_pattern_candidates, snapshot_pattern_candidates,
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

/// Source-preserving information for a Condition that did not match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownCondition {
    pub source: String,
    pub span: MatchSpan,
    pub failure: Option<PatternFailure>,
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

    let candidates = if let Some(snapshot) = session.dynamic_snapshot() {
        snapshot_pattern_candidates(session.catalog(), snapshot, SyntaxKind::Condition)
    } else {
        catalog_pattern_candidates(session.catalog(), SyntaxKind::Condition)
    };
    let matched = session.match_candidates_at_depth(trimmed, &candidates, depth)?;
    let selected = matched
        .selected
        .map(|value| condition_candidate(session, value, trimmed))
        .transpose()?;
    let alternatives = matched
        .alternatives
        .into_iter()
        .map(|value| condition_candidate(session, value, trimmed))
        .collect::<Result<Vec<_>, _>>()?;
    if selected.is_none() {
        unknown_condition(session, trimmed, matched.failure)
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
    failure: Option<PatternFailure>,
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

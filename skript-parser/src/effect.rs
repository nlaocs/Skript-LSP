//! Effect parsing over lossless RawTree nodes and SSG registrations.
//!
//! The parser keeps source provenance and all ranked alternatives while sharing
//! the recursive Expression session used by standalone Expression parsing.
#![allow(missing_docs)] // Aggregate contracts are documented on their owning types.

use crate::{
    CandidateMatch, ExpressionNode, ExpressionParseContext, ExpressionParseEnvironment,
    ExpressionParseError, ExpressionParserConfig, ExpressionSession, MappedSource, MatchSpan,
    PatternCapture, PatternFailure, RawNode, RawNodeId, RawNodeKind, TextRange,
    catalog_pattern_candidates, snapshot_pattern_candidates,
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

    let candidates = if let Some(snapshot) = dynamic_snapshot {
        snapshot_pattern_candidates(catalog, snapshot, SyntaxKind::Effect)
    } else {
        catalog_pattern_candidates(catalog, SyntaxKind::Effect)
    };
    let mut session = ExpressionSession::new(
        catalog,
        dynamic_snapshot,
        request.source,
        environment,
        request.context,
        config.expression,
    );
    let matches = session.match_candidates(range, &candidates)?;
    let selected = matches
        .selected
        .map(|matched| effect_candidate(request.node.id, matched, dynamic_snapshot, &session));
    let alternatives = matches
        .alternatives
        .into_iter()
        .map(|matched| effect_candidate(request.node.id, matched, dynamic_snapshot, &session))
        .collect();
    let unknown = if selected.is_none() {
        let source = range
            .slice(request.source.virtual_source())
            .ok_or(EffectParseError::InvalidCodeRange { range })?
            .to_owned();
        let mapped =
            request
                .source
                .map_range(range)
                .map_err(|error| EffectParseError::SourceMap {
                    message: error.to_string(),
                })?;
        Some(UnknownEffectNode {
            raw_node_id: request.node.id,
            source,
            span: MatchSpan {
                local_range: range,
                mapped,
            },
            failure: matches.failure,
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

fn effect_candidate<E: ExpressionParseEnvironment>(
    raw_node_id: RawNodeId,
    matched: CandidateMatch,
    dynamic_snapshot: Option<&DynamicSyntaxSnapshot>,
    session: &ExpressionSession<'_, E>,
) -> EffectCandidate {
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
    let dynamic = dynamic_snapshot.and_then(|snapshot| {
        snapshot
            .definitions
            .values()
            .find(|definition| definition.id.qualified() == matched.registration_id)
    });
    EffectCandidate {
        raw_node_id,
        matched,
        expressions,
        handler: dynamic.map(|definition| definition.handler.clone()),
        metadata: dynamic.map_or_else(BTreeMap::new, |definition| definition.metadata.clone()),
    }
}

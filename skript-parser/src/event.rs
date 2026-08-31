//! Native matching of SSG Event registrations.
//!
//! This module deliberately stops at the registration boundary.  It identifies
//! the Event syntax and preserves its captures, but it does not decide what an
//! event means or create an event-specific context.  A later `StructEvent` hook
//! can consume the selected candidate and bind its captures to `host.event`.
use crate::{
    CandidateFailure, CandidateMatch, CandidateMatches, ExpressionParseContext,
    ExpressionParseEnvironment, ExpressionParseError, ExpressionParserConfig, ExpressionSession,
    FailureTrace, MappedSource, MatchSpan, PatternCapture, RankedFailures, TextRange,
};
use std::collections::BTreeMap;
use syntaxes::{Catalog, ClassName, DynamicSyntaxSnapshot, SyntaxKind};
use thiserror::Error;

/// Input required to match one complete Event header.
pub struct EventParseRequest<'a> {
    /// Mapped virtual source containing the Event text.
    pub source: &'a MappedSource,
    /// Exact byte range to match as an Event.
    pub range: TextRange,
    /// Parser context inherited from the surrounding Structure.
    pub context: ExpressionParseContext,
}

/// Resource budgets shared by Event matching and future capture parsing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventParserConfig {
    /// Shared recursion, candidate, and matcher limits.
    pub expression: ExpressionParserConfig,
}

/// One Event registration accepted by the native matcher.
///
/// `matched` is retained rather than reduced to a boolean so a WASM
/// `StructEvent` handler can inspect the original registration identity,
/// pattern index, registration order, regex captures, and parse marks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventCandidate {
    /// Full registered-pattern result and SSG identities.
    pub matched: CandidateMatch,
    /// Source-mapped span of the complete Event input.
    pub span: MatchSpan,
    /// Java implementation class for a static registration.
    pub element_class: Option<ClassName>,
    /// Bukkit event classes whose event values are visible in the body.
    pub reference_events: Vec<ClassName>,
    /// Whether the registration represents a cancellable event.
    /// Dynamic registrations leave this unresolved unless their addon publishes metadata.
    pub cancellable: Option<bool>,
    /// Whether this Event accepts an explicit priority modifier.
    pub priority_supported: Option<bool>,
    /// Opaque handler selected by a dynamic registration.
    pub handler: Option<String>,
    /// Dynamic registration metadata retained for addon consumers.
    pub metadata: BTreeMap<String, String>,
}

impl EventCandidate {
    /// Returns captures in the order in which the Event pattern declared them.
    pub fn captures(&self) -> &[PatternCapture] {
        &self.matched.matched.captures
    }

    /// Returns the catalog/declaration registration order used as a tie-breaker.
    pub const fn registration_order(&self) -> usize {
        self.matched.registration_order
    }
}

/// Source-preserving information for an Event range accepted by no candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownEvent {
    /// Exact source text that no Event registration accepted.
    pub source: String,
    /// Source-mapped span of `source`.
    pub span: MatchSpan,
    /// Farthest failure retained for diagnostics.
    pub failure: Option<FailureTrace>,
}

/// One candidate-specific failure retained for Event diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventCandidateFailure {
    /// Registered candidate and its farthest mismatch.
    pub matched: CandidateFailure,
    /// Opaque handler selected by a dynamic registration.
    pub handler: Option<String>,
    /// Dynamic registration metadata retained for diagnostics.
    pub metadata: BTreeMap<String, String>,
}

/// Selected Event, later alternatives, and a source-preserving failed result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventMatches {
    /// Highest-ranked complete Event match.
    pub selected: Option<EventCandidate>,
    /// Other complete matches in parser order.
    pub alternatives: Vec<EventCandidate>,
    /// Source-preserving recovery node when no candidate matched.
    pub unknown: Option<UnknownEvent>,
    /// Candidate-specific failures are retained for richer StructEvent errors.
    pub failures: RankedFailures<EventCandidateFailure>,
}

/// Failure while validating Event input or its mapped source range.
#[derive(Debug, Error)]
pub enum EventParseError {
    /// The requested byte range is outside the virtual source or splits UTF-8.
    #[error("Event range {range} is invalid for the mapped source")]
    InvalidInputRange { range: TextRange },
    /// Recursive pattern or Expression matching failed.
    #[error(transparent)]
    Expression(#[from] ExpressionParseError),
}

/// Matches one Event range against static catalog registrations.
pub fn parse_event<E: ExpressionParseEnvironment>(
    catalog: &Catalog,
    request: EventParseRequest<'_>,
    environment: &mut E,
    config: EventParserConfig,
) -> Result<EventMatches, EventParseError> {
    parse_event_with_snapshot(catalog, None, request, environment, config)
}

/// Matches one Event range against static and frozen dynamic registrations.
pub fn parse_event_with_snapshot<E: ExpressionParseEnvironment>(
    catalog: &Catalog,
    dynamic_snapshot: Option<&DynamicSyntaxSnapshot>,
    request: EventParseRequest<'_>,
    environment: &mut E,
    config: EventParserConfig,
) -> Result<EventMatches, EventParseError> {
    if !request.range.is_valid_for(request.source.virtual_source()) {
        return Err(EventParseError::InvalidInputRange {
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
    parse_event_range_with_session(&mut session, request.range, 0)
}

/// Matches one Event range inside an existing expression session.
///
/// This is the entry point intended for `StructEvent`: the surrounding parser
/// can reuse its source map, dynamic snapshot, hook environment, and resource
/// budget instead of creating a second parse session.
pub(crate) fn parse_event_range_with_session<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    range: TextRange,
    depth: usize,
) -> Result<EventMatches, EventParseError> {
    session.ensure_depth(depth)?;
    if !range.is_valid_for(session.source().virtual_source()) {
        return Err(EventParseError::InvalidInputRange { range });
    }

    let mut candidates = session.syntax_candidates(SyntaxKind::Event);
    session.retain_viable_patterns(range, &mut candidates)?;
    let matches = session.match_candidates_at_depth(range, &candidates, depth)?;
    let CandidateMatches {
        selected,
        alternatives,
        failures,
    } = matches;
    let fallback = failures.fallback.clone();
    let primary_failure = failures
        .primary()
        .map(|failure| &failure.trace)
        .or(fallback.as_ref());

    let selected = selected
        .map(|matched| event_candidate(session, matched, range))
        .transpose()?;
    let alternatives = alternatives
        .into_iter()
        .map(|matched| event_candidate(session, matched, range))
        .collect::<Result<Vec<_>, _>>()?;
    let unknown = if selected.is_none() {
        Some(unknown_event(session, range, primary_failure)?)
    } else {
        None
    };
    let failures = failures
        .candidates
        .into_iter()
        .map(|failure| event_failure(session, failure))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(EventMatches {
        selected,
        alternatives,
        unknown,
        failures: RankedFailures {
            candidates: failures,
            fallback,
        },
    })
}

fn event_candidate<E: ExpressionParseEnvironment>(
    session: &ExpressionSession<'_, E>,
    matched: CandidateMatch,
    range: TextRange,
) -> Result<EventCandidate, EventParseError> {
    let dynamic = session.dynamic_snapshot().and_then(|snapshot| {
        snapshot
            .definitions
            .values()
            .find(|definition| definition.id.qualified() == matched.registration_id)
    });
    let event = session
        .catalog()
        .events()
        .find(|event| event.common.registration_id.as_str() == matched.registration_id);
    let element_class = event.map(|event| event.common.element_class.clone());
    let reference_events = event.map_or_else(Vec::new, |event| event.reference_events.clone());
    let cancellable = event.map(|event| event.cancellable);
    let priority_supported = event.and_then(|event| event.priority_supported);
    Ok(EventCandidate {
        span: session.map_range(range)?,
        matched,
        element_class,
        reference_events,
        cancellable,
        priority_supported,
        handler: dynamic.map(|definition| definition.handler.clone()),
        metadata: dynamic.map_or_else(BTreeMap::new, |definition| definition.metadata.clone()),
    })
}

fn event_failure<E: ExpressionParseEnvironment>(
    session: &ExpressionSession<'_, E>,
    matched: CandidateFailure,
) -> Result<EventCandidateFailure, EventParseError> {
    let dynamic = session.dynamic_snapshot().and_then(|snapshot| {
        snapshot
            .definitions
            .values()
            .find(|definition| definition.id.qualified() == matched.registration_id)
    });
    Ok(EventCandidateFailure {
        matched,
        handler: dynamic.map(|definition| definition.handler.clone()),
        metadata: dynamic.map_or_else(BTreeMap::new, |definition| definition.metadata.clone()),
    })
}

fn unknown_event<E: ExpressionParseEnvironment>(
    session: &ExpressionSession<'_, E>,
    range: TextRange,
    failure: Option<&FailureTrace>,
) -> Result<UnknownEvent, EventParseError> {
    let source = range
        .slice(session.source().virtual_source())
        .ok_or(EventParseError::InvalidInputRange { range })?
        .to_owned();
    Ok(UnknownEvent {
        source,
        span: session.map_range(range)?,
        failure: failure.cloned(),
    })
}

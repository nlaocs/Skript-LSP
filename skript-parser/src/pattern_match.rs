//! Backtracking matcher for parsed Skript registration patterns.
//!
//! Matching supports typed resolver and hook extension points, bounded resource use,
//! deterministic candidate ranking, captures, and farthest-failure diagnostics.
#![allow(missing_docs)] // Type-level docs describe aggregate field contracts.

use crate::{MappedSource, MappedSpan, TextRange};
use fancy_regex::{Error as FancyRegexError, Regex, RegexBuilder, RuntimeError};
use std::collections::{BTreeSet, HashMap, HashSet};
use syntax_pattern_parser::syntax::{
    ParseResult, PatternElement, PatternTypeExpr, Span as PatternSpan, SpannedPatternElement,
};

#[derive(Debug, Clone)]
enum MatchInputMapping<'a> {
    Source {
        source: &'a MappedSource,
        source_range: TextRange,
    },
    Fixed(MappedSpan),
}

/// Pattern input with both node-local byte offsets and editor-facing provenance.
#[derive(Debug, Clone)]
pub struct MatchInput<'a> {
    text: &'a str,
    mapping: MatchInputMapping<'a>,
}

impl<'a> MatchInput<'a> {
    /// Borrows a valid virtual-source range and preserves its composed mapping.
    pub fn from_source(
        source: &'a MappedSource,
        range: TextRange,
    ) -> Result<Self, PatternMatchError> {
        let text = range
            .slice(source.virtual_source())
            .ok_or(PatternMatchError::InvalidInputRange { range })?;
        Ok(Self {
            text,
            mapping: MatchInputMapping::Source {
                source,
                source_range: range,
            },
        })
    }

    /// Creates matcher input whose ranges map to a fixed generated call site.
    pub fn generated(text: &'a str, call_site: MappedSpan) -> Self {
        Self {
            text,
            mapping: MatchInputMapping::Fixed(call_site),
        }
    }

    /// Returns the node-local text presented to the matcher.
    pub const fn text(&self) -> &'a str {
        self.text
    }

    /// Converts a node-local range into editor-facing provenance.
    pub fn map_range(&self, local_range: TextRange) -> Result<MatchSpan, PatternMatchError> {
        if !local_range.is_valid_for(self.text) {
            return Err(PatternMatchError::InvalidInputRange { range: local_range });
        }
        let mapped = match &self.mapping {
            MatchInputMapping::Source {
                source,
                source_range,
            } => source
                .map_range(TextRange::new(
                    source_range.start + local_range.start,
                    source_range.start + local_range.end,
                ))
                .map_err(|error| PatternMatchError::SourceMap {
                    message: error.to_string(),
                })?,
            MatchInputMapping::Fixed(call_site) => call_site.clone(),
        };
        Ok(MatchSpan {
            local_range,
            mapped,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Matched local range paired with mapped original-source provenance.
pub struct MatchSpan {
    pub local_range: TextRange,
    pub mapped: MappedSpan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Syntax category order used by candidate ranking and hooks.
pub enum MatchSyntaxKind {
    Event,
    Condition,
    Effect,
    Expression,
    Type,
    Function,
    Section,
    Structure,
}

#[derive(Debug, Clone, Copy)]
/// Borrowed source and parsed AST for one registration pattern.
pub struct MatchPattern<'a> {
    pub source: &'a str,
    pub parsed: &'a ParseResult,
}

#[derive(Debug, Clone)]
/// One definition/registration and all patterns eligible for matching.
pub struct PatternCandidate<'a> {
    pub kind: MatchSyntaxKind,
    pub definition_id: String,
    pub registration_id: String,
    /// Lower values run first, matching the syntax priority model.
    pub priority: i32,
    pub registration_order: usize,
    /// Resolved order after before/after constraints, when supplied by a registry.
    pub resolved_order: Option<usize>,
    pub patterns: Vec<MatchPattern<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One prefix accepted by recursive typed-expression parsing.
pub struct TypeExpressionResolution {
    pub range: TextRange,
    pub alternative_index: Option<usize>,
    pub resolution_id: Option<String>,
}

/// Context supplied to the recursive typed-expression resolver.
pub struct TypeExpressionRequest<'a> {
    pub input: &'a str,
    pub expression: &'a PatternTypeExpr,
    pub pattern_span: PatternSpan,
    pub remaining: TextRange,
    /// Legal Skript split points in runtime traversal order.
    pub candidate_ends: &'a [usize],
}

/// Extension point that resolves `%type%` placeholders at legal split points.
pub trait TypeExpressionResolver {
    fn resolve(
        &mut self,
        request: TypeExpressionRequest<'_>,
    ) -> Result<Vec<TypeExpressionResolution>, String>;
}

#[derive(Debug, Default)]
/// Resolver that rejects every typed placeholder without failing the matcher.
pub struct RejectTypeExpressions;

impl TypeExpressionResolver for RejectTypeExpressions {
    fn resolve(
        &mut self,
        _request: TypeExpressionRequest<'_>,
    ) -> Result<Vec<TypeExpressionResolution>, String> {
        Ok(Vec::new())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Matcher hierarchy at which a WASM or native hook is invoked.
pub enum PatternHookScope {
    Definition,
    Registration,
    Pattern,
    Element,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Whether a hook runs before or after native matching for its scope.
pub enum PatternHookTiming {
    Before,
    After,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Native or overridden result visible to an after-hook.
pub enum PatternHookOutcome {
    Pending,
    Matched { range: TextRange },
    Failed { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable route to a nested element or choice branch in the pattern AST.
pub enum PatternPathSegment {
    Element(u32),
    Branch(u32),
}

/// Complete typed context delivered for one matcher hook invocation.
pub struct PatternHookEvent<'a> {
    pub kind: MatchSyntaxKind,
    pub definition_id: &'a str,
    pub registration_id: &'a str,
    pub pattern_index: Option<usize>,
    pub pattern: Option<&'a str>,
    pub element_path: &'a [PatternPathSegment],
    pub pattern_span: Option<PatternSpan>,
    pub scope: PatternHookScope,
    pub timing: PatternHookTiming,
    pub input_range: TextRange,
    pub input_span: MatchSpan,
    pub outcome: PatternHookOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Decision returned by a matcher hook.
pub enum PatternHookControl {
    Continue,
    Match(TextRange),
    Fail(String),
}

/// Observer/override interface spanning definition through element scopes.
pub trait PatternMatchHooks {
    fn dispatch(&mut self, event: PatternHookEvent<'_>) -> Result<PatternHookControl, String>;
}

#[derive(Debug, Default)]
/// Hook implementation that always continues native matching.
pub struct NoopPatternMatchHooks;

impl PatternMatchHooks for NoopPatternMatchHooks {
    fn dispatch(&mut self, _event: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
        Ok(PatternHookControl::Continue)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Resource counter that may terminate bounded backtracking.
pub enum PatternMatchLimit {
    States,
    Backtracks,
    RegexExecutions,
    RegexEvaluatedBytes,
    RegexBacktracks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Hard limits applied to one matcher invocation.
pub struct PatternMatcherConfig {
    pub max_states: usize,
    pub max_backtracks: usize,
    pub max_regex_executions: usize,
    pub max_regex_evaluated_bytes: usize,
    pub max_regex_backtracks: usize,
}

impl Default for PatternMatcherConfig {
    fn default() -> Self {
        Self {
            max_states: 100_000,
            max_backtracks: 50_000,
            max_regex_executions: 10_000,
            max_regex_evaluated_bytes: 8 * 1024 * 1024,
            max_regex_backtracks: 1_000_000,
        }
    }
}

impl PatternMatcherConfig {
    fn validate(&self) -> Result<(), PatternMatchError> {
        if self.max_states == 0
            || self.max_backtracks == 0
            || self.max_regex_executions == 0
            || self.max_regex_evaluated_bytes == 0
            || self.max_regex_backtracks == 0
        {
            Err(PatternMatchError::InvalidConfiguration)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
/// Configuration, extension, regex, mapping, or resource failure.
pub enum PatternMatchError {
    #[error("pattern matcher limits must be greater than zero")]
    InvalidConfiguration,
    #[error("input range {range} is not valid for the matcher input")]
    InvalidInputRange { range: TextRange },
    #[error("source mapping failed: {message}")]
    SourceMap { message: String },
    #[error("invalid regular expression at pattern bytes {pattern_span:?}: {message}")]
    InvalidRegex {
        pattern_span: PatternSpan,
        message: String,
    },
    #[error("type expression resolver failed at pattern bytes {pattern_span:?}: {message}")]
    TypeResolver {
        pattern_span: PatternSpan,
        message: String,
    },
    #[error("pattern hook failed: {message}")]
    Hook { message: String },
    #[error("pattern matcher exceeded the {kind:?} limit of {limit}")]
    LimitExceeded {
        kind: PatternMatchLimit,
        limit: usize,
    },
    #[error("type expression resolver returned invalid range {range}")]
    InvalidTypeResolution { range: TextRange },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
/// Expected construct recorded at the farthest failed input offset.
pub enum PatternFailureReason {
    Literal { expected: String },
    Regex { pattern: String },
    TypeExpression { expected: Vec<String> },
    TrailingInput,
    HookRejected { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Merged farthest-failure diagnostic for all unsuccessful candidates.
pub struct PatternFailure {
    pub offset: usize,
    pub span: MatchSpan,
    pub reasons: Vec<PatternFailureReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One numbered regex group and its optional matched span.
pub struct RegexGroupCapture {
    pub index: usize,
    pub value: Option<String>,
    pub span: Option<MatchSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Typed value captured by a regex or `%type%` element.
pub enum PatternCapture {
    Regex {
        pattern_span: PatternSpan,
        value: String,
        span: MatchSpan,
        groups: Vec<RegexGroupCapture>,
    },
    TypeExpression {
        pattern_span: PatternSpan,
        expression: PatternTypeExpr,
        value: String,
        span: MatchSpan,
        alternative_index: Option<usize>,
        resolution_id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Parse tag value and the input position where it became active.
pub struct ParseTagCapture {
    pub value: String,
    pub pattern_span: PatternSpan,
    pub input_span: MatchSpan,
    pub implicit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One parse mark and its accumulated XOR value.
pub struct ParseMarkCapture {
    pub value: i32,
    pub pattern_span: PatternSpan,
    pub input_span: MatchSpan,
    pub accumulated: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Complete match result for one registration pattern.
pub struct PatternMatch {
    pub span: MatchSpan,
    pub captures: Vec<PatternCapture>,
    pub tags: Vec<ParseTagCapture>,
    pub mark: i32,
    pub marks: Vec<ParseMarkCapture>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Successful syntax candidate with identity, ranking, and captures.
pub struct CandidateMatch {
    pub kind: MatchSyntaxKind,
    pub definition_id: String,
    pub registration_id: String,
    pub priority: i32,
    pub registration_order: usize,
    pub pattern_index: usize,
    pub pattern: String,
    pub matched: PatternMatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Selected candidate, later alternatives, or a farthest failure.
pub struct CandidateMatches {
    pub selected: Option<CandidateMatch>,
    pub alternatives: Vec<CandidateMatch>,
    pub failure: Option<PatternFailure>,
}

#[derive(Debug, Clone)]
struct PendingImplicitTag {
    pattern_span: PatternSpan,
    input_span: MatchSpan,
}

#[derive(Debug, Clone)]
struct MatchState {
    cursor: usize,
    captures: Vec<PatternCapture>,
    tags: Vec<ParseTagCapture>,
    mark: i32,
    marks: Vec<ParseMarkCapture>,
    pending_implicit_tag: Option<PendingImplicitTag>,
}

impl MatchState {
    fn new(cursor: usize) -> Self {
        Self {
            cursor,
            captures: Vec::new(),
            tags: Vec::new(),
            mark: 0,
            marks: Vec::new(),
            pending_implicit_tag: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TransitionKey {
    pattern_source: String,
    path: Vec<PatternPathSegment>,
    cursor: usize,
}

#[derive(Debug, Clone)]
enum CachedTransition {
    Literal {
        end: usize,
    },
    Regex {
        range: TextRange,
        groups: Vec<Option<TextRange>>,
    },
}

#[derive(Debug, Default)]
struct FailureTracker {
    offset: usize,
    reasons: BTreeSet<PatternFailureReason>,
    initialized: bool,
}

impl FailureTracker {
    fn record(&mut self, offset: usize, reason: PatternFailureReason) {
        if !self.initialized || offset > self.offset {
            self.offset = offset;
            self.reasons.clear();
            self.initialized = true;
        }
        if offset == self.offset {
            self.reasons.insert(reason);
        }
    }
}

struct CandidateContext<'a> {
    candidate: &'a PatternCandidate<'a>,
    pattern_index: Option<usize>,
    pattern: Option<&'a MatchPattern<'a>>,
}

struct MatchEngine<'input, 'candidate, 'ext, R, H> {
    input: MatchInput<'input>,
    resolver: &'ext mut R,
    hooks: &'ext mut H,
    config: PatternMatcherConfig,
    failure: FailureTracker,
    states: usize,
    backtracks: usize,
    regex_executions: usize,
    regex_evaluated_bytes: usize,
    transitions: HashMap<TransitionKey, Vec<CachedTransition>>,
    regexes: HashMap<String, Result<Regex, String>>,
    trim_range: TextRange,
    current: Option<CandidateContext<'candidate>>,
}

/// Matches and deterministically ranks all candidates against one complete input.
pub fn match_pattern_candidates<R: TypeExpressionResolver, H: PatternMatchHooks>(
    input: MatchInput<'_>,
    candidates: &[PatternCandidate<'_>],
    resolver: &mut R,
    hooks: &mut H,
    config: PatternMatcherConfig,
) -> Result<CandidateMatches, PatternMatchError> {
    config.validate()?;
    let trim_range = java_trim_range(input.text());
    let mut engine = MatchEngine {
        input,
        resolver,
        hooks,
        config,
        failure: FailureTracker::default(),
        states: 0,
        backtracks: 0,
        regex_executions: 0,
        regex_evaluated_bytes: 0,
        transitions: HashMap::new(),
        regexes: HashMap::new(),
        trim_range,
        current: None,
    };

    let mut ranked = candidates.iter().enumerate().collect::<Vec<_>>();
    ranked.sort_by_key(|(declaration_order, candidate)| {
        (
            candidate.resolved_order.is_none(),
            candidate.resolved_order.unwrap_or(usize::MAX),
            candidate.priority,
            candidate.registration_order,
            *declaration_order,
        )
    });

    let mut matches = Vec::new();
    for (_, candidate) in ranked {
        if let Some(value) = engine.match_candidate(candidate)? {
            matches.push(value);
        }
    }

    let selected = (!matches.is_empty()).then(|| matches.remove(0));
    let failure = if selected.is_none() && engine.failure.initialized {
        Some(PatternFailure {
            offset: engine.failure.offset,
            span: engine
                .input
                .map_range(TextRange::empty(engine.failure.offset))?,
            reasons: engine.failure.reasons.into_iter().collect(),
        })
    } else {
        None
    };
    Ok(CandidateMatches {
        selected,
        alternatives: matches,
        failure,
    })
}

impl<'input, 'candidate, 'ext, R: TypeExpressionResolver, H: PatternMatchHooks>
    MatchEngine<'input, 'candidate, 'ext, R, H>
{
    fn match_candidate(
        &mut self,
        candidate: &'candidate PatternCandidate<'candidate>,
    ) -> Result<Option<CandidateMatch>, PatternMatchError> {
        self.current = Some(CandidateContext {
            candidate,
            pattern_index: None,
            pattern: None,
        });

        match self.scope_before(PatternHookScope::Definition)? {
            ScopeDecision::Failed => {
                let after = self.scope_after_failed(
                    PatternHookScope::Definition,
                    "definition hook rejected the candidate",
                )?;
                return if matches!(after, ScopeDecision::Matched(range) if range == self.trim_range)
                {
                    self.first_synthetic_candidate_match(candidate)
                } else {
                    Ok(None)
                };
            }
            ScopeDecision::Matched(range) if range == self.trim_range => {
                let matched = self.first_synthetic_candidate_match(candidate)?;
                if matched.is_none() {
                    self.scope_after_failed(
                        PatternHookScope::Definition,
                        "definition hook matched a candidate without patterns",
                    )?;
                    return Ok(None);
                }
                return if self.scope_after_matched(PatternHookScope::Definition, range)? {
                    Ok(matched)
                } else {
                    Ok(None)
                };
            }
            ScopeDecision::Matched(_) | ScopeDecision::Continue => {}
        }

        match self.scope_before(PatternHookScope::Registration)? {
            ScopeDecision::Failed => {
                let registration_after = self.scope_after_failed(
                    PatternHookScope::Registration,
                    "registration hook rejected the candidate",
                )?;
                if matches!(
                    registration_after,
                    ScopeDecision::Matched(range) if range == self.trim_range
                ) {
                    let matched = self.first_synthetic_candidate_match(candidate)?;
                    if matched.is_some()
                        && self
                            .scope_after_matched(PatternHookScope::Definition, self.trim_range)?
                    {
                        return Ok(matched);
                    }
                    return Ok(None);
                }

                let definition_after = self.scope_after_failed(
                    PatternHookScope::Definition,
                    "registration hook rejected the candidate",
                )?;
                return if matches!(
                    definition_after,
                    ScopeDecision::Matched(range) if range == self.trim_range
                ) {
                    self.first_synthetic_candidate_match(candidate)
                } else {
                    Ok(None)
                };
            }
            ScopeDecision::Matched(range) if range == self.trim_range => {
                let matched = self.first_synthetic_candidate_match(candidate)?;
                if matched.is_none() {
                    self.scope_after_failed(
                        PatternHookScope::Registration,
                        "registration hook matched a candidate without patterns",
                    )?;
                    self.scope_after_failed(
                        PatternHookScope::Definition,
                        "registration hook matched a candidate without patterns",
                    )?;
                    return Ok(None);
                }
                let registration_accepted =
                    self.scope_after_matched(PatternHookScope::Registration, range)?;
                if registration_accepted
                    && self.scope_after_matched(PatternHookScope::Definition, range)?
                {
                    return Ok(matched);
                }
                if !registration_accepted {
                    let definition_after = self.scope_after_failed(
                        PatternHookScope::Definition,
                        "registration hook rejected the candidate after matching",
                    )?;
                    if matches!(
                        definition_after,
                        ScopeDecision::Matched(replacement) if replacement == self.trim_range
                    ) {
                        return Ok(matched);
                    }
                }
                return Ok(None);
            }
            ScopeDecision::Matched(_) | ScopeDecision::Continue => {}
        }

        let mut matched = None;
        for (pattern_index, pattern) in candidate.patterns.iter().enumerate() {
            self.current = Some(CandidateContext {
                candidate,
                pattern_index: Some(pattern_index),
                pattern: Some(pattern),
            });

            match self.scope_before(PatternHookScope::Pattern)? {
                ScopeDecision::Failed => {
                    let after = self.scope_after_failed(
                        PatternHookScope::Pattern,
                        "pattern hook rejected the pattern",
                    )?;
                    if matches!(after, ScopeDecision::Matched(range) if range == self.trim_range) {
                        matched = Some(self.synthetic_candidate_match(
                            candidate,
                            pattern_index,
                            pattern,
                        )?);
                        break;
                    }
                    continue;
                }
                ScopeDecision::Matched(range) if range == self.trim_range => {
                    let value =
                        self.synthetic_candidate_match(candidate, pattern_index, pattern)?;
                    if self.scope_after_matched(PatternHookScope::Pattern, range)? {
                        matched = Some(value);
                        break;
                    }
                    continue;
                }
                ScopeDecision::Matched(_) | ScopeDecision::Continue => {}
            }

            let mut path = Vec::new();
            let states = self.match_sequence(
                &pattern.parsed.elements,
                0,
                MatchState::new(self.trim_range.start),
                &mut path,
            )?;
            if let Some(state) = states
                .iter()
                .find(|state| state.cursor == self.trim_range.end)
                .cloned()
            {
                let value = CandidateMatch {
                    kind: candidate.kind,
                    definition_id: candidate.definition_id.to_owned(),
                    registration_id: candidate.registration_id.to_owned(),
                    priority: candidate.priority,
                    registration_order: candidate.registration_order,
                    pattern_index,
                    pattern: pattern.source.to_owned(),
                    matched: PatternMatch {
                        span: self.input.map_range(self.trim_range)?,
                        captures: state.captures,
                        tags: state.tags,
                        mark: state.mark,
                        marks: state.marks,
                    },
                };
                if self.scope_after_matched(PatternHookScope::Pattern, self.trim_range)? {
                    matched = Some(value);
                    break;
                }
            } else {
                for state in &states {
                    self.failure
                        .record(state.cursor, PatternFailureReason::TrailingInput);
                }
                let after = self.scope_after_failed(
                    PatternHookScope::Pattern,
                    "pattern did not consume the complete input",
                )?;
                if matches!(after, ScopeDecision::Matched(range) if range == self.trim_range) {
                    matched =
                        Some(self.synthetic_candidate_match(candidate, pattern_index, pattern)?);
                    break;
                }
            }
        }

        self.current = Some(CandidateContext {
            candidate,
            pattern_index: matched.as_ref().map(|value| value.pattern_index),
            pattern: matched
                .as_ref()
                .and_then(|value| candidate.patterns.get(value.pattern_index)),
        });

        if let Some(matched) = matched {
            let registration_accepted =
                self.scope_after_matched(PatternHookScope::Registration, self.trim_range)?;
            if registration_accepted {
                return if self.scope_after_matched(PatternHookScope::Definition, self.trim_range)? {
                    Ok(Some(matched))
                } else {
                    Ok(None)
                };
            }

            let definition_after = self.scope_after_failed(
                PatternHookScope::Definition,
                "registration hook rejected the candidate after matching",
            )?;
            return if matches!(
                definition_after,
                ScopeDecision::Matched(range) if range == self.trim_range
            ) {
                Ok(Some(matched))
            } else {
                Ok(None)
            };
        }

        let registration_after = self.scope_after_failed(
            PatternHookScope::Registration,
            "no registered pattern matched",
        )?;
        if matches!(
            registration_after,
            ScopeDecision::Matched(range) if range == self.trim_range
        ) {
            let matched = self.first_synthetic_candidate_match(candidate)?;
            if matched.is_some()
                && self.scope_after_matched(PatternHookScope::Definition, self.trim_range)?
            {
                return Ok(matched);
            }
            return Ok(None);
        }

        let definition_after = self.scope_after_failed(
            PatternHookScope::Definition,
            "no registered pattern matched",
        )?;
        if matches!(
            definition_after,
            ScopeDecision::Matched(range) if range == self.trim_range
        ) {
            return self.first_synthetic_candidate_match(candidate);
        }
        Ok(None)
    }
    fn first_synthetic_candidate_match(
        &mut self,
        candidate: &'candidate PatternCandidate<'candidate>,
    ) -> Result<Option<CandidateMatch>, PatternMatchError> {
        let Some(pattern) = candidate.patterns.first() else {
            return Ok(None);
        };
        self.current = Some(CandidateContext {
            candidate,
            pattern_index: Some(0),
            pattern: Some(pattern),
        });
        self.synthetic_candidate_match(candidate, 0, pattern)
            .map(Some)
    }

    fn synthetic_candidate_match(
        &self,
        candidate: &PatternCandidate<'_>,
        pattern_index: usize,
        pattern: &MatchPattern<'_>,
    ) -> Result<CandidateMatch, PatternMatchError> {
        Ok(CandidateMatch {
            kind: candidate.kind,
            definition_id: candidate.definition_id.to_owned(),
            registration_id: candidate.registration_id.to_owned(),
            priority: candidate.priority,
            registration_order: candidate.registration_order,
            pattern_index,
            pattern: pattern.source.to_owned(),
            matched: PatternMatch {
                span: self.input.map_range(self.trim_range)?,
                captures: Vec::new(),
                tags: Vec::new(),
                mark: 0,
                marks: Vec::new(),
            },
        })
    }

    fn scope_before(
        &mut self,
        scope: PatternHookScope,
    ) -> Result<ScopeDecision, PatternMatchError> {
        let control = self.dispatch_hook(
            scope,
            PatternHookTiming::Before,
            &[],
            None,
            self.trim_range,
            PatternHookOutcome::Pending,
        )?;
        Ok(match control {
            PatternHookControl::Continue => ScopeDecision::Continue,
            PatternHookControl::Match(range) => {
                self.validate_hook_range(range, self.trim_range)?;
                ScopeDecision::Matched(range)
            }
            PatternHookControl::Fail(reason) => {
                self.failure.record(
                    self.trim_range.start,
                    PatternFailureReason::HookRejected { reason },
                );
                ScopeDecision::Failed
            }
        })
    }

    fn scope_after_matched(
        &mut self,
        scope: PatternHookScope,
        range: TextRange,
    ) -> Result<bool, PatternMatchError> {
        match self.dispatch_hook(
            scope,
            PatternHookTiming::After,
            &[],
            None,
            range,
            PatternHookOutcome::Matched { range },
        )? {
            PatternHookControl::Fail(reason) => {
                self.failure
                    .record(range.end, PatternFailureReason::HookRejected { reason });
                Ok(false)
            }
            PatternHookControl::Match(replacement) => {
                self.validate_hook_range(replacement, self.trim_range)?;
                Ok(replacement == self.trim_range)
            }
            PatternHookControl::Continue => Ok(true),
        }
    }

    fn scope_after_failed(
        &mut self,
        scope: PatternHookScope,
        reason: &str,
    ) -> Result<ScopeDecision, PatternMatchError> {
        let control = self.dispatch_hook(
            scope,
            PatternHookTiming::After,
            &[],
            None,
            TextRange::empty(self.trim_range.start),
            PatternHookOutcome::Failed {
                reason: reason.to_owned(),
            },
        )?;
        Ok(match control {
            PatternHookControl::Continue => ScopeDecision::Continue,
            PatternHookControl::Match(range) => {
                self.validate_hook_range(range, self.trim_range)?;
                ScopeDecision::Matched(range)
            }
            PatternHookControl::Fail(reason) => {
                self.failure.record(
                    self.trim_range.start,
                    PatternFailureReason::HookRejected { reason },
                );
                ScopeDecision::Failed
            }
        })
    }

    fn match_sequence(
        &mut self,
        elements: &[SpannedPatternElement],
        index: usize,
        state: MatchState,
        path: &mut Vec<PatternPathSegment>,
    ) -> Result<Vec<MatchState>, PatternMatchError> {
        self.visit_state()?;
        if index == elements.len() {
            return Ok(vec![state]);
        }

        path.push(PatternPathSegment::Element(
            u32::try_from(index).unwrap_or(u32::MAX),
        ));
        let transitions = self.match_element(&elements[index], state, path)?;
        path.pop();
        self.add_backtracks(transitions.len().saturating_sub(1))?;

        let mut matches = Vec::new();
        for transition in transitions {
            let mut tail = self.match_sequence(elements, index + 1, transition, path)?;
            matches.append(&mut tail);
        }
        Ok(matches)
    }

    fn match_element(
        &mut self,
        element: &SpannedPatternElement,
        state: MatchState,
        path: &mut Vec<PatternPathSegment>,
    ) -> Result<Vec<MatchState>, PatternMatchError> {
        let start = state.cursor;
        let control = self.dispatch_hook(
            PatternHookScope::Element,
            PatternHookTiming::Before,
            path,
            Some(element.span),
            TextRange::empty(start),
            PatternHookOutcome::Pending,
        )?;
        let original = state.clone();
        let mut transitions = match control {
            PatternHookControl::Continue => self.match_element_default(element, state, path)?,
            PatternHookControl::Match(range) => {
                self.validate_hook_range(range, TextRange::new(start, self.trim_range.end))?;
                let mut state = state;
                state.cursor = range.end;
                vec![state]
            }
            PatternHookControl::Fail(reason) => {
                self.failure
                    .record(start, PatternFailureReason::HookRejected { reason });
                Vec::new()
            }
        };

        if transitions.is_empty() {
            match self.dispatch_hook(
                PatternHookScope::Element,
                PatternHookTiming::After,
                path,
                Some(element.span),
                TextRange::empty(start),
                PatternHookOutcome::Failed {
                    reason: "pattern element did not match".to_owned(),
                },
            )? {
                PatternHookControl::Match(range) => {
                    self.validate_hook_range(range, TextRange::new(start, self.trim_range.end))?;
                    let mut state = original;
                    state.cursor = range.end;
                    transitions.push(state);
                }
                PatternHookControl::Fail(reason) => self
                    .failure
                    .record(start, PatternFailureReason::HookRejected { reason }),
                PatternHookControl::Continue => {}
            }
            return Ok(transitions);
        }

        let mut accepted = Vec::with_capacity(transitions.len());
        for mut transition in transitions {
            let range = TextRange::new(start, transition.cursor);
            match self.dispatch_hook(
                PatternHookScope::Element,
                PatternHookTiming::After,
                path,
                Some(element.span),
                range,
                PatternHookOutcome::Matched { range },
            )? {
                PatternHookControl::Continue => accepted.push(transition),
                PatternHookControl::Match(replacement) => {
                    self.validate_hook_range(
                        replacement,
                        TextRange::new(start, self.trim_range.end),
                    )?;
                    transition.cursor = replacement.end;
                    accepted.push(transition);
                }
                PatternHookControl::Fail(reason) => self.failure.record(
                    transition.cursor,
                    PatternFailureReason::HookRejected { reason },
                ),
            }
        }
        Ok(accepted)
    }

    fn match_element_default(
        &mut self,
        element: &SpannedPatternElement,
        mut state: MatchState,
        path: &mut Vec<PatternPathSegment>,
    ) -> Result<Vec<MatchState>, PatternMatchError> {
        match &element.value {
            PatternElement::Literal(literal) => {
                if let Some(pending) = state.pending_implicit_tag.take() {
                    let value = literal.trim().to_owned();
                    if !value.is_empty() {
                        state.tags.push(ParseTagCapture {
                            value,
                            pattern_span: pending.pattern_span,
                            input_span: pending.input_span,
                            implicit: true,
                        });
                    }
                }
                self.match_literal(literal, state, path)
            }
            PatternElement::Choice(branches) => {
                self.add_backtracks(branches.len().saturating_sub(1))?;
                let mut matches = Vec::new();
                for (index, branch) in branches.iter().enumerate() {
                    path.push(PatternPathSegment::Branch(
                        u32::try_from(index).unwrap_or(u32::MAX),
                    ));
                    let mut branch_matches = self.match_sequence(branch, 0, state.clone(), path)?;
                    path.pop();
                    matches.append(&mut branch_matches);
                }
                Ok(matches)
            }
            PatternElement::Group(elements) => self.match_sequence(elements, 0, state, path),
            PatternElement::Option(elements) => {
                let mut matches = self.match_sequence(elements, 0, state.clone(), path)?;
                state.pending_implicit_tag = None;
                matches.push(state);
                Ok(matches)
            }
            PatternElement::Regex(pattern) => {
                state.pending_implicit_tag = None;
                self.match_regex(element.span, pattern, state, path)
            }
            PatternElement::TypeExpr(expression) => {
                state.pending_implicit_tag = None;
                self.match_type_expression(element.span, expression, state)
            }
            PatternElement::ParseTag(tag) => {
                let input_span = self.input.map_range(TextRange::empty(state.cursor))?;
                if tag.is_empty() {
                    state.pending_implicit_tag = Some(PendingImplicitTag {
                        pattern_span: element.span,
                        input_span,
                    });
                } else {
                    state.tags.push(ParseTagCapture {
                        value: tag.clone(),
                        pattern_span: element.span,
                        input_span,
                        implicit: false,
                    });
                    state.pending_implicit_tag = None;
                }
                Ok(vec![state])
            }
            PatternElement::ParseMark(mark) => {
                state.pending_implicit_tag = None;
                state.mark ^= *mark;
                state.marks.push(ParseMarkCapture {
                    value: *mark,
                    pattern_span: element.span,
                    input_span: self.input.map_range(TextRange::empty(state.cursor))?,
                    accumulated: state.mark,
                });
                Ok(vec![state])
            }
            PatternElement::Empty => {
                state.pending_implicit_tag = None;
                Ok(vec![state])
            }
        }
    }

    fn match_literal(
        &mut self,
        literal: &str,
        mut state: MatchState,
        path: &[PatternPathSegment],
    ) -> Result<Vec<MatchState>, PatternMatchError> {
        let key = self.transition_key(path, state.cursor);
        let transitions = if let Some(value) = self.transitions.get(&key) {
            value.clone()
        } else {
            let value =
                match match_literal_at(self.input.text(), self.trim_range, state.cursor, literal) {
                    Ok(end) => vec![CachedTransition::Literal { end }],
                    Err(offset) => {
                        self.failure.record(
                            offset,
                            PatternFailureReason::Literal {
                                expected: literal.to_owned(),
                            },
                        );
                        Vec::new()
                    }
                };
            self.transitions.insert(key, value.clone());
            value
        };

        let Some(CachedTransition::Literal { end }) = transitions.first() else {
            self.failure.record(
                state.cursor,
                PatternFailureReason::Literal {
                    expected: literal.to_owned(),
                },
            );
            return Ok(Vec::new());
        };
        state.cursor = *end;
        Ok(vec![state])
    }

    fn match_regex(
        &mut self,
        pattern_span: PatternSpan,
        pattern: &str,
        state: MatchState,
        path: &[PatternPathSegment],
    ) -> Result<Vec<MatchState>, PatternMatchError> {
        let key = self.transition_key(path, state.cursor);
        let transitions = if let Some(value) = self.transitions.get(&key) {
            value.clone()
        } else {
            let regex = self.regex(pattern, pattern_span)?;
            let mut values = Vec::new();
            for end in skript_boundaries(self.input.text(), state.cursor, self.trim_range.end) {
                self.regex_executions = self.regex_executions.saturating_add(1);
                if self.regex_executions > self.config.max_regex_executions {
                    return Err(PatternMatchError::LimitExceeded {
                        kind: PatternMatchLimit::RegexExecutions,
                        limit: self.config.max_regex_executions,
                    });
                }
                self.regex_evaluated_bytes = self
                    .regex_evaluated_bytes
                    .saturating_add(end.saturating_sub(state.cursor));
                if self.regex_evaluated_bytes > self.config.max_regex_evaluated_bytes {
                    return Err(PatternMatchError::LimitExceeded {
                        kind: PatternMatchLimit::RegexEvaluatedBytes,
                        limit: self.config.max_regex_evaluated_bytes,
                    });
                }
                let Some(subject) = self.input.text().get(state.cursor..end) else {
                    continue;
                };
                let captures = match regex.captures(subject) {
                    Ok(Some(captures)) => captures,
                    Ok(None) => continue,
                    Err(FancyRegexError::RuntimeError(RuntimeError::BacktrackLimitExceeded)) => {
                        return Err(PatternMatchError::LimitExceeded {
                            kind: PatternMatchLimit::RegexBacktracks,
                            limit: self.config.max_regex_backtracks,
                        });
                    }
                    Err(error) => {
                        return Err(PatternMatchError::InvalidRegex {
                            pattern_span,
                            message: error.to_string(),
                        });
                    }
                };
                let groups = (1..captures.len())
                    .map(|index| {
                        captures.get(index).map(|capture| {
                            TextRange::new(
                                state.cursor + capture.start(),
                                state.cursor + capture.end(),
                            )
                        })
                    })
                    .collect();
                values.push(CachedTransition::Regex {
                    range: TextRange::new(state.cursor, end),
                    groups,
                });
            }
            self.transitions.insert(key, values.clone());
            values
        };

        if transitions.is_empty() {
            self.failure.record(
                state.cursor,
                PatternFailureReason::Regex {
                    pattern: pattern.to_owned(),
                },
            );
            return Ok(Vec::new());
        }

        transitions
            .into_iter()
            .map(|transition| {
                let CachedTransition::Regex { range, groups } = transition else {
                    unreachable!("transition keys include the complete pattern and element path");
                };
                let mut next = state.clone();
                next.cursor = range.end;
                next.captures.push(PatternCapture::Regex {
                    pattern_span,
                    value: range
                        .slice(self.input.text())
                        .expect("validated regex range")
                        .to_owned(),
                    span: self.input.map_range(range)?,
                    groups: groups
                        .into_iter()
                        .enumerate()
                        .map(|(index, range)| {
                            let value = range
                                .and_then(|range| range.slice(self.input.text()))
                                .map(ToOwned::to_owned);
                            let span =
                                range.map(|range| self.input.map_range(range)).transpose()?;
                            Ok(RegexGroupCapture {
                                index: index + 1,
                                value,
                                span,
                            })
                        })
                        .collect::<Result<Vec<_>, PatternMatchError>>()?,
                });
                Ok(next)
            })
            .collect()
    }

    fn match_type_expression(
        &mut self,
        pattern_span: PatternSpan,
        expression: &PatternTypeExpr,
        state: MatchState,
    ) -> Result<Vec<MatchState>, PatternMatchError> {
        let mut boundaries =
            skript_boundaries(self.input.text(), state.cursor, self.trim_range.end);
        if expression.nullable {
            boundaries.insert(0, state.cursor);
        }
        let resolutions = self
            .resolver
            .resolve(TypeExpressionRequest {
                input: self.input.text(),
                expression,
                pattern_span,
                remaining: TextRange::new(state.cursor, self.trim_range.end),
                candidate_ends: &boundaries,
            })
            .map_err(|message| PatternMatchError::TypeResolver {
                pattern_span,
                message,
            })?;

        let legal = boundaries.into_iter().collect::<HashSet<_>>();
        let mut matches = Vec::new();
        for resolution in resolutions {
            let range = resolution.range;
            if range.start != state.cursor
                || range.end > self.trim_range.end
                || !range.is_valid_for(self.input.text())
                || !legal.contains(&range.end)
                || resolution
                    .alternative_index
                    .is_some_and(|index| index >= expression.alternatives.len())
            {
                return Err(PatternMatchError::InvalidTypeResolution { range });
            }
            let mut next = state.clone();
            next.cursor = range.end;
            next.captures.push(PatternCapture::TypeExpression {
                pattern_span,
                expression: expression.clone(),
                value: range
                    .slice(self.input.text())
                    .expect("validated type expression range")
                    .to_owned(),
                span: self.input.map_range(range)?,
                alternative_index: resolution.alternative_index,
                resolution_id: resolution.resolution_id,
            });
            matches.push(next);
        }

        if matches.is_empty() {
            self.failure.record(
                state.cursor,
                PatternFailureReason::TypeExpression {
                    expected: expression
                        .alternatives
                        .iter()
                        .map(|alternative| alternative.name.clone())
                        .collect(),
                },
            );
        }
        Ok(matches)
    }

    fn regex(
        &mut self,
        pattern: &str,
        pattern_span: PatternSpan,
    ) -> Result<Regex, PatternMatchError> {
        let compiled = self.regexes.entry(pattern.to_owned()).or_insert_with(|| {
            let mut builder = RegexBuilder::new(&format!(r"\A(?:{pattern})\z"));
            builder.backtrack_limit(self.config.max_regex_backtracks);
            builder.build().map_err(|error| error.to_string())
        });
        compiled
            .clone()
            .map_err(|message| PatternMatchError::InvalidRegex {
                pattern_span,
                message,
            })
    }

    fn transition_key(&self, path: &[PatternPathSegment], cursor: usize) -> TransitionKey {
        let pattern_source = self
            .current
            .as_ref()
            .and_then(|current| current.pattern)
            .map_or_else(String::new, |pattern| pattern.source.to_owned());
        TransitionKey {
            pattern_source,
            path: path.to_vec(),
            cursor,
        }
    }

    fn dispatch_hook(
        &mut self,
        scope: PatternHookScope,
        timing: PatternHookTiming,
        element_path: &[PatternPathSegment],
        pattern_span: Option<PatternSpan>,
        input_range: TextRange,
        outcome: PatternHookOutcome,
    ) -> Result<PatternHookControl, PatternMatchError> {
        let current = self
            .current
            .as_ref()
            .expect("hooks only run while matching a candidate");
        self.hooks
            .dispatch(PatternHookEvent {
                kind: current.candidate.kind,
                definition_id: &current.candidate.definition_id,
                registration_id: &current.candidate.registration_id,
                pattern_index: current.pattern_index,
                pattern: current.pattern.map(|pattern| pattern.source),
                element_path,
                pattern_span,
                scope,
                timing,
                input_range,
                input_span: self.input.map_range(input_range)?,
                outcome,
            })
            .map_err(|message| PatternMatchError::Hook { message })
    }

    fn validate_hook_range(
        &self,
        range: TextRange,
        allowed: TextRange,
    ) -> Result<(), PatternMatchError> {
        if !range.is_valid_for(self.input.text())
            || range.start != allowed.start
            || range.end > allowed.end
        {
            Err(PatternMatchError::InvalidInputRange { range })
        } else {
            Ok(())
        }
    }

    fn visit_state(&mut self) -> Result<(), PatternMatchError> {
        self.states = self.states.saturating_add(1);
        if self.states > self.config.max_states {
            Err(PatternMatchError::LimitExceeded {
                kind: PatternMatchLimit::States,
                limit: self.config.max_states,
            })
        } else {
            Ok(())
        }
    }

    fn add_backtracks(&mut self, amount: usize) -> Result<(), PatternMatchError> {
        self.backtracks = self.backtracks.saturating_add(amount);
        if self.backtracks > self.config.max_backtracks {
            Err(PatternMatchError::LimitExceeded {
                kind: PatternMatchLimit::Backtracks,
                limit: self.config.max_backtracks,
            })
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeDecision {
    Continue,
    Matched(TextRange),
    Failed,
}

fn java_trim_range(input: &str) -> TextRange {
    let start = input
        .char_indices()
        .find_map(|(index, ch)| ((ch as u32) > 0x20).then_some(index))
        .unwrap_or(input.len());
    let end = input
        .char_indices()
        .rev()
        .find_map(|(index, ch)| ((ch as u32) > 0x20).then_some(index + ch.len_utf8()))
        .unwrap_or(start);
    TextRange::new(start, end)
}

fn match_literal_at(
    input: &str,
    complete: TextRange,
    mut cursor: usize,
    literal: &str,
) -> Result<usize, usize> {
    for expected in literal.chars() {
        if expected == ' ' {
            if cursor == complete.start || cursor == complete.end {
                continue;
            }
            if input.as_bytes().get(cursor) == Some(&b' ') {
                cursor += 1;
                continue;
            }
            if cursor > complete.start && input.as_bytes().get(cursor - 1) == Some(&b' ') {
                continue;
            }
            return Err(cursor);
        }

        let Some(actual) = input
            .get(cursor..complete.end)
            .and_then(|remaining| remaining.chars().next())
        else {
            return Err(cursor);
        };
        if !char_eq_ignore_case(expected, actual) {
            return Err(cursor);
        }
        cursor += actual.len_utf8();
    }
    Ok(cursor)
}

fn char_eq_ignore_case(left: char, right: char) -> bool {
    left == right || left.to_lowercase().eq(right.to_lowercase())
}

/// Returns legal expression split points in Skript traversal order.
fn skript_boundaries(input: &str, start: usize, end: usize) -> Vec<usize> {
    if start >= end {
        return Vec::new();
    }
    let mut boundaries = Vec::new();
    let mut cursor = start;
    while cursor < end {
        let Some(next) = skript_next(input, cursor, end) else {
            break;
        };
        if next <= cursor || next > end {
            break;
        }
        boundaries.push(next);
        cursor = next;
    }
    boundaries
}

fn skript_next(input: &str, start: usize, end: usize) -> Option<usize> {
    let current = input.get(start..end)?.chars().next()?;
    match current {
        '"' => find_quote_end(input, start + current.len_utf8(), end)
            .map(|index| index + '"'.len_utf8()),
        '{' => find_variable_end(input, start + current.len_utf8(), end)
            .map(|index| index + '}'.len_utf8()),
        '(' => find_parenthesis_end(input, start + current.len_utf8(), end)
            .map(|index| index + ')'.len_utf8()),
        _ => Some(start + current.len_utf8()),
    }
}

fn find_quote_end(input: &str, mut cursor: usize, end: usize) -> Option<usize> {
    let mut in_expression = false;
    while cursor < end {
        let ch = input.get(cursor..end)?.chars().next()?;
        if ch == '"' && !in_expression {
            let next = cursor + ch.len_utf8();
            if input.as_bytes().get(next) == Some(&b'"') {
                cursor = next + 1;
                continue;
            }
            return Some(cursor);
        }
        if ch == '%' {
            in_expression = !in_expression;
        }
        cursor += ch.len_utf8();
    }
    None
}

fn find_variable_end(input: &str, mut cursor: usize, end: usize) -> Option<usize> {
    let mut depth = 0usize;
    while cursor < end {
        let ch = input.get(cursor..end)?.chars().next()?;
        match ch {
            '{' => depth = depth.saturating_add(1),
            '}' if depth == 0 => return Some(cursor),
            '}' => depth -= 1,
            _ => {}
        }
        cursor += ch.len_utf8();
    }
    None
}

fn find_parenthesis_end(input: &str, mut cursor: usize, end: usize) -> Option<usize> {
    let mut depth = 0usize;
    while cursor < end {
        let ch = input.get(cursor..end)?.chars().next()?;
        match ch {
            '"' => {
                cursor = find_quote_end(input, cursor + ch.len_utf8(), end)? + ch.len_utf8();
                continue;
            }
            '{' => {
                cursor = find_variable_end(input, cursor + ch.len_utf8(), end)? + '}'.len_utf8();
                continue;
            }
            '(' => depth = depth.saturating_add(1),
            ')' if depth == 0 => return Some(cursor),
            ')' => depth -= 1,
            _ => {}
        }
        cursor += ch.len_utf8();
    }
    None
}

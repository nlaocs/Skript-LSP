//! Recursive Expression parsing over SSG registrations and parser extensions.
//!
//! The parser owns candidate ordering, type filtering, recursion limits, and
//! memoization. CoreLibrary and addon Components provide leaf expressions such
//! as variables and literals through [`ExpressionParseEnvironment`].
#![allow(missing_docs)] // Aggregate contracts are documented on their owning types.

use crate::{
    CandidateMatch, MappedSource, MatchInput, MatchSpan, ParseTagCapture, PatternCandidate,
    PatternCapture, PatternHookControl, PatternHookEvent, PatternMatchEnvironment,
    PatternMatchError, PatternMatcherConfig, TextRange, TypeExpressionRequest,
    TypeExpressionResolution, catalog_pattern_candidates,
    match_pattern_candidates_with_environment, snapshot_pattern_candidates,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use syntax_pattern_parser::syntax::{PatternElement, SpannedPatternElement};
use syntaxes::{
    Catalog, ClassName, DynamicMultiplicity, DynamicSyntaxSnapshot, Multiplicity,
    PossibleReturnTypesState, ResolutionState, ReturnTypeState, SyntaxKind,
};
use thiserror::Error;

/// Stable parser context included in recursive-expression memo keys.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ExpressionParseContext {
    pub syntax_context: u64,
    pub event_classes: Vec<ClassName>,
    pub values: BTreeMap<String, String>,
}

/// Expected Java type and cardinality for one parse branch.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExpressionExpectedType {
    pub class_name: ClassName,
    pub plural: bool,
}

/// Input required to parse one complete Expression.
pub struct ExpressionParseRequest<'a> {
    pub source: &'a MappedSource,
    pub range: TextRange,
    pub expected_types: Vec<ExpressionExpectedType>,
    pub context: ExpressionParseContext,
}

/// Resource budgets applied across one complete recursive parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpressionParserConfig {
    pub matcher: PatternMatcherConfig,
    pub max_depth: usize,
    pub max_candidates: usize,
    pub max_memo_entries: usize,
}

impl Default for ExpressionParserConfig {
    fn default() -> Self {
        Self {
            matcher: PatternMatcherConfig::default(),
            max_depth: 64,
            max_candidates: 10_000,
            max_memo_entries: 20_000,
        }
    }
}

/// Source of an Expression node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpressionNodeKind {
    Registered {
        definition_id: String,
        registration_id: String,
        pattern_index: usize,
    },
    Variable {
        parser_id: String,
    },
    Literal {
        parser_id: String,
    },
    Function {
        parser_id: String,
    },
    Custom {
        parser_id: String,
    },
}

/// Parsed Expression node with nested typed captures and mapped provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpressionNode {
    pub kind: ExpressionNodeKind,
    pub span: MatchSpan,
    pub return_type: Option<ClassName>,
    pub multiplicity: Option<Multiplicity>,
    pub captures: Vec<PatternCapture>,
    pub tags: Vec<ParseTagCapture>,
    pub mark: i32,
    pub children: Vec<ExpressionNode>,
    pub metadata: BTreeMap<String, String>,
}

/// One valid Expression candidate in deterministic parser order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpressionCandidate {
    pub node: ExpressionNode,
    pub expected_alternative: Option<usize>,
}

/// Selected Expression, later alternatives, or a no-match diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpressionMatches {
    pub selected: Option<ExpressionCandidate>,
    pub alternatives: Vec<ExpressionCandidate>,
    pub failure: Option<ExpressionFailure>,
}

/// Farthest useful failure produced when no Expression consumes the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpressionFailure {
    pub span: MatchSpan,
    pub expected_types: Vec<ExpressionExpectedType>,
}

/// Kind assigned to one CoreLibrary or addon leaf parser result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpressionLeafKind {
    Variable,
    Literal,
    Function,
    Custom,
}

/// Request delivered to CoreLibrary and addon leaf-expression parsers.
pub struct ExpressionLeafRequest<'a> {
    pub input: &'a str,
    pub remaining: TextRange,
    pub span: MatchSpan,
    pub expected_types: &'a [ExpressionExpectedType],
    pub candidate_ends: &'a [usize],
    pub allow_literals: bool,
    pub allow_expressions: bool,
    pub time: i32,
    pub depth: usize,
    pub context: &'a ExpressionParseContext,
}

/// One prefix recognized by CoreLibrary or an addon parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpressionLeafCandidate {
    pub parser_id: String,
    pub kind: ExpressionLeafKind,
    pub range: TextRange,
    pub return_type: Option<ClassName>,
    pub multiplicity: Option<Multiplicity>,
    pub metadata: BTreeMap<String, String>,
}

/// Context supplied after a registered Expression and all typed captures matched.
pub struct RegisteredExpressionRequest<'a> {
    pub input: &'a str,
    pub definition_id: &'a str,
    pub registration_id: &'a str,
    pub element_class: &'a ClassName,
    pub related_property: Option<&'a str>,
    pub pattern_index: usize,
    pub span: &'a MatchSpan,
    pub expected_types: &'a [ExpressionExpectedType],
    pub declared_return_type: Option<&'a ClassName>,
    pub declared_multiplicity: Option<Multiplicity>,
    pub return_type_state: ReturnTypeState,
    pub possible_return_types: &'a [ClassName],
    pub possible_return_types_state: PossibleReturnTypesState,
    pub captures: &'a [PatternCapture],
    pub tags: &'a [ParseTagCapture],
    pub mark: i32,
    pub children: &'a [ExpressionNode],
    pub context: &'a ExpressionParseContext,
}

/// Semantic decision returned by CoreLibrary or an addon for a dynamic Expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisteredExpressionDecision {
    UseDeclared,
    Resolved {
        return_type: Option<ClassName>,
        multiplicity: Option<Multiplicity>,
        metadata: BTreeMap<String, String>,
    },
    Reject {
        reason: String,
    },
}

/// Unified native/WASM environment used during recursive Expression parsing.
pub trait ExpressionParseEnvironment: PatternMatchEnvironment {
    /// Returns leaf candidates in parser priority order.
    fn parse_expression_leaf(
        &mut self,
        request: ExpressionLeafRequest<'_>,
    ) -> Result<Vec<ExpressionLeafCandidate>, String>;

    /// Finalizes state and effects staged while producing the latest leaf set.
    ///
    /// `accepted` is `true` when at least one returned leaf survived native
    /// range, kind, type, and multiplicity validation. Environments may use
    /// this callback to retain or roll back their speculative transaction.
    fn finish_expression_leaf(&mut self, accepted: bool) -> Result<(), String> {
        let _ = accepted;
        Ok(())
    }

    /// Returns whether a semantic handler may replace this registration's
    /// declared return type after its captures have matched.
    fn can_resolve_registered_expression(&self, _element_class: &ClassName) -> bool {
        false
    }

    /// Resolves context-dependent return metadata after typed children matched.
    fn resolve_registered_expression(
        &mut self,
        _request: RegisteredExpressionRequest<'_>,
    ) -> Result<RegisteredExpressionDecision, String> {
        Ok(RegisteredExpressionDecision::UseDeclared)
    }

    /// Finalizes speculative state written by the latest registered resolver.
    fn finish_registered_expression(&mut self, accepted: bool) -> Result<(), String> {
        let _ = accepted;
        Ok(())
    }

    /// Returns the StateStore revision included in memoization keys.
    fn state_revision(&self) -> Result<u64, String>;
}

/// Native environment that contributes no leaves and never changes matching.
#[derive(Debug, Default)]
pub struct NoopExpressionEnvironment;

impl PatternMatchEnvironment for NoopExpressionEnvironment {
    fn resolve_type(
        &mut self,
        _request: TypeExpressionRequest<'_>,
    ) -> Result<Vec<TypeExpressionResolution>, String> {
        Ok(Vec::new())
    }

    fn dispatch_hook(
        &mut self,
        _event: PatternHookEvent<'_>,
    ) -> Result<PatternHookControl, String> {
        Ok(PatternHookControl::Continue)
    }
}

impl ExpressionParseEnvironment for NoopExpressionEnvironment {
    fn parse_expression_leaf(
        &mut self,
        _request: ExpressionLeafRequest<'_>,
    ) -> Result<Vec<ExpressionLeafCandidate>, String> {
        Ok(Vec::new())
    }

    fn state_revision(&self) -> Result<u64, String> {
        Ok(0)
    }
}

/// Failure while validating or recursively parsing an Expression.
#[derive(Debug, Error)]
pub enum ExpressionParseError {
    #[error("invalid expression input range {range}")]
    InvalidInputRange { range: TextRange },
    #[error("failed to map expression range: {message}")]
    SourceMap { message: String },
    #[error("expression parser environment failed: {message}")]
    Environment { message: String },
    #[error("expression matcher failed: {0}")]
    Matcher(#[from] PatternMatchError),
    #[error("expression parser exceeded the recursion depth limit of {limit}")]
    DepthLimit { limit: usize },
    #[error("expression parser exceeded the candidate limit of {limit}")]
    CandidateLimit { limit: usize },
    #[error("expression parser exceeded the memo entry limit of {limit}")]
    MemoLimit { limit: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MemoKey {
    // Context and registry candidates are immutable for the lifetime of a session.
    range: TextRange,
    candidate_ends: Vec<usize>,
    expected_type_id: usize,
    allow_literals: bool,
    allow_expressions: bool,
    time: i32,
    state_revision: u64,
}

#[derive(Debug, Clone)]
struct RegistrationMetadata {
    element_class: ClassName,
    related_property: Option<String>,
    return_type: Option<ClassName>,
    return_type_state: ReturnTypeState,
    possible_return_types: Vec<ClassName>,
    possible_return_types_state: PossibleReturnTypesState,
    multiplicity: Option<Multiplicity>,
    multiplicity_state: ResolutionState,
    metadata: BTreeMap<String, String>,
}

pub(crate) struct ExpressionSession<'a, E> {
    catalog: &'a Catalog,
    registered_candidates: Vec<PatternCandidate<'a>>,
    pattern_prefilters: HashMap<&'a str, PatternPrefilter>,
    pattern_initials: PatternInitialIndex,
    expected_type_ids: RefCell<HashMap<Vec<ExpressionExpectedType>, usize>>,
    candidate_compatibility_cache: RefCell<Vec<Vec<bool>>>,
    matcher_position_cache: RefCell<HashMap<MatcherPositionKey, Vec<PatternPosition>>>,
    registrations: HashMap<String, RegistrationMetadata>,
    source: &'a MappedSource,
    environment: &'a mut E,
    context: ExpressionParseContext,
    config: ExpressionParserConfig,
    memo: HashMap<MemoKey, Vec<ExpressionCandidate>>,
    active: HashSet<MemoKey>,
    resolved_nodes: HashMap<String, ExpressionNode>,
    frame_starts: Vec<usize>,
    frame_depths: Vec<usize>,
    next_resolution_id: u64,
    candidates_seen: usize,
}

/// Parses one complete Expression from an SSG catalog and extension environment.
///
/// Registered expressions are explored in catalog registration/pattern order.
/// Typed captures recursively invoke the same parser, while CoreLibrary and
/// addon parsers can contribute variables, literals, functions, and custom
/// leaves. The first candidate is selected and all later candidates are kept.
///
/// # Examples
///
/// ```no_run
/// use skript_parser::{
///     ExpressionExpectedType, ExpressionParseContext, ExpressionParseRequest,
///     ExpressionParserConfig, MappedSource, NoopExpressionEnvironment, TextRange,
///     parse_expression,
/// };
/// use syntaxes::{Catalog, ClassName};
///
/// fn parse_string(catalog: &Catalog) -> Result<(), Box<dyn std::error::Error>> {
///     let source = MappedSource::identity("dummy expression");
///     let mut environment = NoopExpressionEnvironment;
///     let matches = parse_expression(
///         catalog,
///         ExpressionParseRequest {
///             source: &source,
///             range: TextRange::new(0, source.virtual_source().len()),
///             expected_types: vec![ExpressionExpectedType {
///                 class_name: ClassName("java.lang.String".to_owned()),
///                 plural: false,
///             }],
///             context: ExpressionParseContext::default(),
///         },
///         &mut environment,
///         ExpressionParserConfig::default(),
///     )?;
///     let _selected = matches.selected;
///     Ok(())
/// }
/// ```
pub fn parse_expression<E: ExpressionParseEnvironment>(
    catalog: &Catalog,
    request: ExpressionParseRequest<'_>,
    environment: &mut E,
    config: ExpressionParserConfig,
) -> Result<ExpressionMatches, ExpressionParseError> {
    parse_expression_with_snapshot(catalog, None, request, environment, config)
}

/// Parses one complete Expression using a frozen dynamic registry when present.
///
/// The snapshot contributes dynamic definitions, overrides, and resolved
/// before/after ordering without mutating the immutable SSG catalog.
pub fn parse_expression_with_snapshot<E: ExpressionParseEnvironment>(
    catalog: &Catalog,
    dynamic_snapshot: Option<&DynamicSyntaxSnapshot>,
    request: ExpressionParseRequest<'_>,
    environment: &mut E,
    config: ExpressionParserConfig,
) -> Result<ExpressionMatches, ExpressionParseError> {
    if !request.range.is_valid_for(request.source.virtual_source()) {
        return Err(ExpressionParseError::InvalidInputRange {
            range: request.range,
        });
    }

    let expected_types = request.expected_types;

    let mut session = ExpressionSession::new(
        catalog,
        dynamic_snapshot,
        request.source,
        environment,
        request.context,
        config,
    );
    let candidates = session.parse_prefixes(
        request.range,
        &[request.range.end],
        &expected_types,
        true,
        true,
        0,
        0,
    )?;
    let mut candidates = candidates
        .into_iter()
        .filter(|candidate| candidate.node.span.local_range.end == request.range.end)
        .collect::<Vec<_>>();
    let selected = (!candidates.is_empty()).then(|| candidates.remove(0));
    let failure = if selected.is_none() {
        Some(ExpressionFailure {
            span: session.map_range(TextRange::empty(request.range.start))?,
            expected_types,
        })
    } else {
        None
    };
    Ok(ExpressionMatches {
        selected,
        alternatives: candidates,
        failure,
    })
}

impl<'a, E: ExpressionParseEnvironment> ExpressionSession<'a, E> {
    pub(crate) fn new(
        catalog: &'a Catalog,
        dynamic_snapshot: Option<&'a DynamicSyntaxSnapshot>,
        source: &'a MappedSource,
        environment: &'a mut E,
        context: ExpressionParseContext,
        config: ExpressionParserConfig,
    ) -> ExpressionSession<'a, E> {
        let registered_candidates = if let Some(snapshot) = dynamic_snapshot {
            snapshot_pattern_candidates(catalog, snapshot, SyntaxKind::Expression)
        } else {
            catalog_pattern_candidates(catalog, SyntaxKind::Expression)
        };
        let pattern_prefilters = pattern_prefilter_index(&registered_candidates);
        let pattern_initials = pattern_initial_index(&registered_candidates, &pattern_prefilters);
        ExpressionSession {
            catalog,
            registered_candidates,
            pattern_prefilters,
            pattern_initials,
            expected_type_ids: RefCell::new(HashMap::new()),
            candidate_compatibility_cache: RefCell::new(Vec::new()),
            matcher_position_cache: RefCell::new(HashMap::new()),
            registrations: registration_metadata_index(catalog, dynamic_snapshot),
            source,
            environment,
            context,
            config,
            memo: HashMap::new(),
            active: HashSet::new(),
            resolved_nodes: HashMap::new(),
            frame_starts: Vec::new(),
            frame_depths: Vec::new(),
            next_resolution_id: 0,
            candidates_seen: 0,
        }
    }

    pub(crate) fn match_candidates(
        &mut self,
        range: TextRange,
        candidates: &[PatternCandidate<'_>],
    ) -> Result<crate::CandidateMatches, ExpressionParseError> {
        if !range.is_valid_for(self.source.virtual_source()) {
            return Err(ExpressionParseError::InvalidInputRange { range });
        }
        let input = MatchInput::from_source(self.source, range)?;
        self.frame_starts.push(range.start);
        self.frame_depths.push(0);
        let matcher_config = self.config.matcher.clone();
        let matched =
            match_pattern_candidates_with_environment(input, candidates, self, matcher_config);
        self.frame_depths.pop();
        self.frame_starts.pop();
        Ok(matched?)
    }

    pub(crate) fn resolved_node(&self, id: &str) -> Option<&ExpressionNode> {
        self.resolved_nodes.get(id)
    }

    #[allow(clippy::too_many_arguments)]
    fn parse_prefixes(
        &mut self,
        range: TextRange,
        candidate_ends: &[usize],
        expected_types: &[ExpressionExpectedType],
        allow_literals: bool,
        allow_expressions: bool,
        time: i32,
        depth: usize,
    ) -> Result<Vec<ExpressionCandidate>, ExpressionParseError> {
        if depth > self.config.max_depth {
            return Err(ExpressionParseError::DepthLimit {
                limit: self.config.max_depth,
            });
        }
        self.validate_prefix_request(range, candidate_ends)?;
        let expected_type_id = self.expected_type_id(expected_types);
        let key = MemoKey {
            range,
            candidate_ends: candidate_ends.to_vec(),
            expected_type_id,
            allow_literals,
            allow_expressions,
            time,
            state_revision: self
                .environment
                .state_revision()
                .map_err(|message| ExpressionParseError::Environment { message })?,
        };
        if let Some(cached) = self.memo.get(&key) {
            return Ok(cached.clone());
        }
        if !self.active.insert(key.clone()) {
            return Ok(Vec::new());
        }
        if self.memo.len() >= self.config.max_memo_entries {
            self.active.remove(&key);
            return Err(ExpressionParseError::MemoLimit {
                limit: self.config.max_memo_entries,
            });
        }

        let result = (|| {
            let base = self.parse_prefixes_uncached(
                range,
                candidate_ends,
                expected_types,
                allow_literals,
                allow_expressions,
                time,
                depth,
                RegisteredPass::Base,
                true,
            )?;
            let mut candidates = Vec::new();
            self.extend_unique_candidates(&mut candidates, base)?;
            self.memo.insert(key.clone(), candidates.clone());

            for _ in 0..=self.config.max_depth {
                let recursive = self.parse_prefixes_uncached(
                    range,
                    candidate_ends,
                    expected_types,
                    allow_literals,
                    allow_expressions,
                    time,
                    depth,
                    RegisteredPass::LeftRecursive,
                    false,
                )?;
                let added = self.extend_unique_candidates(&mut candidates, recursive)?;
                self.memo.insert(key.clone(), candidates.clone());
                if added == 0 {
                    break;
                }
            }
            Ok(candidates)
        })();
        self.active.remove(&key);
        if result.is_err() {
            self.memo.remove(&key);
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn parse_prefixes_uncached(
        &mut self,
        range: TextRange,
        candidate_ends: &[usize],
        expected_types: &[ExpressionExpectedType],
        allow_literals: bool,
        allow_expressions: bool,
        time: i32,
        depth: usize,
        registered_pass: RegisteredPass,
        include_leaves: bool,
    ) -> Result<Vec<ExpressionCandidate>, ExpressionParseError> {
        let mut candidates = Vec::new();
        if include_leaves {
            let leaf_request = ExpressionLeafRequest {
                input: self.source.virtual_source(),
                remaining: range,
                span: self.map_range(range)?,
                expected_types,
                candidate_ends,
                allow_literals,
                allow_expressions,
                time,
                depth,
                context: &self.context,
            };
            let leaves = self
                .environment
                .parse_expression_leaf(leaf_request)
                .map_err(|message| ExpressionParseError::Environment { message })?;
            let mut accepted_leaves = Vec::new();
            for leaf in leaves {
                if !self.valid_leaf(
                    &leaf,
                    range,
                    candidate_ends,
                    expected_types,
                    allow_literals,
                    allow_expressions,
                ) {
                    continue;
                }
                let kind = match leaf.kind {
                    ExpressionLeafKind::Variable => ExpressionNodeKind::Variable {
                        parser_id: leaf.parser_id,
                    },
                    ExpressionLeafKind::Literal => ExpressionNodeKind::Literal {
                        parser_id: leaf.parser_id,
                    },
                    ExpressionLeafKind::Function => ExpressionNodeKind::Function {
                        parser_id: leaf.parser_id,
                    },
                    ExpressionLeafKind::Custom => ExpressionNodeKind::Custom {
                        parser_id: leaf.parser_id,
                    },
                };
                accepted_leaves.push(ExpressionCandidate {
                    node: ExpressionNode {
                        kind,
                        span: self.map_range(leaf.range)?,
                        return_type: leaf.return_type,
                        multiplicity: leaf.multiplicity,
                        captures: Vec::new(),
                        tags: Vec::new(),
                        mark: 0,
                        children: Vec::new(),
                        metadata: leaf.metadata,
                    },
                    expected_alternative: None,
                });
            }
            self.environment
                .finish_expression_leaf(!accepted_leaves.is_empty())
                .map_err(|message| ExpressionParseError::Environment { message })?;
            candidates.extend(accepted_leaves);
        }

        if allow_expressions {
            let input = range
                .slice(self.source.virtual_source())
                .expect("validated Expression range");
            let matcher_candidates =
                self.matcher_candidates(input, expected_types, registered_pass);
            for end in candidate_ends.iter().copied() {
                let candidate_range = TextRange::new(range.start, end);
                let input = MatchInput::from_source(self.source, candidate_range)?;
                self.frame_starts.push(candidate_range.start);
                self.frame_depths.push(depth);
                let matcher_config = self.config.matcher.clone();
                let matched = match_pattern_candidates_with_environment(
                    input,
                    &matcher_candidates,
                    self,
                    matcher_config,
                );
                self.frame_depths.pop();
                self.frame_starts.pop();
                let matched = matched?;
                if let Some(selected) = matched.selected
                    && let Some(candidate) =
                        self.registered_node(selected, candidate_range.start, expected_types)?
                {
                    candidates.push(candidate);
                }
                for alternative in matched.alternatives {
                    if let Some(candidate) =
                        self.registered_node(alternative, candidate_range.start, expected_types)?
                    {
                        candidates.push(candidate);
                    }
                }
            }
        }
        Ok(candidates)
    }

    fn extend_unique_candidates(
        &mut self,
        target: &mut Vec<ExpressionCandidate>,
        incoming: Vec<ExpressionCandidate>,
    ) -> Result<usize, ExpressionParseError> {
        let mut added = 0;
        for candidate in incoming {
            if target.contains(&candidate) {
                continue;
            }
            self.count_candidate()?;
            target.push(candidate);
            added += 1;
        }
        Ok(added)
    }
    fn registered_candidate_matches(
        &self,
        candidate: &PatternCandidate<'_>,
        expected_types: &[ExpressionExpectedType],
    ) -> bool {
        self.registration_metadata(&candidate.registration_id)
            .is_some_and(|metadata| {
                let return_type_matches = match metadata.return_type_state {
                    ReturnTypeState::Static => {
                        self.return_type_matches(metadata.return_type.as_ref(), expected_types)
                    }
                    ReturnTypeState::Dynamic | ReturnTypeState::Unresolved => {
                        self.environment
                            .can_resolve_registered_expression(&metadata.element_class)
                            || self
                                .return_type_matches(metadata.return_type.as_ref(), expected_types)
                            || metadata.possible_return_types.iter().any(|return_type| {
                                self.return_type_matches(Some(return_type), expected_types)
                            })
                    }
                };
                let multiplicity_matches = metadata.multiplicity_state
                    == ResolutionState::Unresolved
                    || self.multiplicity_matches(metadata.multiplicity, expected_types);
                return_type_matches && multiplicity_matches
            })
    }

    fn matcher_candidates(
        &self,
        input: &str,
        expected_types: &[ExpressionExpectedType],
        registered_pass: RegisteredPass,
    ) -> Vec<PatternCandidate<'a>> {
        let initial = input
            .chars()
            .find(|character| *character != ' ')
            .and_then(|character| character.to_lowercase().next());
        let key = MatcherPositionKey {
            initial,
            expected_type_id: self.expected_type_id(expected_types),
            registered_pass,
        };
        if !self.matcher_position_cache.borrow().contains_key(&key) {
            let compatibility_cache = self.candidate_compatibility_cache.borrow();
            let compatible = &compatibility_cache[key.expected_type_id];
            let mut positions = self.pattern_initials.wildcard.clone();
            if let Some(initial) = initial
                && let Some(indexed) = self.pattern_initials.by_initial.get(&initial)
            {
                positions.extend_from_slice(indexed);
            }
            positions.sort_unstable();
            positions.dedup();
            positions.retain(|(candidate_index, pattern_index)| {
                let candidate = &self.registered_candidates[*candidate_index];
                let pattern = candidate.patterns[*pattern_index];
                self.pattern_prefilters[pattern.source].left_recursive
                    == matches!(registered_pass, RegisteredPass::LeftRecursive)
                    && compatible[*candidate_index]
            });
            self.matcher_position_cache
                .borrow_mut()
                .insert(key.clone(), positions);
        }
        let position_cache = self.matcher_position_cache.borrow();
        let positions = &position_cache[&key];

        let mut result = Vec::new();
        let mut cursor = 0;
        while cursor < positions.len() {
            let candidate_index = positions[cursor].0;
            let candidate = &self.registered_candidates[candidate_index];
            let mut patterns = Vec::new();
            while cursor < positions.len() && positions[cursor].0 == candidate_index {
                let pattern = candidate.patterns[positions[cursor].1];
                let prefilter = &self.pattern_prefilters[pattern.source];
                if prefix_states_may_start_with(&prefilter.leading, input)
                    && (!prefilter.left_recursive
                        || suffix_states_may_end_with(&prefilter.trailing, input))
                {
                    patterns.push(pattern);
                }
                cursor += 1;
            }
            if !patterns.is_empty() {
                result.push(PatternCandidate {
                    kind: candidate.kind,
                    definition_id: candidate.definition_id.clone(),
                    registration_id: candidate.registration_id.clone(),
                    priority: candidate.priority,
                    registration_order: candidate.registration_order,
                    resolved_order: candidate.resolved_order,
                    patterns,
                });
            }
        }
        result
    }

    fn expected_type_id(&self, expected_types: &[ExpressionExpectedType]) -> usize {
        if let Some(id) = self.expected_type_ids.borrow().get(expected_types) {
            return *id;
        }
        let compatible = self
            .registered_candidates
            .iter()
            .map(|candidate| self.registered_candidate_matches(candidate, expected_types))
            .collect();
        let mut compatibility_cache = self.candidate_compatibility_cache.borrow_mut();
        let id = compatibility_cache.len();
        compatibility_cache.push(compatible);
        self.expected_type_ids
            .borrow_mut()
            .insert(expected_types.to_vec(), id);
        id
    }

    fn registered_node(
        &mut self,
        matched: CandidateMatch,
        frame_start: usize,
        expected_types: &[ExpressionExpectedType],
    ) -> Result<Option<ExpressionCandidate>, ExpressionParseError> {
        let metadata = self
            .registration_metadata(&matched.registration_id)
            .cloned();
        let local = matched.matched.span.local_range;
        let absolute = TextRange::new(frame_start + local.start, frame_start + local.end);
        let span = self.map_range(absolute)?;
        let children: Vec<ExpressionNode> = matched
            .matched
            .captures
            .iter()
            .filter_map(|capture| match capture {
                PatternCapture::TypeExpression {
                    resolution_id: Some(id),
                    ..
                } => self.resolved_nodes.get(id).cloned(),
                PatternCapture::Regex { .. }
                | PatternCapture::TypeExpression {
                    resolution_id: None,
                    ..
                } => None,
            })
            .collect();
        let mut return_type = metadata
            .as_ref()
            .and_then(|value| value.return_type.clone());
        let mut multiplicity = metadata.as_ref().and_then(|value| value.multiplicity);
        let mut node_metadata = metadata
            .as_ref()
            .map_or_else(BTreeMap::new, |value| value.metadata.clone());
        let needs_resolution = metadata.as_ref().is_some_and(|value| {
            value.return_type_state != ReturnTypeState::Static
                || value.multiplicity_state == ResolutionState::Unresolved
        });
        if needs_resolution {
            let value = metadata.as_ref().expect("checked registered metadata");
            let decision = self
                .environment
                .resolve_registered_expression(RegisteredExpressionRequest {
                    input: self.source.virtual_source(),
                    definition_id: &matched.definition_id,
                    registration_id: &matched.registration_id,
                    element_class: &value.element_class,
                    related_property: value.related_property.as_deref(),
                    pattern_index: matched.pattern_index,
                    span: &span,
                    expected_types,
                    declared_return_type: value.return_type.as_ref(),
                    declared_multiplicity: value.multiplicity,
                    return_type_state: value.return_type_state,
                    possible_return_types: &value.possible_return_types,
                    possible_return_types_state: value.possible_return_types_state,
                    captures: &matched.matched.captures,
                    tags: &matched.matched.tags,
                    mark: matched.matched.mark,
                    children: &children,
                    context: &self.context,
                })
                .map_err(|message| ExpressionParseError::Environment { message })?;
            match decision {
                RegisteredExpressionDecision::UseDeclared => {}
                RegisteredExpressionDecision::Resolved {
                    return_type: resolved_return_type,
                    multiplicity: resolved_multiplicity,
                    metadata,
                } => {
                    return_type = resolved_return_type;
                    multiplicity = resolved_multiplicity;
                    node_metadata.extend(metadata);
                }
                RegisteredExpressionDecision::Reject { .. } => {
                    self.environment
                        .finish_registered_expression(false)
                        .map_err(|message| ExpressionParseError::Environment { message })?;
                    return Ok(None);
                }
            }
        }
        let accepted = self.return_type_matches(return_type.as_ref(), expected_types)
            && self.multiplicity_matches(multiplicity, expected_types);
        if needs_resolution {
            self.environment
                .finish_registered_expression(accepted)
                .map_err(|message| ExpressionParseError::Environment { message })?;
        }
        if !accepted {
            return Ok(None);
        }
        Ok(Some(ExpressionCandidate {
            node: ExpressionNode {
                kind: ExpressionNodeKind::Registered {
                    definition_id: matched.definition_id,
                    registration_id: matched.registration_id,
                    pattern_index: matched.pattern_index,
                },
                span,
                return_type,
                multiplicity,
                captures: matched.matched.captures,
                tags: matched.matched.tags,
                mark: matched.matched.mark,
                children,
                metadata: node_metadata,
            },
            expected_alternative: None,
        }))
    }

    fn registration_metadata(&self, registration_id: &str) -> Option<&RegistrationMetadata> {
        self.registrations.get(registration_id)
    }
    fn valid_leaf(
        &self,
        leaf: &ExpressionLeafCandidate,
        range: TextRange,
        candidate_ends: &[usize],
        expected_types: &[ExpressionExpectedType],
        allow_literals: bool,
        allow_expressions: bool,
    ) -> bool {
        range.contains(leaf.range)
            && leaf.range.start == range.start
            && candidate_ends.contains(&leaf.range.end)
            && leaf.range.is_valid_for(self.source.virtual_source())
            && (allow_literals || leaf.kind != ExpressionLeafKind::Literal)
            && (allow_expressions || leaf.kind == ExpressionLeafKind::Literal)
            && self.return_type_matches(leaf.return_type.as_ref(), expected_types)
            && self.multiplicity_matches(leaf.multiplicity, expected_types)
    }

    fn return_type_matches(
        &self,
        return_type: Option<&ClassName>,
        expected_types: &[ExpressionExpectedType],
    ) -> bool {
        expected_types.is_empty()
            || return_type.is_some_and(|return_type| {
                expected_types.iter().any(|expected| {
                    self.catalog
                        .is_class_assignable(return_type.as_str(), expected.class_name.as_str())
                })
            })
    }

    fn multiplicity_matches(
        &self,
        multiplicity: Option<Multiplicity>,
        expected_types: &[ExpressionExpectedType],
    ) -> bool {
        expected_types.is_empty()
            || expected_types.iter().any(|expected| match multiplicity {
                None | Some(Multiplicity::Both | Multiplicity::Single) => true,
                Some(Multiplicity::Multiple) => expected.plural,
            })
    }

    fn validate_prefix_request(
        &self,
        range: TextRange,
        candidate_ends: &[usize],
    ) -> Result<(), ExpressionParseError> {
        if !range.is_valid_for(self.source.virtual_source())
            || candidate_ends.iter().any(|end| {
                *end < range.start
                    || *end > range.end
                    || !self.source.virtual_source().is_char_boundary(*end)
            })
        {
            Err(ExpressionParseError::InvalidInputRange { range })
        } else {
            Ok(())
        }
    }

    fn count_candidate(&mut self) -> Result<(), ExpressionParseError> {
        self.candidates_seen = self.candidates_seen.saturating_add(1);
        if self.candidates_seen > self.config.max_candidates {
            Err(ExpressionParseError::CandidateLimit {
                limit: self.config.max_candidates,
            })
        } else {
            Ok(())
        }
    }

    fn map_range(&self, range: TextRange) -> Result<MatchSpan, ExpressionParseError> {
        let mapped =
            self.source
                .map_range(range)
                .map_err(|error| ExpressionParseError::SourceMap {
                    message: error.to_string(),
                })?;
        Ok(MatchSpan {
            local_range: range,
            mapped,
        })
    }
}

fn registration_metadata_index(
    catalog: &Catalog,
    dynamic_snapshot: Option<&DynamicSyntaxSnapshot>,
) -> HashMap<String, RegistrationMetadata> {
    let mut registrations = catalog
        .expressions()
        .map(|expression| {
            (
                expression.common.registration_id.as_str().to_owned(),
                RegistrationMetadata {
                    element_class: expression.common.element_class.clone(),
                    related_property: expression.common.related_property.clone(),
                    return_type: expression.return_type.clone(),
                    return_type_state: expression.return_type_state,
                    possible_return_types: expression.possible_return_types.clone(),
                    possible_return_types_state: expression.possible_return_types_state,
                    multiplicity: expression.return_type_multiplicity,
                    multiplicity_state: expression.return_type_multiplicity_state,
                    metadata: BTreeMap::new(),
                },
            )
        })
        .collect::<HashMap<_, _>>();
    if let Some(snapshot) = dynamic_snapshot {
        registrations.extend(
            snapshot
                .definitions
                .values()
                .filter(|definition| definition.kind == SyntaxKind::Expression)
                .map(|definition| {
                    (
                        definition.id.qualified(),
                        RegistrationMetadata {
                            element_class: ClassName(definition.id.qualified()),
                            related_property: None,
                            return_type: definition.return_type.clone().map(ClassName),
                            return_type_state: if definition.return_type.is_some() {
                                ReturnTypeState::Static
                            } else {
                                ReturnTypeState::Unresolved
                            },
                            possible_return_types: definition
                                .return_type
                                .iter()
                                .cloned()
                                .map(ClassName)
                                .collect(),
                            possible_return_types_state: if definition.return_type.is_some() {
                                PossibleReturnTypesState::Complete
                            } else {
                                PossibleReturnTypesState::Unresolved
                            },
                            multiplicity: definition.return_multiplicity.map(|value| match value {
                                DynamicMultiplicity::Single => Multiplicity::Single,
                                DynamicMultiplicity::Multiple => Multiplicity::Multiple,
                                DynamicMultiplicity::Both => Multiplicity::Both,
                            }),
                            multiplicity_state: if definition.return_multiplicity.is_some() {
                                ResolutionState::Resolved
                            } else {
                                ResolutionState::Unresolved
                            },
                            metadata: definition.metadata.clone(),
                        },
                    )
                }),
        );
    }
    registrations
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RegisteredPass {
    Base,
    LeftRecursive,
}

#[derive(Debug, Clone)]
struct PrefixState {
    text: String,
    terminal: bool,
}

struct PatternPrefilter {
    left_recursive: bool,
    leading: Vec<PrefixState>,
    trailing: Vec<PrefixState>,
}

type PatternPosition = (usize, usize);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MatcherPositionKey {
    initial: Option<char>,
    expected_type_id: usize,
    registered_pass: RegisteredPass,
}

struct PatternInitialIndex {
    wildcard: Vec<PatternPosition>,
    by_initial: HashMap<char, Vec<PatternPosition>>,
}

fn pattern_prefilter_index<'a>(
    candidates: &[PatternCandidate<'a>],
) -> HashMap<&'a str, PatternPrefilter> {
    let mut result = HashMap::new();
    for pattern in candidates
        .iter()
        .flat_map(|candidate| candidate.patterns.iter())
    {
        result
            .entry(pattern.source)
            .or_insert_with(|| PatternPrefilter {
                left_recursive: pattern_is_left_recursive(&pattern.parsed.elements),
                leading: leading_prefix_states(&pattern.parsed.elements),
                trailing: trailing_suffix_states(&pattern.parsed.elements),
            });
    }
    result
}

fn pattern_initial_index(
    candidates: &[PatternCandidate<'_>],
    prefilters: &HashMap<&str, PatternPrefilter>,
) -> PatternInitialIndex {
    let mut wildcard = Vec::new();
    let mut by_initial = HashMap::<char, Vec<PatternPosition>>::new();
    for (candidate_index, candidate) in candidates.iter().enumerate() {
        for (pattern_index, pattern) in candidate.patterns.iter().enumerate() {
            let position = (candidate_index, pattern_index);
            let mut has_wildcard = false;
            for state in &prefilters[pattern.source].leading {
                let initial = state
                    .text
                    .chars()
                    .find(|character| *character != ' ')
                    .and_then(|character| character.to_lowercase().next());
                if let Some(initial) = initial {
                    by_initial.entry(initial).or_default().push(position);
                } else {
                    has_wildcard = true;
                }
            }
            if has_wildcard {
                wildcard.push(position);
            }
        }
    }
    wildcard.sort_unstable();
    wildcard.dedup();
    for positions in by_initial.values_mut() {
        positions.sort_unstable();
        positions.dedup();
    }
    PatternInitialIndex {
        wildcard,
        by_initial,
    }
}

fn pattern_is_left_recursive(elements: &[SpannedPatternElement]) -> bool {
    sequence_start(elements).0
}

fn sequence_start(elements: &[SpannedPatternElement]) -> (bool, bool) {
    let mut can_be_empty = true;
    for element in elements {
        let (begins_with_type, element_empty) = element_start(&element.value);
        if begins_with_type {
            return (true, false);
        }
        if !element_empty {
            return (false, false);
        }
        can_be_empty &= element_empty;
    }
    (false, can_be_empty)
}

fn element_start(element: &PatternElement) -> (bool, bool) {
    match element {
        PatternElement::TypeExpr(_) => (true, false),
        PatternElement::Literal(_) | PatternElement::Regex(_) => (false, false),
        PatternElement::Group(children) => sequence_start(children),
        PatternElement::Option(children) => (sequence_start(children).0, true),
        PatternElement::Choice(branches) => {
            let starts = branches.iter().map(|branch| sequence_start(branch));
            let mut begins_with_type = false;
            let mut can_be_empty = false;
            for (branch_type, branch_empty) in starts {
                begins_with_type |= branch_type;
                can_be_empty |= branch_empty;
            }
            (begins_with_type, can_be_empty)
        }
        PatternElement::ParseTag(_) | PatternElement::ParseMark(_) | PatternElement::Empty => {
            (false, true)
        }
    }
}

fn prefix_states_may_start_with(states: &[PrefixState], input: &str) -> bool {
    states.iter().any(|state| {
        state.text.is_empty()
            || (state.text.bytes().any(|value| value != b' ')
                && starts_with_skript_literal(input, &state.text))
    })
}

fn suffix_states_may_end_with(states: &[PrefixState], input: &str) -> bool {
    states.iter().any(|state| {
        state.text.is_empty()
            || (state.text.bytes().any(|value| value != b' ')
                && ends_with_skript_literal(input, &state.text))
    })
}

fn trailing_suffix_states(elements: &[SpannedPatternElement]) -> Vec<PrefixState> {
    let mut states = vec![PrefixState {
        text: String::new(),
        terminal: false,
    }];
    for element in elements.iter().rev() {
        let mut next = Vec::new();
        for state in states {
            if state.terminal {
                next.push(state);
                continue;
            }
            match &element.value {
                PatternElement::Literal(value) => {
                    let mut text = value.clone();
                    text.push_str(&state.text);
                    next.push(PrefixState {
                        text,
                        terminal: false,
                    });
                }
                PatternElement::Regex(_) | PatternElement::TypeExpr(_) => {
                    let mut state = state;
                    state.terminal = true;
                    next.push(state);
                }
                PatternElement::Group(children) => {
                    prepend_suffix_states(&state, trailing_suffix_states(children), &mut next);
                }
                PatternElement::Option(children) => {
                    next.push(state.clone());
                    prepend_suffix_states(&state, trailing_suffix_states(children), &mut next);
                }
                PatternElement::Choice(branches) => {
                    for branch in branches {
                        prepend_suffix_states(&state, trailing_suffix_states(branch), &mut next);
                    }
                }
                PatternElement::ParseTag(_)
                | PatternElement::ParseMark(_)
                | PatternElement::Empty => next.push(state),
            }
        }
        if next.len() > 256 {
            return vec![PrefixState {
                text: String::new(),
                terminal: true,
            }];
        }
        states = next;
    }
    states
}

fn prepend_suffix_states(
    parent: &PrefixState,
    children: Vec<PrefixState>,
    output: &mut Vec<PrefixState>,
) {
    for child in children {
        let mut text = child.text;
        text.push_str(&parent.text);
        output.push(PrefixState {
            text,
            terminal: child.terminal,
        });
    }
}

fn leading_prefix_states(elements: &[SpannedPatternElement]) -> Vec<PrefixState> {
    let mut states = vec![PrefixState {
        text: String::new(),
        terminal: false,
    }];
    for element in elements {
        let mut next = Vec::new();
        for state in states {
            if state.terminal {
                next.push(state);
                continue;
            }
            match &element.value {
                PatternElement::Literal(value) => {
                    let mut state = state;
                    state.text.push_str(value);
                    next.push(state);
                }
                PatternElement::Regex(_) | PatternElement::TypeExpr(_) => {
                    let mut state = state;
                    state.terminal = true;
                    next.push(state);
                }
                PatternElement::Group(children) => {
                    append_prefix_states(&state, leading_prefix_states(children), &mut next);
                }
                PatternElement::Option(children) => {
                    next.push(state.clone());
                    append_prefix_states(&state, leading_prefix_states(children), &mut next);
                }
                PatternElement::Choice(branches) => {
                    for branch in branches {
                        append_prefix_states(&state, leading_prefix_states(branch), &mut next);
                    }
                }
                PatternElement::ParseTag(_)
                | PatternElement::ParseMark(_)
                | PatternElement::Empty => next.push(state),
            }
        }
        if next.len() > 256 {
            return vec![PrefixState {
                text: String::new(),
                terminal: true,
            }];
        }
        states = next;
    }
    states
}

fn append_prefix_states(
    parent: &PrefixState,
    children: Vec<PrefixState>,
    output: &mut Vec<PrefixState>,
) {
    for child in children {
        let mut text = parent.text.clone();
        text.push_str(&child.text);
        output.push(PrefixState {
            text,
            terminal: child.terminal,
        });
    }
}

fn ends_with_skript_literal(input: &str, suffix: &str) -> bool {
    let mut cursor = input.len();
    for expected in suffix.chars().rev() {
        if expected == ' ' {
            if cursor == 0 || cursor == input.len() {
                continue;
            }
            if input.as_bytes().get(cursor - 1) == Some(&b' ') {
                cursor -= 1;
                continue;
            }
            if input.as_bytes().get(cursor) == Some(&b' ') {
                continue;
            }
            return false;
        }
        let Some((index, actual)) = input[..cursor].char_indices().next_back() else {
            return false;
        };
        if !chars_equal_ignore_case(expected, actual) {
            return false;
        }
        cursor = index;
    }
    true
}

fn starts_with_skript_literal(input: &str, prefix: &str) -> bool {
    let mut cursor = 0;
    for expected in prefix.chars() {
        if expected == ' ' {
            if cursor == 0 || cursor == input.len() {
                continue;
            }
            if input.as_bytes().get(cursor) == Some(&b' ') {
                cursor += 1;
                continue;
            }
            if input.as_bytes().get(cursor - 1) == Some(&b' ') {
                continue;
            }
            return false;
        }
        let Some(actual) = input[cursor..].chars().next() else {
            return false;
        };
        if !chars_equal_ignore_case(expected, actual) {
            return false;
        }
        cursor += actual.len_utf8();
    }
    true
}

fn chars_equal_ignore_case(left: char, right: char) -> bool {
    left == right || left.to_lowercase().eq(right.to_lowercase())
}

impl<E: ExpressionParseEnvironment> PatternMatchEnvironment for ExpressionSession<'_, E> {
    fn begin_pattern_match(&mut self) -> Result<(), String> {
        self.environment.begin_pattern_match()
    }

    fn finish_pattern_match(&mut self, accepted: bool) -> Result<(), String> {
        self.environment.finish_pattern_match(accepted)
    }

    fn allows_regex_pattern(
        &mut self,
        kind: crate::MatchSyntaxKind,
        registration_id: &str,
    ) -> Result<bool, String> {
        self.environment.allows_regex_pattern(kind, registration_id)
    }

    fn resolve_type(
        &mut self,
        request: TypeExpressionRequest<'_>,
    ) -> Result<Vec<TypeExpressionResolution>, String> {
        self.resolve_type_inner(request)
            .map_err(|error| error.to_string())
    }

    fn dispatch_hook(&mut self, event: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
        self.environment.dispatch_hook(event)
    }
}

impl<E: ExpressionParseEnvironment> ExpressionSession<'_, E> {
    fn resolve_type_inner(
        &mut self,
        request: TypeExpressionRequest<'_>,
    ) -> Result<Vec<TypeExpressionResolution>, ExpressionParseError> {
        let frame_start = *self
            .frame_starts
            .last()
            .expect("typed expressions are resolved inside a matcher frame");
        let depth = self.frame_depths.last().copied().unwrap_or(0) + 1;
        let remaining = TextRange::new(
            frame_start + request.remaining.start,
            frame_start + request.remaining.end,
        );
        let candidate_ends = request
            .candidate_ends
            .iter()
            .map(|end| frame_start + *end)
            .collect::<Vec<_>>();
        let mut resolutions = Vec::new();
        if request.expression.nullable && candidate_ends.contains(&remaining.start) {
            resolutions.push(TypeExpressionResolution {
                range: TextRange::empty(request.remaining.start),
                alternative_index: None,
                resolution_id: None,
            });
        }
        for (alternative_index, alternative) in request.expression.alternatives.iter().enumerate() {
            let Some(value_type) = self.catalog.type_by_code_name(&alternative.name) else {
                continue;
            };
            let expected = [ExpressionExpectedType {
                class_name: value_type.original_class.clone(),
                plural: alternative.plural,
            }];
            let candidates = self.parse_prefixes(
                remaining,
                &candidate_ends,
                &expected,
                request.expression.allow_literals,
                request.expression.allow_expressions,
                request.expression.time,
                depth,
            )?;
            for mut candidate in candidates {
                candidate.expected_alternative = Some(alternative_index);
                let absolute = candidate.node.span.local_range;
                let id = format!("expression:{}", self.next_resolution_id);
                self.next_resolution_id = self.next_resolution_id.saturating_add(1);
                self.resolved_nodes.insert(id.clone(), candidate.node);
                resolutions.push(TypeExpressionResolution {
                    range: TextRange::new(absolute.start - frame_start, absolute.end - frame_start),
                    alternative_index: Some(alternative_index),
                    resolution_id: Some(id),
                });
            }
        }
        Ok(resolutions)
    }
}

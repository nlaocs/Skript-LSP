//! Recursive Expression parsing over SSG registrations and parser extensions.
//!
//! The parser owns candidate ordering, type filtering, recursion limits, and
//! memoization. CoreLibrary and addon Components provide leaf expressions such
//! as variables and literals through [`ExpressionParseEnvironment`].
#![allow(missing_docs)] // Aggregate contracts are documented on their owning types.

use crate::{
    CandidateMatch, MappedSource, MatchInput, MatchSpan, PatternCandidate, PatternCapture,
    PatternHookControl, PatternHookEvent, PatternMatchEnvironment, PatternMatchError,
    PatternMatcherConfig, TextRange, TypeExpressionRequest, TypeExpressionResolution,
    catalog_pattern_candidates, match_pattern_candidates_with_environment,
    snapshot_pattern_candidates,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use syntax_pattern_parser::syntax::{PatternElement, SpannedPatternElement};
use syntaxes::{
    Catalog, ClassName, DynamicMultiplicity, DynamicSyntaxSnapshot, Multiplicity, SyntaxKind,
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
    range: TextRange,
    candidate_ends: Vec<usize>,
    expected_types: Vec<ExpressionExpectedType>,
    allow_literals: bool,
    allow_expressions: bool,
    time: i32,
    context: ExpressionParseContext,
    state_revision: u64,
    registry_revision: u64,
}

#[derive(Debug, Clone)]
struct RegistrationMetadata {
    return_type: Option<ClassName>,
    multiplicity: Option<Multiplicity>,
    metadata: BTreeMap<String, String>,
}

pub(crate) struct ExpressionSession<'a, E> {
    catalog: &'a Catalog,
    registered_candidates: Vec<PatternCandidate<'a>>,
    registry_revision: u64,
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

impl<E: ExpressionParseEnvironment> ExpressionSession<'_, E> {
    pub(crate) fn new<'a>(
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
        ExpressionSession {
            catalog,
            registered_candidates,
            registry_revision: dynamic_snapshot.map_or(0, |snapshot| snapshot.registry_revision),
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
        let key = MemoKey {
            range,
            candidate_ends: candidate_ends.to_vec(),
            expected_types: expected_types.to_vec(),
            allow_literals,
            allow_expressions,
            time,
            context: self.context.clone(),
            state_revision: self
                .environment
                .state_revision()
                .map_err(|message| ExpressionParseError::Environment { message })?,
            registry_revision: self.registry_revision,
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
            let matcher_candidates = self
                .registered_candidates
                .iter()
                .filter(|candidate| self.registered_candidate_matches(candidate, expected_types))
                .filter_map(|candidate| {
                    let patterns = candidate
                        .patterns
                        .iter()
                        .copied()
                        .filter(|pattern| {
                            let left_recursive =
                                pattern_is_left_recursive(&pattern.parsed.elements);
                            left_recursive
                                == matches!(registered_pass, RegisteredPass::LeftRecursive)
                                && pattern_may_start_with(&pattern.parsed.elements, input)
                                && (!left_recursive
                                    || pattern_may_end_with(&pattern.parsed.elements, input))
                        })
                        .collect::<Vec<_>>();
                    (!patterns.is_empty()).then(|| PatternCandidate {
                        kind: candidate.kind,
                        definition_id: candidate.definition_id.clone(),
                        registration_id: candidate.registration_id.clone(),
                        priority: candidate.priority,
                        registration_order: candidate.registration_order,
                        resolved_order: candidate.resolved_order,
                        patterns,
                    })
                })
                .collect::<Vec<_>>();
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
                if let Some(selected) = matched.selected {
                    candidates.push(self.registered_node(selected, candidate_range.start)?);
                }
                for alternative in matched.alternatives {
                    candidates.push(self.registered_node(alternative, candidate_range.start)?);
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
                self.return_type_matches(metadata.return_type.as_ref(), expected_types)
                    && self.multiplicity_matches(metadata.multiplicity, expected_types)
            })
    }

    fn registered_node(
        &self,
        matched: CandidateMatch,
        frame_start: usize,
    ) -> Result<ExpressionCandidate, ExpressionParseError> {
        let metadata = self.registration_metadata(&matched.registration_id);
        let local = matched.matched.span.local_range;
        let absolute = TextRange::new(frame_start + local.start, frame_start + local.end);
        let children = matched
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
        Ok(ExpressionCandidate {
            node: ExpressionNode {
                kind: ExpressionNodeKind::Registered {
                    definition_id: matched.definition_id,
                    registration_id: matched.registration_id,
                    pattern_index: matched.pattern_index,
                },
                span: self.map_range(absolute)?,
                return_type: metadata
                    .as_ref()
                    .and_then(|value| value.return_type.clone()),
                multiplicity: metadata.as_ref().and_then(|value| value.multiplicity),
                captures: matched.matched.captures,
                children,
                metadata: metadata.map_or_else(BTreeMap::new, |value| value.metadata.clone()),
            },
            expected_alternative: None,
        })
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
                    return_type: expression.return_type.clone(),
                    multiplicity: expression.return_type_multiplicity,
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
                            return_type: definition.return_type.clone().map(ClassName),
                            multiplicity: definition.return_multiplicity.map(|value| match value {
                                DynamicMultiplicity::Single => Multiplicity::Single,
                                DynamicMultiplicity::Multiple => Multiplicity::Multiple,
                                DynamicMultiplicity::Both => Multiplicity::Both,
                            }),
                            metadata: definition.metadata.clone(),
                        },
                    )
                }),
        );
    }
    registrations
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegisteredPass {
    Base,
    LeftRecursive,
}

#[derive(Debug, Clone)]
struct PrefixState {
    text: String,
    terminal: bool,
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

fn pattern_may_start_with(elements: &[SpannedPatternElement], input: &str) -> bool {
    leading_prefix_states(elements)
        .into_iter()
        .any(|state| state.text.is_empty() || starts_with_ignore_case(input, &state.text))
}

fn pattern_may_end_with(elements: &[SpannedPatternElement], input: &str) -> bool {
    trailing_suffix_states(elements)
        .into_iter()
        .any(|state| state.text.is_empty() || ends_with_ignore_case(input, &state.text))
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

fn ends_with_ignore_case(input: &str, suffix: &str) -> bool {
    let mut input = input.chars().rev();
    suffix.chars().rev().all(|expected| {
        input.next().is_some_and(|actual| {
            actual == expected || actual.to_lowercase().eq(expected.to_lowercase())
        })
    })
}

fn starts_with_ignore_case(input: &str, prefix: &str) -> bool {
    let mut input = input.chars();
    prefix.chars().all(|expected| {
        input.next().is_some_and(|actual| {
            actual == expected || actual.to_lowercase().eq(expected.to_lowercase())
        })
    })
}

impl<E: ExpressionParseEnvironment> PatternMatchEnvironment for ExpressionSession<'_, E> {
    fn begin_pattern_match(&mut self) -> Result<(), String> {
        self.environment.begin_pattern_match()
    }

    fn finish_pattern_match(&mut self, accepted: bool) -> Result<(), String> {
        self.environment.finish_pattern_match(accepted)
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

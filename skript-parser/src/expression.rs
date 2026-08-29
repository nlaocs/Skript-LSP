//! Recursive Expression parsing over SSG registrations and parser extensions.
//!
//! The parser owns candidate ordering, type filtering, recursion limits, and
//! memoization. CoreLibrary and addon Components provide leaf expressions such
//! as variables and literals through [`ExpressionParseEnvironment`].
#![allow(missing_docs)] // Aggregate contracts are documented on their owning types.

use crate::expression_list::{ExpressionListConjunction, split_expression_list};
use crate::pattern_match::{
    find_parenthesis_end, find_quote_end, find_variable_end, java_trim_range,
};
use crate::{
    CandidateFailure, CandidateMatch, ConditionNode, FailureFrame, FailureFrameRole, FailureTrace,
    MappedSource, MatchInput, MatchPattern, MatchSpan, ParseTagCapture, PatternCandidate,
    PatternCapture, PatternFailure, PatternFailureReason, PatternHookControl, PatternHookEvent,
    PatternMatchEnvironment, PatternMatchError, PatternMatcherConfig, TextRange,
    TypeExpressionOutcome, TypeExpressionRequest, TypeExpressionResolution,
    catalog_pattern_candidates, choose_failure_trace, match_pattern_candidates_with_environment,
    snapshot_pattern_candidates,
};
use crate::{FunctionCall, FunctionDefinition, FunctionLookupRequest};
use std::borrow::Cow;
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
    /// A complete child Expression wrapped in one pair of parentheses.
    Grouped,
    /// A top-level Skript Expression list joined by commas, `and`, `or`, or `nor`.
    List {
        conjunction: ExpressionListConjunction,
    },
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
    Arithmetic {
        operator: String,
        operation_registration_id: String,
    },
    Custom {
        parser_id: String,
    },
}

/// Parsed Expression node with nested typed captures and mapped provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpressionNode {
    pub kind: ExpressionNodeKind,
    pub function: Option<FunctionCall>,
    pub span: MatchSpan,
    pub return_type: Option<ClassName>,
    pub multiplicity: Option<Multiplicity>,
    pub captures: Vec<PatternCapture>,
    pub tags: Vec<ParseTagCapture>,
    pub mark: i32,
    pub children: Vec<ExpressionNode>,
    /// Non-Expression captures resolved through open parser IDs.
    pub(crate) routed_captures: Vec<ParsedCapture>,
    pub metadata: BTreeMap<String, String>,
}

/// Parser ID used by the built-in recursive Expression route.
pub const HOST_EXPRESSION_PARSER_ID: &str = "host.expression";
/// Parser ID used by the built-in recursive Condition route.
pub const HOST_CONDITION_PARSER_ID: &str = "host.condition";
/// Parser ID used by the built-in recursive Effect route.
pub const HOST_EFFECT_PARSER_ID: &str = "host.effect";

/// A parser binding for one registration capture.
///
/// Parser IDs are intentionally open strings. Built-in routes use the
/// `host.*` IDs above, while addons may provide their own route without
/// requiring a new Rust enum variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredCaptureBinding {
    pub capture_index: usize,
    pub parser_id: String,
    pub required: bool,
    pub options: BTreeMap<String, String>,
}

/// Stable SSG identity of one registered syntax pattern.
///
/// Semantic routing uses these identifiers exclusively. A WASM host may use a
/// Java class suffix while loading a legacy manifest, but resolves that suffix
/// to these identifiers before parsing begins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisteredSyntaxIdentity<'a> {
    pub kind: SyntaxKind,
    pub definition_id: &'a str,
    pub registration_id: &'a str,
    pub pattern_index: Option<usize>,
}

/// Completeness of one generic capture result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedCaptureStatus {
    Success,
    Partial,
    Failed,
}

/// Optional semantic information attached to a generic capture result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCaptureSemanticSummary {
    pub kind: String,
    pub definition_id: Option<String>,
    pub registration_id: Option<String>,
    pub element_class: Option<ClassName>,
    pub pattern_index: Option<usize>,
    pub return_type: Option<ClassName>,
    pub multiplicity: Option<Multiplicity>,
    pub metadata: BTreeMap<String, String>,
}

/// Open diagnostic payload returned by a native parser or an addon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCaptureDiagnostic {
    pub message: String,
    pub span: Option<MatchSpan>,
    pub metadata: BTreeMap<String, String>,
}

/// Opaque addon-owned data attached to a generic parse result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCaptureAttachment {
    pub owner_component_id: String,
    pub schema_id: String,
    pub schema_version: u32,
    pub encoding: String,
    pub bytes: Vec<u8>,
}

/// Native values that can be carried by a generic parsed capture.
///
/// Recursive candidate values are boxed so nested Effect and Section parses
/// remain finite while retaining their complete native result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedCaptureValue {
    Expression(ExpressionNode),
    Condition(ConditionNode),
    Effect(Box<crate::EffectCandidate>),
    Section(Box<crate::SectionCandidate>),
    Raw(String),
}

/// Result of routing one registration capture through a parser ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCaptureResult {
    pub parser_id: String,
    pub status: ParsedCaptureStatus,
    pub span: MatchSpan,
    pub summary: Option<ParsedCaptureSemanticSummary>,
    pub value: Option<ParsedCaptureValue>,
    pub diagnostics: Vec<ParsedCaptureDiagnostic>,
    pub attachments: Vec<ParsedCaptureAttachment>,
}

/// One ordered binding/result pair exposed by a syntax candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCapture {
    pub capture_index: usize,
    pub binding: RegisteredCaptureBinding,
    pub result: ParsedCaptureResult,
}

impl ParsedCaptureResult {
    pub(crate) fn success(
        parser_id: impl Into<String>,
        span: MatchSpan,
        summary: Option<ParsedCaptureSemanticSummary>,
        value: ParsedCaptureValue,
    ) -> Self {
        Self {
            parser_id: parser_id.into(),
            status: ParsedCaptureStatus::Success,
            span,
            summary,
            value: Some(value),
            diagnostics: Vec::new(),
            attachments: Vec::new(),
        }
    }

    pub(crate) fn failure(
        parser_id: impl Into<String>,
        span: MatchSpan,
        message: impl Into<String>,
    ) -> Self {
        Self {
            parser_id: parser_id.into(),
            status: ParsedCaptureStatus::Failed,
            span: span.clone(),
            summary: None,
            value: None,
            diagnostics: vec![ParsedCaptureDiagnostic {
                message: message.into(),
                span: Some(span),
                metadata: BTreeMap::new(),
            }],
            attachments: Vec::new(),
        }
    }
}

impl ExpressionNode {
    /// Rewrites this node and every nested Expression/Condition span.
    pub fn try_map_spans<E>(
        &mut self,
        mapper: &mut impl FnMut(&MatchSpan) -> Result<MatchSpan, E>,
    ) -> Result<(), E> {
        self.span = mapper(&self.span)?;
        try_map_pattern_captures(&mut self.captures, mapper)?;
        for tag in &mut self.tags {
            tag.input_span = mapper(&tag.input_span)?;
        }
        for child in &mut self.children {
            child.try_map_spans(mapper)?;
        }
        for capture in &mut self.routed_captures {
            capture.result.span = mapper(&capture.result.span)?;
            for diagnostic in &mut capture.result.diagnostics {
                if let Some(span) = diagnostic.span.as_mut() {
                    *span = mapper(span)?;
                }
            }
            match capture.result.value.as_mut() {
                Some(ParsedCaptureValue::Expression(node)) => node.try_map_spans(mapper)?,
                Some(ParsedCaptureValue::Condition(node)) => try_map_condition_spans(node, mapper)?,
                _ => {}
            }
        }
        Ok(())
    }

    /// Iterates recursively parsed Condition captures.
    pub fn conditions(&self) -> impl Iterator<Item = &ConditionNode> {
        self.routed_captures.iter().filter_map(|capture| {
            if let Some(ParsedCaptureValue::Condition(condition)) = capture.result.value.as_ref() {
                Some(condition)
            } else {
                None
            }
        })
    }

    /// Returns all recursively parsed captures in source/pattern order.
    ///
    /// Typed children and routed captures use distinct compact storage
    /// internally; this method presents both through one parser-neutral view.
    pub fn parsed_captures(&self) -> Vec<ParsedCapture> {
        let mut children = self.children.iter();
        let routed = self.routed_captures.iter();
        let mut captures = Vec::new();
        if self.captures.is_empty() {
            return self
                .children
                .iter()
                .cloned()
                .enumerate()
                .map(|(capture_index, node)| expression_parsed_capture(capture_index, node))
                .collect();
        }
        for (capture_index, capture) in self.captures.iter().enumerate() {
            match capture {
                PatternCapture::TypeExpression {
                    resolution_id: Some(_),
                    span,
                    ..
                } => {
                    if let Some(node) = children.next() {
                        captures.push(expression_parsed_capture(capture_index, node.clone()));
                    }
                    let _ = span;
                }
                PatternCapture::Regex { .. } => {
                    if let Some(capture) = routed
                        .clone()
                        .find(|capture| capture.capture_index == capture_index)
                    {
                        captures.push(capture.clone());
                    }
                }
                PatternCapture::TypeExpression {
                    resolution_id: None,
                    ..
                } => {}
            }
        }
        captures
    }
}

fn try_map_pattern_captures<E>(
    captures: &mut [PatternCapture],
    mapper: &mut impl FnMut(&MatchSpan) -> Result<MatchSpan, E>,
) -> Result<(), E> {
    for capture in captures {
        match capture {
            PatternCapture::Regex { span, groups, .. } => {
                *span = mapper(span)?;
                for group in groups {
                    if let Some(span) = group.span.as_mut() {
                        *span = mapper(span)?;
                    }
                }
            }
            PatternCapture::TypeExpression { span, .. } => *span = mapper(span)?,
        }
    }
    Ok(())
}

fn try_map_condition_spans<E>(
    node: &mut ConditionNode,
    mapper: &mut impl FnMut(&MatchSpan) -> Result<MatchSpan, E>,
) -> Result<(), E> {
    node.span = mapper(&node.span)?;
    try_map_pattern_captures(&mut node.captures, mapper)?;
    for tag in &mut node.tags {
        tag.input_span = mapper(&tag.input_span)?;
    }
    for mark in &mut node.marks {
        mark.input_span = mapper(&mark.input_span)?;
    }
    for expression in &mut node.expressions {
        expression.try_map_spans(mapper)?;
    }
    for child in &mut node.children {
        try_map_condition_spans(child, mapper)?;
    }
    Ok(())
}

fn expression_semantic_summary(node: &ExpressionNode) -> ParsedCaptureSemanticSummary {
    let (kind, definition_id, registration_id, pattern_index) = match &node.kind {
        ExpressionNodeKind::Grouped => ("grouped-expression", None, None, None),
        ExpressionNodeKind::List { .. } => ("expression-list", None, None, None),
        ExpressionNodeKind::Registered {
            definition_id,
            registration_id,
            pattern_index,
        } => (
            "registered-expression",
            Some(definition_id.clone()),
            Some(registration_id.clone()),
            Some(*pattern_index),
        ),
        ExpressionNodeKind::Variable { .. } => ("variable", None, None, None),
        ExpressionNodeKind::Literal { .. } => ("literal", None, None, None),
        ExpressionNodeKind::Function { .. } => ("function", None, None, None),
        ExpressionNodeKind::Arithmetic { .. } => ("arithmetic", None, None, None),
        ExpressionNodeKind::Custom { .. } => ("custom", None, None, None),
    };
    ParsedCaptureSemanticSummary {
        kind: kind.to_owned(),
        definition_id,
        registration_id,
        element_class: None,
        pattern_index,
        return_type: node.return_type.clone(),
        multiplicity: node.multiplicity,
        metadata: node.metadata.clone(),
    }
}

pub(crate) fn expression_parsed_capture(
    capture_index: usize,
    node: ExpressionNode,
) -> ParsedCapture {
    let span = node.span.clone();
    ParsedCapture {
        capture_index,
        binding: RegisteredCaptureBinding {
            capture_index,
            parser_id: HOST_EXPRESSION_PARSER_ID.to_owned(),
            required: true,
            options: BTreeMap::new(),
        },
        result: ParsedCaptureResult::success(
            HOST_EXPRESSION_PARSER_ID,
            span,
            Some(expression_semantic_summary(&node)),
            ParsedCaptureValue::Expression(node),
        ),
    }
}

pub(crate) fn condition_parsed_capture(capture_index: usize, node: ConditionNode) -> ParsedCapture {
    let span = node.span.clone();
    let (definition_id, registration_id, pattern_index) = match &node.kind {
        crate::condition::ConditionNodeKind::Registered {
            definition_id,
            registration_id,
            pattern_index,
        } => (
            Some(definition_id.clone()),
            Some(registration_id.clone()),
            Some(*pattern_index),
        ),
        crate::condition::ConditionNodeKind::Grouped => (None, None, None),
    };
    ParsedCapture {
        capture_index,
        binding: RegisteredCaptureBinding {
            capture_index,
            parser_id: HOST_CONDITION_PARSER_ID.to_owned(),
            required: true,
            options: BTreeMap::new(),
        },
        result: ParsedCaptureResult::success(
            HOST_CONDITION_PARSER_ID,
            span,
            Some(ParsedCaptureSemanticSummary {
                kind: "condition".to_owned(),
                definition_id,
                registration_id,
                element_class: None,
                pattern_index,
                return_type: None,
                multiplicity: None,
                metadata: node.metadata.clone(),
            }),
            ParsedCaptureValue::Condition(node),
        ),
    }
}

/// Selected Section identity supplied around recursive child parsing.
pub struct SectionChildrenRequest<'a> {
    pub input: &'a str,
    pub raw_node_id: crate::RawNodeId,
    pub definition_id: &'a str,
    pub registration_id: &'a str,
    pub element_class: Option<&'a ClassName>,
    pub pattern_index: usize,
    pub span: &'a MatchSpan,
    pub loop_section: bool,
    pub effect_section: bool,
    pub section_expression: bool,
    pub captures: &'a [PatternCapture],
    pub parsed_captures: &'a [ParsedCapture],
    pub context: &'a ExpressionParseContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionChildrenDecision {
    Accept(ExpressionParseContext),
    Reject { reason: String },
}

/// One valid Expression candidate in deterministic parser order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpressionCandidate {
    pub node: ExpressionNode,
    pub expected_alternative: Option<usize>,
}

impl ExpressionCandidate {
    /// Returns the candidate's recursively parsed captures as one collection.
    pub fn parsed_captures(&self) -> Vec<ParsedCapture> {
        self.node.parsed_captures()
    }
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
    /// Most specific reason found for the failed complete parse.
    pub kind: ExpressionFailureKind,
    /// Primary source location to underline in a diagnostic.
    pub span: MatchSpan,
    /// Related opening delimiter, when the primary location is elsewhere.
    pub related_span: Option<MatchSpan>,
    pub expected_types: Vec<ExpressionExpectedType>,
}

/// Syntactic reason selected for an unsuccessful complete Expression parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpressionFailureKind {
    /// No candidate matched and no more specific parenthesis error was found.
    ExpectedExpression,
    /// A complete parenthesized group contained no Expression.
    EmptyGroup,
    /// An opening parenthesis had no corresponding closing parenthesis.
    UnclosedParenthesis,
    /// A closing parenthesis had no corresponding opening parenthesis.
    UnexpectedClosingParenthesis,
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
    /// Expressions parsed through host requests and owned by this leaf.
    pub children: Vec<ExpressionNode>,
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
    pub pattern: &'a str,
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
    pub parsed_captures: &'a [ParsedCapture],
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

    /// Returns document- or project-defined Functions visible before catalog globals.
    ///
    /// Definitions with the same parameter shape shadow catalog definitions.
    /// The default keeps existing environments catalog-only.
    fn lookup_functions(
        &mut self,
        _request: FunctionLookupRequest<'_>,
    ) -> Result<Vec<FunctionDefinition>, String> {
        Ok(Vec::new())
    }

    /// Returns whether a semantic handler may replace this registration's
    /// declared return type after its captures have matched.
    fn can_resolve_registered_expression(&self, _syntax: RegisteredSyntaxIdentity<'_>) -> bool {
        false
    }

    /// Declares how regex captures for one registered syntax are recursively parsed.
    fn registered_capture_bindings(
        &self,
        _syntax: RegisteredSyntaxIdentity<'_>,
    ) -> Result<Vec<RegisteredCaptureBinding>, String> {
        Ok(Vec::new())
    }

    /// Starts speculative work for semantic regex captures of one candidate.
    fn begin_semantic_candidate(&mut self) -> Result<(), String> {
        Ok(())
    }

    /// Keeps only semantic capture work belonging to the selected candidate.
    fn finish_semantic_candidate(&mut self, accepted: bool) -> Result<(), String> {
        let _ = accepted;
        Ok(())
    }

    /// Pushes parser context immediately before a selected Section's children.
    fn enter_section_children(
        &mut self,
        request: SectionChildrenRequest<'_>,
    ) -> Result<SectionChildrenDecision, String> {
        Ok(SectionChildrenDecision::Accept(request.context.clone()))
    }

    /// Pops parser context after all children of a selected Section were visited.
    fn exit_section_children(
        &mut self,
        _request: SectionChildrenRequest<'_>,
    ) -> Result<(), String> {
        Ok(())
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
    ) -> Result<TypeExpressionOutcome, String> {
        Ok(TypeExpressionOutcome::default())
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
    allow_lists: bool,
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

#[derive(Debug, Clone, Default)]
pub(crate) struct PrefixParse {
    pub(crate) candidates: Vec<ExpressionCandidate>,
    pub(crate) failure: Option<FailureTrace>,
}

enum RegisteredNodeResolution {
    Accepted(ExpressionCandidate),
    Rejected(Option<FailureTrace>),
}

enum EventApplicability {
    Match,
    NoMatch {
        supported: Vec<String>,
        current: Vec<String>,
    },
    Unknown,
}

pub(crate) struct ExpressionSession<'a, E> {
    catalog: &'a Catalog,
    dynamic_snapshot: Option<&'a DynamicSyntaxSnapshot>,
    registered_candidates: Vec<PatternCandidate<'a>>,
    syntax_candidate_templates: HashMap<SyntaxKind, Vec<PatternCandidate<'a>>>,
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
    memo: HashMap<MemoKey, PrefixParse>,
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
        let (kind, primary, related) =
            expression_failure_ranges(request.source.virtual_source(), request.range);
        Some(ExpressionFailure {
            kind,
            span: session.map_range(primary)?,
            related_span: related.map(|range| session.map_range(range)).transpose()?,
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
            dynamic_snapshot,
            registered_candidates,
            syntax_candidate_templates: HashMap::new(),
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

    pub(crate) fn match_candidates_at_depth(
        &mut self,
        range: TextRange,
        candidates: &[PatternCandidate<'_>],
        depth: usize,
    ) -> Result<crate::CandidateMatches, ExpressionParseError> {
        self.ensure_depth(depth)?;
        if !range.is_valid_for(self.source.virtual_source()) {
            return Err(ExpressionParseError::InvalidInputRange { range });
        }
        let input = MatchInput::from_source(self.source, range)?;
        self.frame_starts.push(range.start);
        self.frame_depths.push(depth);
        let matcher_config = self.config.matcher.clone();
        let matched =
            match_pattern_candidates_with_environment(input, candidates, self, matcher_config);
        self.frame_depths.pop();
        self.frame_starts.pop();
        Ok(matched?)
    }

    pub(crate) fn recover_candidate_failures_at_depth(
        &mut self,
        range: TextRange,
        candidates: &[PatternCandidate<'_>],
        depth: usize,
    ) -> Result<Option<CandidateFailure>, ExpressionParseError> {
        self.ensure_depth(depth)?;
        if !range.is_valid_for(self.source.virtual_source()) {
            return Err(ExpressionParseError::InvalidInputRange { range });
        }
        let input = MatchInput::from_source(self.source, range)?;
        self.begin_semantic_candidate()
            .map_err(|message| ExpressionParseError::Environment { message })?;
        let first_resolution = self.next_resolution_id;
        self.frame_starts.push(range.start);
        self.frame_depths.push(depth);
        let mut matcher_config = self.config.matcher.clone();
        matcher_config.recover_type_expression_failures = true;
        matcher_config.max_candidate_failures = 1;
        let matched =
            match_pattern_candidates_with_environment(input, candidates, self, matcher_config);
        self.frame_depths.pop();
        self.frame_starts.pop();
        for id in first_resolution..self.next_resolution_id {
            self.resolved_nodes.remove(&format!("expression:{id}"));
        }
        self.next_resolution_id = first_resolution;
        let rollback = self
            .finish_semantic_candidate(false)
            .map_err(|message| ExpressionParseError::Environment { message });
        let recovered = matched?.failures.candidates.into_iter().next();
        rollback?;
        Ok(recovered)
    }

    pub(crate) fn retain_viable_patterns(
        &mut self,
        range: TextRange,
        candidates: &mut Vec<PatternCandidate<'a>>,
    ) -> Result<(), ExpressionParseError> {
        let input = range
            .slice(self.source.virtual_source())
            .ok_or(ExpressionParseError::InvalidInputRange { range })?;
        for candidate in candidates.iter_mut() {
            candidate.patterns.retain(|pattern| {
                if self.environment.may_override_pattern(
                    candidate.kind,
                    &candidate.registration_id,
                    pattern.pattern_index,
                ) {
                    return true;
                }
                let prefilter = self
                    .pattern_prefilters
                    .entry(pattern.source)
                    .or_insert_with(|| PatternPrefilter::new(pattern));
                pattern_prefilter_matches(prefilter, input)
            });
        }
        candidates.retain(|candidate| !candidate.patterns.is_empty());
        Ok(())
    }

    pub(crate) fn resolved_node(&self, id: &str) -> Option<&ExpressionNode> {
        self.resolved_nodes.get(id)
    }

    pub(crate) fn syntax_candidates(&mut self, kind: SyntaxKind) -> Vec<PatternCandidate<'a>> {
        if let Some(candidates) = self.syntax_candidate_templates.get(&kind) {
            return candidates.clone();
        }
        let candidates = if let Some(snapshot) = self.dynamic_snapshot {
            snapshot_pattern_candidates(self.catalog, snapshot, kind)
        } else {
            catalog_pattern_candidates(self.catalog, kind)
        };
        self.syntax_candidate_templates
            .insert(kind, candidates.clone());
        candidates
    }

    pub(crate) const fn catalog(&self) -> &'a Catalog {
        self.catalog
    }

    pub(crate) const fn dynamic_snapshot(&self) -> Option<&'a DynamicSyntaxSnapshot> {
        self.dynamic_snapshot
    }

    pub(crate) const fn source(&self) -> &'a MappedSource {
        self.source
    }

    pub(crate) fn environment(&self) -> &E {
        self.environment
    }

    pub(crate) fn environment_mut(&mut self) -> &mut E {
        self.environment
    }

    pub(crate) fn function_definitions(
        &mut self,
        name: &str,
    ) -> Result<Vec<FunctionDefinition>, ExpressionParseError> {
        let context = self.context.clone();
        let mut definitions = self
            .environment
            .lookup_functions(FunctionLookupRequest {
                name,
                context: &context,
            })
            .map_err(|message| ExpressionParseError::Environment { message })?;
        definitions.retain(|definition| definition.name == name);

        let mut shapes = definitions
            .iter()
            .map(FunctionDefinition::shape)
            .collect::<HashSet<_>>();
        definitions.extend(
            self.catalog
                .functions_named(name)
                .into_iter()
                .map(FunctionDefinition::from_catalog)
                .filter(|definition| shapes.insert(definition.shape())),
        );
        Ok(definitions)
    }

    pub(crate) fn begin_semantic_candidate(&mut self) -> Result<(), String> {
        self.environment.begin_semantic_candidate()
    }

    pub(crate) fn finish_semantic_candidate(&mut self, accepted: bool) -> Result<(), String> {
        self.environment.finish_semantic_candidate(accepted)
    }

    pub(crate) fn context(&self) -> &ExpressionParseContext {
        &self.context
    }

    pub(crate) fn event_restriction_failure(
        &self,
        registration_id: &str,
        span: MatchSpan,
    ) -> Option<FailureTrace> {
        let EventApplicability::NoMatch { supported, current } =
            self.event_applicability(registration_id)
        else {
            return None;
        };
        Some(FailureTrace::leaf(PatternFailure {
            span,
            reasons: vec![PatternFailureReason::EventRestricted { supported, current }],
        }))
    }

    fn event_applicability(&self, registration_id: &str) -> EventApplicability {
        let common = self
            .catalog
            .syntax_by_registration_id(registration_id)
            .into_iter()
            .find_map(syntaxes::Syntax::common);
        let Some(common) = common else {
            return EventApplicability::Match;
        };
        match common.supported_events_state {
            None => return EventApplicability::Match,
            Some(ResolutionState::Unresolved) => return EventApplicability::Unknown,
            Some(ResolutionState::Resolved) => {}
        }
        let Some(supported) = common.supported_events.as_deref() else {
            return EventApplicability::Unknown;
        };
        let mut unknown_relation = false;
        if supported.iter().any(|supported| {
            self.context.event_classes.iter().any(|current| {
                if current == supported {
                    return true;
                }
                if self.catalog.class(current.as_str()).is_none()
                    || self.catalog.class(supported.as_str()).is_none()
                {
                    unknown_relation = true;
                    return false;
                }
                self.catalog
                    .is_class_assignable(current.as_str(), supported.as_str())
            })
        }) {
            return EventApplicability::Match;
        }
        if unknown_relation {
            EventApplicability::Unknown
        } else {
            EventApplicability::NoMatch {
                supported: supported
                    .iter()
                    .map(|event| event.as_str().to_owned())
                    .collect(),
                current: self
                    .context
                    .event_classes
                    .iter()
                    .map(|event| event.as_str().to_owned())
                    .collect(),
            }
        }
    }

    pub(crate) fn replace_context(
        &mut self,
        context: ExpressionParseContext,
    ) -> ExpressionParseContext {
        std::mem::replace(&mut self.context, context)
    }

    pub(crate) fn ensure_depth(&self, depth: usize) -> Result<(), ExpressionParseError> {
        if depth > self.config.max_depth {
            Err(ExpressionParseError::DepthLimit {
                limit: self.config.max_depth,
            })
        } else {
            Ok(())
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn parse_prefixes(
        &mut self,
        range: TextRange,
        candidate_ends: &[usize],
        expected_types: &[ExpressionExpectedType],
        allow_literals: bool,
        allow_expressions: bool,
        time: i32,
        depth: usize,
    ) -> Result<Vec<ExpressionCandidate>, ExpressionParseError> {
        Ok(self
            .parse_prefixes_mode(
                range,
                candidate_ends,
                expected_types,
                allow_literals,
                allow_expressions,
                time,
                depth,
                true,
            )?
            .candidates)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn parse_prefixes_detailed(
        &mut self,
        range: TextRange,
        candidate_ends: &[usize],
        expected_types: &[ExpressionExpectedType],
        allow_literals: bool,
        allow_expressions: bool,
        time: i32,
        depth: usize,
    ) -> Result<PrefixParse, ExpressionParseError> {
        self.parse_prefixes_mode(
            range,
            candidate_ends,
            expected_types,
            allow_literals,
            allow_expressions,
            time,
            depth,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn parse_prefixes_mode(
        &mut self,
        range: TextRange,
        candidate_ends: &[usize],
        expected_types: &[ExpressionExpectedType],
        allow_literals: bool,
        allow_expressions: bool,
        time: i32,
        depth: usize,
        allow_lists: bool,
    ) -> Result<PrefixParse, ExpressionParseError> {
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
            allow_lists,
            state_revision: self
                .environment
                .state_revision()
                .map_err(|message| ExpressionParseError::Environment { message })?,
        };
        if let Some(cached) = self.memo.get(&key) {
            return Ok(cached.clone());
        }
        if !self.active.insert(key.clone()) {
            return Ok(PrefixParse::default());
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
                allow_lists,
            )?;
            let mut candidates = Vec::new();
            self.extend_unique_candidates(&mut candidates, base.candidates)?;
            let mut failure = base.failure;
            self.memo.insert(
                key.clone(),
                PrefixParse {
                    candidates: candidates.clone(),
                    failure: failure.clone(),
                },
            );

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
                    allow_lists,
                )?;
                let added = self.extend_unique_candidates(&mut candidates, recursive.candidates)?;
                failure = choose_failure_trace(failure, recursive.failure);
                self.memo.insert(
                    key.clone(),
                    PrefixParse {
                        candidates: candidates.clone(),
                        failure: failure.clone(),
                    },
                );
                if added == 0 {
                    break;
                }
            }
            Ok(PrefixParse {
                candidates,
                failure,
            })
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
        allow_lists: bool,
    ) -> Result<PrefixParse, ExpressionParseError> {
        let mut candidates = Vec::new();
        let mut failure = None;
        let mut ordinary_candidate_ends = candidate_ends.to_vec();
        if self
            .source
            .virtual_source()
            .get(range.start..range.end)
            .is_some_and(|input| input.starts_with('('))
            && let Some(close) = find_parenthesis_end(
                self.source.virtual_source(),
                range.start + '('.len_utf8(),
                range.end,
            )
        {
            let group_end = close + ')'.len_utf8();
            if candidate_ends.contains(&group_end) {
                ordinary_candidate_ends.retain(|end| *end != group_end);
                if include_leaves {
                    let raw_inner = TextRange::new(range.start + '('.len_utf8(), close);
                    let inner_text = raw_inner
                        .slice(self.source.virtual_source())
                        .expect("parenthesized range is validated");
                    let local_inner = java_trim_range(inner_text);
                    let inner = TextRange::new(
                        raw_inner.start + local_inner.start,
                        raw_inner.start + local_inner.end,
                    );
                    if !inner.is_empty() {
                        let inner_candidates = self
                            .parse_prefixes_mode(
                                inner,
                                &[inner.end],
                                expected_types,
                                allow_literals,
                                allow_expressions,
                                time,
                                depth + 1,
                                allow_lists,
                            )?
                            .candidates;
                        let group_span = self.map_range(TextRange::new(range.start, group_end))?;
                        for inner_candidate in inner_candidates {
                            let child = inner_candidate.node;
                            candidates.push(ExpressionCandidate {
                                node: ExpressionNode {
                                    kind: ExpressionNodeKind::Grouped,
                                    function: None,
                                    span: group_span.clone(),
                                    return_type: child.return_type.clone(),
                                    multiplicity: child.multiplicity,
                                    captures: Vec::new(),
                                    tags: Vec::new(),
                                    mark: 0,
                                    metadata: child.metadata.clone(),
                                    children: vec![child],
                                    routed_captures: Vec::new(),
                                },
                                expected_alternative: inner_candidate.expected_alternative,
                            });
                        }
                    }
                }
            }
        }
        let candidate_ends = ordinary_candidate_ends.as_slice();
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
                        function: None,
                        span: self.map_range(leaf.range)?,
                        return_type: leaf.return_type,
                        multiplicity: leaf.multiplicity,
                        captures: Vec::new(),
                        tags: Vec::new(),
                        mark: 0,
                        children: leaf.children,
                        routed_captures: Vec::new(),
                        metadata: leaf.metadata,
                    },
                    expected_alternative: None,
                });
            }
            self.environment
                .finish_expression_leaf(!accepted_leaves.is_empty())
                .map_err(|message| ExpressionParseError::Environment { message })?;
            candidates.extend(accepted_leaves);

            if allow_expressions {
                candidates.extend(crate::arithmetic::parse_arithmetic(
                    self,
                    range,
                    candidate_ends,
                    expected_types,
                    depth,
                )?);
                let functions = crate::function::parse_function_call(
                    self,
                    range,
                    candidate_ends,
                    expected_types,
                    depth,
                )?;
                candidates.extend(functions.candidates);
                failure = choose_failure_trace(failure, functions.failure);
            }
        }

        if allow_expressions {
            for end in candidate_ends.iter().copied() {
                let candidate_range = TextRange::new(range.start, end);
                let candidate_text = candidate_range
                    .slice(self.source.virtual_source())
                    .expect("validated Expression range");
                let matcher_candidates =
                    self.matcher_candidates(candidate_text, expected_types, registered_pass);
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
                failure = choose_failure_trace(failure, matched.primary_failure().cloned());
                if let Some(selected) = matched.selected {
                    match self.registered_node(
                        selected,
                        candidate_range.start,
                        expected_types,
                        depth,
                    )? {
                        RegisteredNodeResolution::Accepted(candidate) => candidates.push(candidate),
                        RegisteredNodeResolution::Rejected(trace) => {
                            failure = choose_failure_trace(failure, trace);
                        }
                    }
                }
                for alternative in matched.alternatives {
                    match self.registered_node(
                        alternative,
                        candidate_range.start,
                        expected_types,
                        depth,
                    )? {
                        RegisteredNodeResolution::Accepted(candidate) => candidates.push(candidate),
                        RegisteredNodeResolution::Rejected(trace) => {
                            failure = choose_failure_trace(failure, trace);
                        }
                    }
                }
            }

            if include_leaves && allow_lists {
                let lists = self.parse_expression_lists(
                    range,
                    candidate_ends,
                    expected_types,
                    allow_literals,
                    time,
                    depth,
                )?;
                candidates.extend(lists.candidates);
                failure = choose_failure_trace(failure, lists.failure);
            }
        }
        Ok(PrefixParse {
            candidates,
            failure,
        })
    }

    fn parse_expression_lists(
        &mut self,
        range: TextRange,
        candidate_ends: &[usize],
        expected_types: &[ExpressionExpectedType],
        allow_literals: bool,
        time: i32,
        depth: usize,
    ) -> Result<PrefixParse, ExpressionParseError> {
        let mut lists = Vec::new();
        let mut failure = None;
        for end in candidate_ends.iter().copied() {
            let list_range = TextRange::new(range.start, end);
            let Some(raw) = split_expression_list(self.source.virtual_source(), list_range) else {
                continue;
            };

            let mut children = Vec::with_capacity(raw.pieces.len());
            let mut child_starts = Vec::with_capacity(raw.pieces.len());
            let mut first = 0;
            let mut valid = true;
            while first < raw.pieces.len() {
                let mut accepted = None;
                let mut current_failure = None;
                let mut attempted_range = None;
                for last in first..raw.pieces.len() {
                    if first == 0 && last + 1 == raw.pieces.len() {
                        continue;
                    }
                    let candidate_range =
                        TextRange::new(raw.pieces[first].start, raw.pieces[last].end);
                    attempted_range = Some(candidate_range);
                    let nested_lists =
                        completely_parenthesized(self.source.virtual_source(), candidate_range);
                    let mut parsed = self.parse_prefixes_mode(
                        candidate_range,
                        &[candidate_range.end],
                        expected_types,
                        allow_literals,
                        true,
                        time,
                        depth + 1,
                        nested_lists,
                    )?;
                    if !parsed.candidates.is_empty() {
                        accepted = Some((last, parsed.candidates.remove(0).node));
                        break;
                    }
                    current_failure = choose_failure_trace(current_failure, parsed.failure);
                }
                let Some((last, node)) = accepted else {
                    if current_failure.is_none()
                        && let Some(range) = attempted_range
                    {
                        current_failure = Some(FailureTrace::leaf(PatternFailure {
                            span: self.map_range(range)?,
                            reasons: vec![PatternFailureReason::Expression],
                        }));
                    }
                    failure = choose_failure_trace(failure, current_failure);
                    valid = false;
                    break;
                };
                child_starts.push(first);
                children.push(node);
                first = last + 1;
            }
            if !valid || children.len() < 2 {
                continue;
            }

            let conjunction = raw.conjunction_for_children(&child_starts);
            if conjunction == ExpressionListConjunction::And
                && expected_types.iter().all(|expected| !expected.plural)
            {
                continue;
            }

            let return_type = list_return_type(self.catalog, &children);
            let multiplicity = match conjunction {
                ExpressionListConjunction::And => Multiplicity::Multiple,
                ExpressionListConjunction::Or
                    if children.iter().all(|child| {
                        matches!(
                            child.multiplicity,
                            None | Some(Multiplicity::Single | Multiplicity::Both)
                        )
                    }) =>
                {
                    Multiplicity::Single
                }
                ExpressionListConjunction::Or => Multiplicity::Multiple,
            };
            lists.push(ExpressionCandidate {
                node: ExpressionNode {
                    kind: ExpressionNodeKind::List { conjunction },
                    function: None,
                    span: self.map_range(list_range)?,
                    return_type,
                    multiplicity: Some(multiplicity),
                    captures: Vec::new(),
                    tags: Vec::new(),
                    mark: 0,
                    children,
                    routed_captures: Vec::new(),
                    metadata: BTreeMap::new(),
                },
                expected_alternative: None,
            });
        }
        Ok(PrefixParse {
            candidates: lists,
            failure,
        })
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
                if !self.catalog.operators().is_empty()
                    && !self.catalog.operations().is_empty()
                    && metadata.element_class.as_str().ends_with(".ExprArithmetic")
                {
                    return false;
                }
                let return_type_matches = match metadata.return_type_state {
                    ReturnTypeState::Static => {
                        self.environment.can_resolve_registered_expression(
                            RegisteredSyntaxIdentity {
                                kind: SyntaxKind::Expression,
                                definition_id: &candidate.definition_id,
                                registration_id: &candidate.registration_id,
                                pattern_index: None,
                            },
                        ) || self.return_type_matches(metadata.return_type.as_ref(), expected_types)
                    }
                    ReturnTypeState::Dynamic | ReturnTypeState::Unresolved => {
                        self.environment.can_resolve_registered_expression(
                            RegisteredSyntaxIdentity {
                                kind: SyntaxKind::Expression,
                                definition_id: &candidate.definition_id,
                                registration_id: &candidate.registration_id,
                                pattern_index: None,
                            },
                        ) || self.return_type_matches(metadata.return_type.as_ref(), expected_types)
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
            for (candidate_index, candidate) in self.registered_candidates.iter().enumerate() {
                positions.extend(candidate.patterns.iter().enumerate().filter_map(
                    |(pattern_position, pattern)| {
                        self.environment
                            .may_override_pattern(
                                candidate.kind,
                                &candidate.registration_id,
                                pattern.pattern_index,
                            )
                            .then_some((candidate_index, pattern_position))
                    },
                ));
            }
            positions.sort_unstable();
            positions.dedup();
            positions.retain(|(candidate_index, pattern_index)| {
                let candidate = &self.registered_candidates[*candidate_index];
                let pattern = candidate.patterns[*pattern_index];
                let overridden = self.environment.may_override_pattern(
                    candidate.kind,
                    &candidate.registration_id,
                    pattern.pattern_index,
                );
                (overridden && matches!(registered_pass, RegisteredPass::Base))
                    || (!overridden
                        && self.pattern_prefilters[pattern.source].left_recursive
                            == matches!(registered_pass, RegisteredPass::LeftRecursive)
                        && compatible[*candidate_index])
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
                let overridden = self.environment.may_override_pattern(
                    candidate.kind,
                    &candidate.registration_id,
                    pattern.pattern_index,
                );
                if overridden || pattern_prefilter_matches(prefilter, input) {
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
        depth: usize,
    ) -> Result<RegisteredNodeResolution, ExpressionParseError> {
        let metadata = self
            .registration_metadata(&matched.registration_id)
            .cloned();
        let local = matched.matched.span.local_range;
        let absolute = TextRange::new(frame_start + local.start, frame_start + local.end);
        let span = self.map_range(absolute)?;
        if let Some(failure) =
            self.event_restriction_failure(&matched.registration_id, span.clone())
        {
            return Ok(RegisteredNodeResolution::Rejected(Some(failure)));
        }
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
        let mut parsed_captures = Vec::new();
        for (capture_index, capture) in matched.matched.captures.iter().enumerate() {
            if let PatternCapture::TypeExpression {
                resolution_id: Some(id),
                ..
            } = capture
                && let Some(node) = self.resolved_nodes.get(id).cloned()
            {
                parsed_captures.push(expression_parsed_capture(capture_index, node));
            }
        }
        let mut return_type = metadata
            .as_ref()
            .and_then(|value| value.return_type.clone());
        let mut multiplicity = metadata.as_ref().and_then(|value| value.multiplicity);
        let mut node_metadata = metadata
            .as_ref()
            .map_or_else(BTreeMap::new, |value| value.metadata.clone());
        let capture_bindings = if metadata.is_some() {
            self.environment
                .registered_capture_bindings(RegisteredSyntaxIdentity {
                    kind: SyntaxKind::Expression,
                    definition_id: &matched.definition_id,
                    registration_id: &matched.registration_id,
                    pattern_index: Some(matched.pattern_index),
                })
                .map_err(|message| ExpressionParseError::Environment { message })?
        } else {
            Vec::new()
        };
        for (capture_index, capture) in matched.matched.captures.iter().enumerate() {
            let Some(binding) = capture_bindings
                .iter()
                .find(|binding| binding.capture_index == capture_index)
            else {
                continue;
            };
            let PatternCapture::Regex {
                pattern_span, span, ..
            } = capture
            else {
                continue;
            };
            let local = span.local_range;
            let range = TextRange::new(frame_start + local.start, frame_start + local.end);
            let parsed = match binding.parser_id.as_str() {
                HOST_CONDITION_PARSER_ID => {
                    let parsed =
                        crate::condition::parse_condition_with_session(self, range, depth + 1)
                            .map_err(|error| ExpressionParseError::Environment {
                                message: error.to_string(),
                            })?;
                    match parsed.selected {
                        Some(selected) => condition_parsed_capture(capture_index, selected.node),
                        None => {
                            let cause = parsed.unknown.and_then(|unknown| unknown.failure);
                            let frame = FailureFrame {
                                kind: matched.kind,
                                definition_id: matched.definition_id.clone(),
                                registration_id: matched.registration_id.clone(),
                                pattern_index: matched.pattern_index,
                                pattern: matched.pattern.clone(),
                                element_path: Vec::new(),
                                pattern_span: Some(*pattern_span),
                                input_span: span.clone(),
                                role: FailureFrameRole::ConditionCapture {
                                    index: capture_index,
                                },
                            };
                            if binding.required {
                                return Ok(RegisteredNodeResolution::Rejected(
                                    cause.map(|cause| cause.with_parent(frame)),
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
                    let parsed = crate::effect::parse_effect_range_with_session(
                        self,
                        range,
                        crate::RawNodeId::new(0),
                        depth + 1,
                    )
                    .map_err(|error| ExpressionParseError::Environment {
                        message: error.to_string(),
                    })?;
                    match parsed.selected {
                        Some(selected) => {
                            let summary =
                                crate::effect::effect_semantic_summary(&selected, self.catalog);
                            ParsedCapture {
                                capture_index,
                                binding: binding.clone(),
                                result: ParsedCaptureResult::success(
                                    binding.parser_id.clone(),
                                    span.clone(),
                                    Some(summary),
                                    ParsedCaptureValue::Effect(Box::new(selected)),
                                ),
                            }
                        }
                        None => {
                            if binding.required {
                                return Ok(RegisteredNodeResolution::Rejected(Some(
                                    crate::effect::nested_failure_trace(span.clone()),
                                )));
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
                        return Ok(RegisteredNodeResolution::Rejected(Some(
                            crate::effect::nested_failure_trace(span.clone()),
                        )));
                    }
                    ParsedCapture {
                        capture_index,
                        binding: binding.clone(),
                        result,
                    }
                }
            };
            if let ParsedCaptureResult {
                status: ParsedCaptureStatus::Failed,
                ..
            } = &parsed.result
                && binding.required
            {
                return Ok(RegisteredNodeResolution::Rejected(Some(
                    crate::effect::nested_failure_trace(span.clone()),
                )));
            }
            parsed_captures.push(parsed);
        }
        let routed_captures = parsed_captures
            .iter()
            .filter(|capture| capture.result.parser_id != HOST_EXPRESSION_PARSER_ID)
            .cloned()
            .collect::<Vec<_>>();
        let needs_resolution = metadata.as_ref().is_some_and(|value| {
            value.return_type_state != ReturnTypeState::Static
                || value.multiplicity_state == ResolutionState::Unresolved
                || self
                    .environment
                    .can_resolve_registered_expression(RegisteredSyntaxIdentity {
                        kind: SyntaxKind::Expression,
                        definition_id: &matched.definition_id,
                        registration_id: &matched.registration_id,
                        pattern_index: Some(matched.pattern_index),
                    })
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
                    pattern: &matched.pattern,
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
                    parsed_captures: &parsed_captures,
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
                    return Ok(RegisteredNodeResolution::Rejected(None));
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
            return Ok(RegisteredNodeResolution::Rejected(None));
        }
        Ok(RegisteredNodeResolution::Accepted(ExpressionCandidate {
            node: ExpressionNode {
                kind: ExpressionNodeKind::Registered {
                    definition_id: matched.definition_id,
                    registration_id: matched.registration_id,
                    pattern_index: matched.pattern_index,
                },
                function: None,
                span,
                return_type,
                multiplicity,
                captures: matched.matched.captures,
                tags: matched.matched.tags,
                mark: matched.matched.mark,
                children,
                routed_captures,
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

    pub(crate) fn return_type_matches(
        &self,
        return_type: Option<&ClassName>,
        expected_types: &[ExpressionExpectedType],
    ) -> bool {
        expected_types.is_empty()
            || return_type.is_some_and(|return_type| {
                expected_types.iter().any(|expected| {
                    let from = return_type.as_str();
                    let to = expected.class_name.as_str();
                    // Skript can attempt Object conversions using the runtime value. Static
                    // candidates must first be narrowed by CoreLibrary or an addon, otherwise
                    // a generic Object candidate hides a known Player, Number, and so on.
                    to == "java.lang.Object"
                        || (from != "java.lang.Object" && self.catalog.can_convert(from, to))
                })
            })
    }

    pub(crate) fn multiplicity_matches(
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

    pub(crate) fn map_range(&self, range: TextRange) -> Result<MatchSpan, ExpressionParseError> {
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
    minimum_input_len: usize,
    leading: Vec<PrefixState>,
    trailing: Vec<PrefixState>,
    required_literal_branches: Vec<Vec<String>>,
}

impl PatternPrefilter {
    fn new(pattern: &MatchPattern<'_>) -> Self {
        Self {
            left_recursive: pattern_is_left_recursive(&pattern.parsed.elements),
            minimum_input_len: minimum_pattern_input_len(&pattern.parsed.elements),
            leading: leading_prefix_states(&pattern.parsed.elements),
            trailing: trailing_suffix_states(&pattern.parsed.elements),
            required_literal_branches: required_literal_branches(&pattern.parsed.elements),
        }
    }
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
            .or_insert_with(|| PatternPrefilter::new(pattern));
    }
    result
}

fn minimum_pattern_input_len(elements: &[SpannedPatternElement]) -> usize {
    elements.iter().fold(0usize, |length, element| {
        let element_length = match &element.value {
            // Skript may elide literal spaces at capture boundaries. Counting
            // non-space characters is a conservative byte lower bound even for UTF-8.
            PatternElement::Literal(value) => value.chars().filter(|ch| *ch != ' ').count(),
            PatternElement::Group(children) => minimum_pattern_input_len(children),
            PatternElement::Choice(branches) => branches
                .iter()
                .map(|branch| minimum_pattern_input_len(branch))
                .min()
                .unwrap_or(0),
            PatternElement::Regex(_)
            | PatternElement::TypeExpr(_)
            | PatternElement::Option(_)
            | PatternElement::ParseTag(_)
            | PatternElement::ParseMark(_)
            | PatternElement::Empty => 0,
        };
        length.saturating_add(element_length)
    })
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

fn required_literal_branches(elements: &[SpannedPatternElement]) -> Vec<Vec<String>> {
    let mut branches = vec![Vec::new()];
    for element in elements {
        let element_branches = match &element.value {
            PatternElement::Literal(value) => {
                let value = value.trim();
                if value.is_empty() {
                    vec![Vec::new()]
                } else {
                    vec![vec![value.to_lowercase()]]
                }
            }
            PatternElement::Group(children) => required_literal_branches(children),
            PatternElement::Choice(choices) => choices
                .iter()
                .flat_map(|choice| required_literal_branches(choice))
                .collect(),
            PatternElement::Regex(_)
            | PatternElement::TypeExpr(_)
            | PatternElement::Option(_)
            | PatternElement::ParseTag(_)
            | PatternElement::ParseMark(_)
            | PatternElement::Empty => vec![Vec::new()],
        };
        let mut combined = Vec::new();
        for branch in &branches {
            for element_branch in &element_branches {
                let mut value = branch.clone();
                value.extend(element_branch.iter().cloned());
                combined.push(value);
                if combined.len() > 256 {
                    return vec![Vec::new()];
                }
            }
        }
        branches = combined;
    }
    branches
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
        state.text.bytes().all(|value| value == b' ')
            || starts_with_skript_literal(input, &state.text)
    })
}

fn suffix_states_may_end_with(states: &[PrefixState], input: &str) -> bool {
    states.iter().any(|state| {
        state.text.bytes().all(|value| value == b' ')
            || ends_with_skript_literal(input, &state.text)
    })
}

fn pattern_prefilter_matches(prefilter: &PatternPrefilter, input: &str) -> bool {
    if input.len() < prefilter.minimum_input_len
        || !prefix_states_may_start_with(&prefilter.leading, input)
        || !suffix_states_may_end_with(&prefilter.trailing, input)
    {
        return false;
    }
    let lowercase_input: Cow<'_, str> = if input.is_ascii() {
        Cow::Borrowed(input)
    } else {
        Cow::Owned(input.to_lowercase())
    };
    prefilter.required_literal_branches.iter().any(|branch| {
        branch
            .iter()
            .all(|literal| contains_literal_ignore_case(input, lowercase_input.as_ref(), literal))
    })
}

fn contains_literal_ignore_case(input: &str, lowercase_input: &str, literal: &str) -> bool {
    if literal.is_empty() {
        return true;
    }
    if input.is_ascii() && literal.is_ascii() {
        return input
            .as_bytes()
            .windows(literal.len())
            .any(|window| window.eq_ignore_ascii_case(literal.as_bytes()));
    }
    lowercase_input.contains(literal)
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

fn expression_failure_ranges(
    input: &str,
    range: TextRange,
) -> (ExpressionFailureKind, TextRange, Option<TextRange>) {
    if input
        .get(range.start..range.end)
        .is_some_and(|text| text.starts_with('('))
        && let Some(close) = find_parenthesis_end(input, range.start + '('.len_utf8(), range.end)
        && close + ')'.len_utf8() == range.end
    {
        let raw_inner = TextRange::new(range.start + '('.len_utf8(), close);
        let inner_text = raw_inner
            .slice(input)
            .expect("parenthesized failure range is validated");
        let inner = java_trim_range(inner_text);
        if inner.is_empty() {
            let empty = TextRange::empty(raw_inner.start + inner.start);
            return (
                ExpressionFailureKind::EmptyGroup,
                empty,
                Some(TextRange::new(range.start, range.start + '('.len_utf8())),
            );
        }
    }

    let mut stack = Vec::new();
    let mut cursor = range.start;
    while cursor < range.end {
        let Some(ch) = input
            .get(cursor..range.end)
            .and_then(|text| text.chars().next())
        else {
            break;
        };
        match ch {
            '"' => {
                let Some(end) = find_quote_end(input, cursor + ch.len_utf8(), range.end) else {
                    break;
                };
                cursor = end + '"'.len_utf8();
                continue;
            }
            '{' => {
                let Some(end) = find_variable_end(input, cursor + ch.len_utf8(), range.end) else {
                    break;
                };
                cursor = end + '}'.len_utf8();
                continue;
            }
            '(' => stack.push(cursor),
            ')' if stack.is_empty() => {
                return (
                    ExpressionFailureKind::UnexpectedClosingParenthesis,
                    TextRange::new(cursor, cursor + ch.len_utf8()),
                    None,
                );
            }
            ')' => {
                stack.pop();
            }
            _ => {}
        }
        cursor += ch.len_utf8();
    }
    if let Some(open) = stack.last().copied() {
        return (
            ExpressionFailureKind::UnclosedParenthesis,
            TextRange::empty(range.end),
            Some(TextRange::new(open, open + '('.len_utf8())),
        );
    }
    (
        ExpressionFailureKind::ExpectedExpression,
        TextRange::empty(range.start),
        None,
    )
}

fn completely_parenthesized(input: &str, range: TextRange) -> bool {
    range
        .slice(input)
        .is_some_and(|value| value.starts_with('('))
        && find_parenthesis_end(input, range.start + '('.len_utf8(), range.end)
            .is_some_and(|close| close + ')'.len_utf8() == range.end)
}

fn list_return_type(catalog: &Catalog, children: &[ExpressionNode]) -> Option<ClassName> {
    let types = children
        .iter()
        .map(|child| child.return_type.clone())
        .collect::<Option<Vec<_>>>()?;
    catalog.common_skript_class(&types)
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
        pattern_index: usize,
    ) -> Result<bool, String> {
        self.environment
            .allows_regex_pattern(kind, registration_id, pattern_index)
    }

    fn may_override_pattern(
        &self,
        kind: crate::MatchSyntaxKind,
        registration_id: &str,
        pattern_index: usize,
    ) -> bool {
        self.environment
            .may_override_pattern(kind, registration_id, pattern_index)
    }

    fn resolve_type(
        &mut self,
        request: TypeExpressionRequest<'_>,
    ) -> Result<TypeExpressionOutcome, String> {
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
    ) -> Result<TypeExpressionOutcome, ExpressionParseError> {
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
        let mut failure = None;
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
            let parsed = self.parse_prefixes_mode(
                remaining,
                &candidate_ends,
                &expected,
                request.expression.allow_literals,
                request.expression.allow_expressions,
                request.expression.time,
                depth,
                true,
            )?;
            failure = choose_failure_trace(failure, parsed.failure);
            // Skript keeps the first successful registration. The parent matcher only
            // needs one AST for each amount of input this capture can consume.
            let mut resolved_ends = HashSet::new();
            for mut candidate in parsed.candidates {
                candidate.expected_alternative = Some(alternative_index);
                let absolute = candidate.node.span.local_range;
                if !resolved_ends.insert(absolute.end) {
                    continue;
                }
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
        Ok(TypeExpressionOutcome {
            resolutions,
            failure,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_prefilter_matches_unicode_literals() {
        let prefilter = PatternPrefilter {
            left_recursive: false,
            minimum_input_len: 0,
            leading: vec![PrefixState {
                text: String::new(),
                terminal: false,
            }],
            trailing: vec![PrefixState {
                text: String::new(),
                terminal: false,
            }],
            required_literal_branches: vec![vec!["café".to_owned(), "über".to_owned()]],
        };

        assert!(pattern_prefilter_matches(&prefilter, "CAFÉ ÜBER"));
        assert!(!pattern_prefilter_matches(&prefilter, "CAFE ÜBER"));
    }

    #[test]
    fn capture_summary_exposes_the_concrete_expression_kind() {
        let source = MappedSource::identity("{value}");
        let node = ExpressionNode {
            kind: ExpressionNodeKind::Variable {
                parser_id: "core.variable".to_owned(),
            },
            function: None,
            span: MatchSpan {
                local_range: TextRange::new(0, 7),
                mapped: source.map_range(TextRange::new(0, 7)).unwrap(),
            },
            return_type: Some(ClassName("java.lang.Object".to_owned())),
            multiplicity: Some(Multiplicity::Single),
            captures: Vec::new(),
            tags: Vec::new(),
            mark: 0,
            children: Vec::new(),
            routed_captures: Vec::new(),
            metadata: BTreeMap::new(),
        };

        let summary = expression_semantic_summary(&node);

        assert_eq!(summary.kind, "variable");
        assert_eq!(summary.multiplicity, Some(Multiplicity::Single));
    }
}

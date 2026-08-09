use crate::OutputFormat;
use parser_wasm::host::WasmEffectParseResult;
use serde::Serialize;
use skript_parser::{
    CandidateMatch, EffectCandidate, ExpressionNode, ExpressionNodeKind, MatchSpan,
    ParseMarkCapture, ParseTagCapture, PatternCapture, PatternFailure, PatternFailureReason,
    TextRange,
};
use std::collections::BTreeMap;
use std::io::{self, Write};
use syntax_pattern_parser::syntax::{
    PatternElement, PatternTypeExpr, Span as PatternSpan, SpannedPatternElement, parse,
};
use syntaxes::{Catalog, CommonSyntax, Multiplicity, Syntax};

const REPORT_SCHEMA_VERSION: u32 = 1;
const MAX_REPORT_EXPRESSION_DEPTH: usize = 8;
const MAX_REPORT_PATTERN_DEPTH: usize = 16;

#[derive(Debug, Clone)]
pub(crate) struct SnapshotDescription {
    pub snapshot_id: String,
    pub minecraft_version: String,
    pub skript_version: String,
    pub plugin_count: usize,
}

/// Stable Effect analysis result used by terminal and JSON rendering.
///
/// The internal DTO deliberately differs from parser implementation types. It
/// keeps the utility's JSON contract stable while the parser grows additional
/// node kinds, including future structured Function calls.
#[derive(Debug, Clone)]
pub struct AnalysisReport {
    data: ReportData,
}

impl AnalysisReport {
    pub(crate) fn from_result(
        input: &str,
        snapshot: &SnapshotDescription,
        result: WasmEffectParseResult,
        catalog: &Catalog,
    ) -> Self {
        let diagnostics = result
            .effects
            .diagnostics
            .into_iter()
            .map(|diagnostic| DiagnosticReport {
                code: diagnostic.code,
                message: diagnostic.message,
                severity: format!("{:?}", diagnostic.severity).to_ascii_lowercase(),
                span: SpanReport {
                    start: usize::try_from(diagnostic.span.virtual_range.start)
                        .unwrap_or(usize::MAX),
                    end: usize::try_from(diagnostic.span.virtual_range.end).unwrap_or(usize::MAX),
                },
            })
            .collect();
        let component_failures = result
            .failures
            .into_iter()
            .map(|failure| ComponentFailureReport {
                component_id: failure.component_id,
                subscription_id: failure.subscription_id,
                message: failure.error.to_string(),
            })
            .collect();
        let parse_result = if let Some(selected) = result.matches.selected {
            ParseResultReport::Matched {
                effect: Box::new(effect_report(input, selected, catalog)),
                alternatives: result
                    .matches
                    .alternatives
                    .into_iter()
                    .map(|candidate| candidate_summary(candidate, catalog))
                    .collect(),
            }
        } else {
            ParseResultReport::Unknown {
                source: result
                    .matches
                    .unknown
                    .as_ref()
                    .map_or_else(|| input.to_owned(), |unknown| unknown.source.clone()),
                failure: result
                    .matches
                    .unknown
                    .and_then(|unknown| unknown.failure)
                    .map(failure_report),
            }
        };

        Self {
            data: ReportData {
                schema_version: REPORT_SCHEMA_VERSION,
                input: input.to_owned(),
                snapshot: SnapshotReport {
                    id: snapshot.snapshot_id.clone(),
                    minecraft_version: snapshot.minecraft_version.clone(),
                    skript_version: snapshot.skript_version.clone(),
                    plugin_count: snapshot.plugin_count,
                },
                result: parse_result,
                diagnostics,
                component_failures,
            },
        }
    }

    /// Returns whether one Effect candidate consumed the complete input.
    pub fn matched(&self) -> bool {
        matches!(self.data.result, ParseResultReport::Matched { .. })
    }

    /// Serializes the versioned machine-readable report.
    ///
    /// Serialization consumes the report so recursive DTO destruction happens
    /// on the same bounded worker stack used for rendering.
    pub fn to_json(self) -> io::Result<String> {
        String::from_utf8(self.render(OutputFormat::Json)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// Writes this report using the selected terminal or JSON representation.
    pub fn write(self, format: OutputFormat, mut writer: impl Write) -> io::Result<()> {
        writer.write_all(&self.render(format)?)
    }

    fn render(self, format: OutputFormat) -> io::Result<Vec<u8>> {
        let worker = std::thread::Builder::new()
            .name("effectcommandcli-report".to_owned())
            .stack_size(32 * 1024 * 1024)
            .spawn(move || {
                let mut output = Vec::new();
                match format {
                    OutputFormat::Human => self.write_human(&mut output)?,
                    OutputFormat::Json => {
                        serde_json::to_writer_pretty(&mut output, &self.data)
                            .map_err(io::Error::other)?;
                        writeln!(output)?;
                    }
                }
                Ok(output)
            })?;
        worker
            .join()
            .map_err(|_| io::Error::other("Effect report worker panicked"))?
    }

    fn write_human(&self, writer: &mut dyn Write) -> io::Result<()> {
        writeln!(
            writer,
            "snapshot: {} (Skript {}, Minecraft {}, {} plugins)",
            self.data.snapshot.id,
            self.data.snapshot.skript_version,
            self.data.snapshot.minecraft_version,
            self.data.snapshot.plugin_count,
        )?;
        match &self.data.result {
            ParseResultReport::Matched {
                effect,
                alternatives,
            } => {
                writeln!(writer, "effect:")?;
                write_identity(writer, &effect.syntax, 1)?;
                writeln!(writer, "  patternIndex: {}", effect.pattern.index)?;
                writeln!(writer, "  pattern: {}", effect.pattern.source)?;
                writeln!(writer, "  span: {}", effect.span)?;
                if let Some(handler) = &effect.handler {
                    writeln!(writer, "  handler: {handler}")?;
                }
                if !effect.metadata.is_empty() {
                    writeln!(writer, "  metadata: {:?}", effect.metadata)?;
                }
                writeln!(writer, "  patternElements:")?;
                write_pattern_elements(writer, &effect.pattern.elements, 2)?;
                writeln!(writer, "  elements:")?;
                if effect.elements.is_empty() {
                    writeln!(writer, "    []")?;
                } else {
                    write_resolved_elements(writer, &effect.elements, 2)?;
                }
                if !effect.tags.is_empty() {
                    writeln!(writer, "  parseTags:")?;
                    for tag in &effect.tags {
                        writeln!(
                            writer,
                            "    - {}{} at {}",
                            tag.value,
                            if tag.implicit { " (implicit)" } else { "" },
                            tag.span,
                        )?;
                    }
                }
                if !effect.marks.is_empty() {
                    writeln!(writer, "  parseMarks:")?;
                    for mark in &effect.marks {
                        writeln!(
                            writer,
                            "    - {} (accumulated {}) at {}",
                            mark.value, mark.accumulated, mark.span,
                        )?;
                    }
                }
                if !alternatives.is_empty() {
                    writeln!(writer, "alternatives:")?;
                    for alternative in alternatives {
                        writeln!(
                            writer,
                            "  - {} pattern[{}]: {}",
                            alternative.syntax.display_name(),
                            alternative.pattern_index,
                            alternative.pattern,
                        )?;
                    }
                }
            }
            ParseResultReport::Unknown { source, failure } => {
                writeln!(writer, "effect: unknown")?;
                writeln!(writer, "source: {source}")?;
                if let Some(failure) = failure {
                    writeln!(writer, "failure:")?;
                    writeln!(writer, "  offset: {}", failure.offset)?;
                    writeln!(writer, "  span: {}", failure.span)?;
                    for reason in &failure.reasons {
                        writeln!(writer, "  - {}", reason.human())?;
                    }
                }
            }
        }
        if !self.data.diagnostics.is_empty() {
            writeln!(writer, "diagnostics:")?;
            for diagnostic in &self.data.diagnostics {
                writeln!(
                    writer,
                    "  - [{}] {}: {} at {}",
                    diagnostic.severity, diagnostic.code, diagnostic.message, diagnostic.span,
                )?;
            }
        }
        if !self.data.component_failures.is_empty() {
            writeln!(writer, "componentFailures:")?;
            for failure in &self.data.component_failures {
                writeln!(
                    writer,
                    "  - {}/{}: {}",
                    failure.component_id, failure.subscription_id, failure.message,
                )?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReportData {
    schema_version: u32,
    input: String,
    snapshot: SnapshotReport,
    result: ParseResultReport,
    diagnostics: Vec<DiagnosticReport>,
    component_failures: Vec<ComponentFailureReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotReport {
    id: String,
    minecraft_version: String,
    skript_version: String,
    plugin_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum ParseResultReport {
    Matched {
        effect: Box<EffectReport>,
        alternatives: Vec<CandidateSummaryReport>,
    },
    Unknown {
        source: String,
        failure: Option<FailureReport>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EffectReport {
    syntax: SyntaxIdentityReport,
    pattern: PatternReport,
    span: SpanReport,
    elements: Vec<ResolvedElementReport>,
    tags: Vec<TagReport>,
    marks: Vec<MarkReport>,
    handler: Option<String>,
    metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CandidateSummaryReport {
    syntax: SyntaxIdentityReport,
    pattern_index: usize,
    pattern: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SyntaxIdentityReport {
    syntax_id: Option<String>,
    definition_id: String,
    registration_id: String,
    element_class: Option<String>,
    addon: Option<AddonReport>,
}

impl SyntaxIdentityReport {
    fn display_name(&self) -> &str {
        self.element_class
            .as_deref()
            .or(self.syntax_id.as_deref())
            .unwrap_or(&self.registration_id)
    }
}

#[derive(Debug, Clone, Serialize)]
struct AddonReport {
    name: String,
    version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PatternReport {
    index: usize,
    source: String,
    elements: Vec<PatternElementReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum PatternElementReport {
    Literal {
        span: SpanReport,
        text: String,
    },
    Choice {
        span: SpanReport,
        branches: Vec<Vec<PatternElementReport>>,
    },
    Group {
        span: SpanReport,
        elements: Vec<PatternElementReport>,
    },
    Option {
        span: SpanReport,
        elements: Vec<PatternElementReport>,
    },
    Regex {
        span: SpanReport,
        pattern: String,
    },
    Type {
        span: SpanReport,
        expression: TypeExpressionReport,
    },
    ParseTag {
        span: SpanReport,
        value: String,
    },
    ParseMark {
        span: SpanReport,
        value: i32,
    },
    Empty {
        span: SpanReport,
    },
    Truncated {
        span: SpanReport,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TypeExpressionReport {
    alternatives: Vec<ExpectedTypeReport>,
    nullable: bool,
    allow_literals: bool,
    allow_expressions: bool,
    time: i32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExpectedTypeReport {
    code_name: String,
    plural: bool,
    java_class: Option<String>,
    addon: Option<AddonReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum ResolvedElementReport {
    Regex {
        pattern_span: SpanReport,
        span: SpanReport,
        source: String,
        groups: Vec<RegexGroupReport>,
    },
    Expression {
        pattern_span: SpanReport,
        span: SpanReport,
        source: String,
        expected: TypeExpressionReport,
        selected_alternative: Option<usize>,
        resolution_id: Option<String>,
        resolved: Option<Box<ExpressionReport>>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RegexGroupReport {
    index: usize,
    value: Option<String>,
    span: Option<SpanReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExpressionReport {
    source: String,
    span: SpanReport,
    expression: ExpressionIdentityReport,
    return_type: Option<String>,
    multiplicity: Option<MultiplicityReport>,
    pattern: Option<PatternReport>,
    elements: Vec<ResolvedElementReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inner: Option<Box<ExpressionReport>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    arguments: Vec<FunctionArgumentReport>,
    metadata: BTreeMap<String, String>,
    truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FunctionArgumentReport {
    parameter_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    supplied_name: Option<String>,
    omitted: bool,
    values: Vec<ExpressionReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum ExpressionIdentityReport {
    Grouped,
    Registered {
        syntax: SyntaxIdentityReport,
    },
    Variable {
        parser_id: String,
    },
    Literal {
        parser_id: String,
    },
    Function {
        parser_id: String,
        structured: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        syntax: Option<SyntaxIdentityReport>,
    },
    Custom {
        parser_id: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum MultiplicityReport {
    Single,
    Multiple,
    Both,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TagReport {
    value: String,
    implicit: bool,
    span: SpanReport,
    pattern_span: SpanReport,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MarkReport {
    value: i32,
    accumulated: i32,
    span: SpanReport,
    pattern_span: SpanReport,
}

#[derive(Debug, Clone, Serialize)]
struct FailureReport {
    offset: usize,
    span: SpanReport,
    reasons: Vec<FailureReasonReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum FailureReasonReport {
    Literal { expected: String },
    Regex { pattern: String },
    TypeExpression { expected: Vec<String> },
    TrailingInput,
    HookRejected { reason: String },
}

impl FailureReasonReport {
    fn human(&self) -> String {
        match self {
            Self::Literal { expected } => format!("expected literal {expected:?}"),
            Self::Regex { pattern } => format!("expected regex <{pattern}>"),
            Self::TypeExpression { expected } => {
                format!("expected expression of type {}", expected.join(" or "))
            }
            Self::TrailingInput => "unexpected trailing input".to_owned(),
            Self::HookRejected { reason } => format!("hook rejected candidate: {reason}"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct DiagnosticReport {
    code: String,
    message: String,
    severity: String,
    span: SpanReport,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ComponentFailureReport {
    component_id: String,
    subscription_id: String,
    message: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct SpanReport {
    start: usize,
    end: usize,
}

impl std::fmt::Display for SpanReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}..{}", self.start, self.end)
    }
}

fn effect_report(input: &str, candidate: EffectCandidate, catalog: &Catalog) -> EffectReport {
    let EffectCandidate {
        matched,
        expressions,
        handler,
        metadata,
        ..
    } = candidate;
    let syntax = syntax_identity(&matched, catalog, SyntaxCategory::Effect);
    let pattern = pattern_report(matched.pattern_index, &matched.pattern, catalog, true);
    let span = match_span(&matched.matched.span);
    let elements = resolved_elements(&matched.matched.captures, &expressions, input, catalog, 0);
    let tags = matched.matched.tags.iter().map(tag_report).collect();
    let marks = matched.matched.marks.iter().map(mark_report).collect();
    EffectReport {
        syntax,
        pattern,
        span,
        elements,
        tags,
        marks,
        handler,
        metadata,
    }
}

fn candidate_summary(candidate: EffectCandidate, catalog: &Catalog) -> CandidateSummaryReport {
    CandidateSummaryReport {
        syntax: syntax_identity(&candidate.matched, catalog, SyntaxCategory::Effect),
        pattern_index: candidate.matched.pattern_index,
        pattern: candidate.matched.pattern,
    }
}

#[derive(Clone, Copy)]
enum SyntaxCategory {
    Effect,
    Expression,
}

fn syntax_identity(
    matched: &CandidateMatch,
    catalog: &Catalog,
    category: SyntaxCategory,
) -> SyntaxIdentityReport {
    syntax_identity_from_ids(
        &matched.definition_id,
        &matched.registration_id,
        catalog,
        category,
    )
}

fn syntax_identity_from_ids(
    definition_id: &str,
    registration_id: &str,
    catalog: &Catalog,
    category: SyntaxCategory,
) -> SyntaxIdentityReport {
    let common = catalog
        .syntax_by_registration_id(registration_id)
        .into_iter()
        .filter_map(|syntax| match (category, syntax) {
            (SyntaxCategory::Effect, Syntax::Effect(value)) => Some(&value.common),
            (SyntaxCategory::Expression, Syntax::Expression(value)) => Some(&value.common),
            _ => None,
        })
        .find(|common| common.definition_id.as_str() == definition_id);
    identity_from_common(definition_id, registration_id, common)
}

fn identity_from_common(
    definition_id: &str,
    registration_id: &str,
    common: Option<&CommonSyntax>,
) -> SyntaxIdentityReport {
    SyntaxIdentityReport {
        syntax_id: common.and_then(|value| value.id.clone()),
        definition_id: definition_id.to_owned(),
        registration_id: registration_id.to_owned(),
        element_class: common.map(|value| value.element_class.as_str().to_owned()),
        addon: common.map(|value| AddonReport {
            name: value.addon.name.clone(),
            version: value.addon.version.clone(),
        }),
    }
}

fn pattern_report(
    index: usize,
    source: &str,
    catalog: &Catalog,
    include_elements: bool,
) -> PatternReport {
    let elements = if include_elements {
        parse(source, catalog.plural_rules())
            .map(|parsed| pattern_elements(&parsed.elements, catalog, 0))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    PatternReport {
        index,
        source: source.to_owned(),
        elements,
    }
}

fn pattern_elements(
    elements: &[SpannedPatternElement],
    catalog: &Catalog,
    depth: usize,
) -> Vec<PatternElementReport> {
    elements
        .iter()
        .map(|element| pattern_element(element, catalog, depth))
        .collect()
}

fn pattern_element(
    element: &SpannedPatternElement,
    catalog: &Catalog,
    depth: usize,
) -> PatternElementReport {
    let span = pattern_span(element.span);
    if depth >= MAX_REPORT_PATTERN_DEPTH {
        return PatternElementReport::Truncated { span };
    }
    match &element.value {
        PatternElement::Literal(text) => PatternElementReport::Literal {
            span,
            text: text.clone(),
        },
        PatternElement::Choice(branches) => PatternElementReport::Choice {
            span,
            branches: branches
                .iter()
                .map(|branch| pattern_elements(branch, catalog, depth + 1))
                .collect(),
        },
        PatternElement::Group(elements) => PatternElementReport::Group {
            span,
            elements: pattern_elements(elements, catalog, depth + 1),
        },
        PatternElement::Option(elements) => PatternElementReport::Option {
            span,
            elements: pattern_elements(elements, catalog, depth + 1),
        },
        PatternElement::Regex(pattern) => PatternElementReport::Regex {
            span,
            pattern: pattern.clone(),
        },
        PatternElement::TypeExpr(expression) => PatternElementReport::Type {
            span,
            expression: type_expression(expression, catalog),
        },
        PatternElement::ParseTag(value) => PatternElementReport::ParseTag {
            span,
            value: value.clone(),
        },
        PatternElement::ParseMark(value) => PatternElementReport::ParseMark {
            span,
            value: *value,
        },
        PatternElement::Empty => PatternElementReport::Empty { span },
    }
}
fn type_expression(expression: &PatternTypeExpr, catalog: &Catalog) -> TypeExpressionReport {
    TypeExpressionReport {
        alternatives: expression
            .alternatives
            .iter()
            .map(|alternative| {
                let registered_type = catalog.type_by_code_name(&alternative.name);
                ExpectedTypeReport {
                    code_name: alternative.name.clone(),
                    plural: alternative.plural,
                    java_class: registered_type
                        .map(|value| value.original_class.as_str().to_owned()),
                    addon: registered_type.map(|value| AddonReport {
                        name: value.addon.name.clone(),
                        version: value.addon.version.clone(),
                    }),
                }
            })
            .collect(),
        nullable: expression.nullable,
        allow_literals: expression.allow_literals,
        allow_expressions: expression.allow_expressions,
        time: expression.time,
    }
}

fn resolved_elements(
    captures: &[PatternCapture],
    expressions: &[ExpressionNode],
    input: &str,
    catalog: &Catalog,
    depth: usize,
) -> Vec<ResolvedElementReport> {
    let mut resolved = expressions.iter();
    captures
        .iter()
        .map(|capture| match capture {
            PatternCapture::Regex {
                pattern_span: capture_pattern_span,
                value,
                span,
                groups,
            } => ResolvedElementReport::Regex {
                pattern_span: pattern_span(*capture_pattern_span),
                span: match_span(span),
                source: value.clone(),
                groups: groups
                    .iter()
                    .map(|group| RegexGroupReport {
                        index: group.index,
                        value: group.value.clone(),
                        span: group.span.as_ref().map(match_span),
                    })
                    .collect(),
            },
            PatternCapture::TypeExpression {
                pattern_span: capture_pattern_span,
                expression,
                value,
                span,
                alternative_index,
                resolution_id,
            } => {
                let expression_node = resolution_id.as_ref().and_then(|_| resolved.next());
                ResolvedElementReport::Expression {
                    pattern_span: pattern_span(*capture_pattern_span),
                    span: match_span(span),
                    source: value.clone(),
                    expected: type_expression(expression, catalog),
                    selected_alternative: *alternative_index,
                    resolution_id: resolution_id.clone(),
                    resolved: expression_node
                        .map(|node| Box::new(expression_report(node, input, catalog, depth + 1))),
                }
            }
        })
        .collect()
}

fn expression_report(
    node: &ExpressionNode,
    input: &str,
    catalog: &Catalog,
    depth: usize,
) -> ExpressionReport {
    let (expression, pattern) = match &node.kind {
        ExpressionNodeKind::Grouped => (ExpressionIdentityReport::Grouped, None),
        ExpressionNodeKind::Registered {
            definition_id,
            registration_id,
            pattern_index,
        } => {
            let identity = syntax_identity_from_ids(
                definition_id,
                registration_id,
                catalog,
                SyntaxCategory::Expression,
            );
            let pattern_source = catalog
                .syntax_by_registration_id(registration_id)
                .into_iter()
                .filter_map(|syntax| match syntax {
                    Syntax::Expression(value)
                        if value.common.definition_id.as_str() == definition_id =>
                    {
                        Some(&value.common)
                    }
                    _ => None,
                })
                .find_map(|common| common.patterns.get(*pattern_index))
                .map(|pattern| pattern.source.as_str());
            (
                ExpressionIdentityReport::Registered { syntax: identity },
                pattern_source.map(|source| pattern_report(*pattern_index, source, catalog, false)),
            )
        }
        ExpressionNodeKind::Variable { parser_id } => (
            ExpressionIdentityReport::Variable {
                parser_id: parser_id.clone(),
            },
            None,
        ),
        ExpressionNodeKind::Literal { parser_id } => (
            ExpressionIdentityReport::Literal {
                parser_id: parser_id.clone(),
            },
            None,
        ),
        ExpressionNodeKind::Function { parser_id } => {
            let function = node.function.as_ref();
            (
                ExpressionIdentityReport::Function {
                    parser_id: parser_id.clone(),
                    structured: function.is_some(),
                    name: function.map(|function| function.name.clone()),
                    syntax: function.map(|function| {
                        function_identity(
                            &function.definition_id,
                            &function.registration_id,
                            catalog,
                        )
                    }),
                },
                None,
            )
        }
        ExpressionNodeKind::Custom { parser_id } => (
            ExpressionIdentityReport::Custom {
                parser_id: parser_id.clone(),
            },
            None,
        ),
    };
    let span = match_span(&node.span);
    let truncated = depth >= MAX_REPORT_EXPRESSION_DEPTH;
    let inner = (!truncated && matches!(node.kind, ExpressionNodeKind::Grouped))
        .then(|| node.children.first())
        .flatten()
        .map(|node| Box::new(expression_report(node, input, catalog, depth + 1)));
    let arguments = if truncated {
        Vec::new()
    } else {
        node.function
            .iter()
            .flat_map(|function| &function.arguments)
            .map(|argument| {
                let end = argument.child_start.saturating_add(argument.child_count);
                let values = node
                    .children
                    .get(argument.child_start..end)
                    .unwrap_or_default()
                    .iter()
                    .map(|child| expression_report(child, input, catalog, depth + 1))
                    .collect();
                FunctionArgumentReport {
                    parameter_name: argument.parameter_name.clone(),
                    supplied_name: argument.supplied_name.clone(),
                    omitted: argument.omitted,
                    values,
                }
            })
            .collect()
    };
    ExpressionReport {
        source: source_slice(input, node.span.mapped.virtual_range),
        span,
        expression,
        return_type: node
            .return_type
            .as_ref()
            .map(|return_type| return_type.as_str().to_owned()),
        multiplicity: node.multiplicity.map(multiplicity),
        pattern,
        elements: if truncated {
            Vec::new()
        } else {
            resolved_elements(&node.captures, &node.children, input, catalog, depth)
        },
        inner,
        arguments,
        metadata: node.metadata.clone(),
        truncated,
    }
}

fn function_identity(
    definition_id: &str,
    registration_id: &str,
    catalog: &Catalog,
) -> SyntaxIdentityReport {
    let function = catalog
        .syntax_by_registration_id(registration_id)
        .into_iter()
        .find_map(|syntax| match syntax {
            Syntax::Function(function) if function.definition_id.as_str() == definition_id => {
                Some(function)
            }
            _ => None,
        });
    SyntaxIdentityReport {
        syntax_id: None,
        definition_id: definition_id.to_owned(),
        registration_id: registration_id.to_owned(),
        element_class: None,
        addon: function.map(|function| AddonReport {
            name: function.addon.name.clone(),
            version: function.addon.version.clone(),
        }),
    }
}

fn multiplicity(value: Multiplicity) -> MultiplicityReport {
    match value {
        Multiplicity::Single => MultiplicityReport::Single,
        Multiplicity::Multiple => MultiplicityReport::Multiple,
        Multiplicity::Both => MultiplicityReport::Both,
    }
}

fn tag_report(tag: &ParseTagCapture) -> TagReport {
    TagReport {
        value: tag.value.clone(),
        implicit: tag.implicit,
        span: match_span(&tag.input_span),
        pattern_span: pattern_span(tag.pattern_span),
    }
}

fn mark_report(mark: &ParseMarkCapture) -> MarkReport {
    MarkReport {
        value: mark.value,
        accumulated: mark.accumulated,
        span: match_span(&mark.input_span),
        pattern_span: pattern_span(mark.pattern_span),
    }
}

fn failure_report(failure: PatternFailure) -> FailureReport {
    FailureReport {
        offset: failure.offset,
        span: match_span(&failure.span),
        reasons: failure
            .reasons
            .into_iter()
            .map(|reason| match reason {
                PatternFailureReason::Literal { expected } => {
                    FailureReasonReport::Literal { expected }
                }
                PatternFailureReason::Regex { pattern } => FailureReasonReport::Regex { pattern },
                PatternFailureReason::TypeExpression { expected } => {
                    FailureReasonReport::TypeExpression { expected }
                }
                PatternFailureReason::TrailingInput => FailureReasonReport::TrailingInput,
                PatternFailureReason::HookRejected { reason } => {
                    FailureReasonReport::HookRejected { reason }
                }
            })
            .collect(),
    }
}

fn match_span(span: &MatchSpan) -> SpanReport {
    text_span(span.mapped.virtual_range)
}

fn text_span(span: TextRange) -> SpanReport {
    SpanReport {
        start: span.start,
        end: span.end,
    }
}

fn pattern_span(span: PatternSpan) -> SpanReport {
    SpanReport {
        start: span.start,
        end: span.end,
    }
}

fn source_slice(input: &str, span: TextRange) -> String {
    span.slice(input).unwrap_or_default().to_owned()
}

fn write_identity(
    writer: &mut dyn Write,
    identity: &SyntaxIdentityReport,
    indent: usize,
) -> io::Result<()> {
    let prefix = "  ".repeat(indent);
    if let Some(syntax_id) = &identity.syntax_id {
        writeln!(writer, "{prefix}syntaxId: {syntax_id}")?;
    }
    if let Some(element_class) = &identity.element_class {
        writeln!(writer, "{prefix}class: {element_class}")?;
    }
    if let Some(addon) = &identity.addon {
        writeln!(writer, "{prefix}addon: {} {}", addon.name, addon.version)?;
    }
    writeln!(writer, "{prefix}definitionId: {}", identity.definition_id)?;
    writeln!(
        writer,
        "{prefix}registrationId: {}",
        identity.registration_id
    )
}

fn write_pattern_elements(
    writer: &mut dyn Write,
    elements: &[PatternElementReport],
    indent: usize,
) -> io::Result<()> {
    if elements.is_empty() {
        writeln!(writer, "{}[]", "  ".repeat(indent))?;
        return Ok(());
    }
    for element in elements {
        let prefix = "  ".repeat(indent);
        match element {
            PatternElementReport::Literal { span, text } => {
                writeln!(writer, "{prefix}- literal {text:?} at {span}")?;
            }
            PatternElementReport::Choice { span, branches } => {
                writeln!(writer, "{prefix}- choice at {span}")?;
                for (index, branch) in branches.iter().enumerate() {
                    writeln!(writer, "{prefix}  branch[{index}]:")?;
                    write_pattern_elements(writer, branch, indent + 2)?;
                }
            }
            PatternElementReport::Group { span, elements } => {
                writeln!(writer, "{prefix}- group at {span}")?;
                write_pattern_elements(writer, elements, indent + 1)?;
            }
            PatternElementReport::Option { span, elements } => {
                writeln!(writer, "{prefix}- option at {span}")?;
                write_pattern_elements(writer, elements, indent + 1)?;
            }
            PatternElementReport::Regex { span, pattern } => {
                writeln!(writer, "{prefix}- regex <{pattern}> at {span}")?;
            }
            PatternElementReport::Type { span, expression } => {
                writeln!(
                    writer,
                    "{prefix}- type {} at {span}",
                    format_expected_types(expression)
                )?;
            }
            PatternElementReport::ParseTag { span, value } => {
                writeln!(writer, "{prefix}- parseTag {value:?} at {span}")?;
            }
            PatternElementReport::ParseMark { span, value } => {
                writeln!(writer, "{prefix}- parseMark {value} at {span}")?;
            }
            PatternElementReport::Empty { span } => {
                writeln!(writer, "{prefix}- empty at {span}")?;
            }
            PatternElementReport::Truncated { span } => {
                writeln!(writer, "{prefix}- truncated at {span}")?;
            }
        }
    }
    Ok(())
}

fn write_resolved_elements(
    writer: &mut dyn Write,
    elements: &[ResolvedElementReport],
    indent: usize,
) -> io::Result<()> {
    let prefix = "  ".repeat(indent);
    for element in elements {
        match element {
            ResolvedElementReport::Regex { source, span, .. } => {
                writeln!(writer, "{prefix}- regex {source:?} at {span}")?;
            }
            ResolvedElementReport::Expression {
                source,
                span,
                expected,
                selected_alternative,
                resolved,
                ..
            } => {
                writeln!(writer, "{prefix}- expression {source:?} at {span}")?;
                writeln!(
                    writer,
                    "{prefix}  expectedTypes: {}",
                    format_expected_types(expected)
                )?;
                if let Some(index) = selected_alternative {
                    writeln!(writer, "{prefix}  selectedAlternative: {index}")?;
                }
                if let Some(resolved) = resolved {
                    write_expression(writer, resolved, indent + 1)?;
                } else {
                    writeln!(writer, "{prefix}  resolved: null")?;
                }
            }
        }
    }
    Ok(())
}

fn write_expression(
    writer: &mut dyn Write,
    expression: &ExpressionReport,
    indent: usize,
) -> io::Result<()> {
    let prefix = "  ".repeat(indent);
    match &expression.expression {
        ExpressionIdentityReport::Grouped => {
            writeln!(writer, "{prefix}resolved: groupedExpression")?;
        }
        ExpressionIdentityReport::Registered { syntax } => {
            writeln!(writer, "{prefix}resolved: registeredExpression")?;
            write_identity(writer, syntax, indent + 1)?;
        }
        ExpressionIdentityReport::Variable { parser_id } => {
            writeln!(writer, "{prefix}resolved: variable ({parser_id})")?;
        }
        ExpressionIdentityReport::Literal { parser_id } => {
            writeln!(writer, "{prefix}resolved: literal ({parser_id})")?;
        }
        ExpressionIdentityReport::Function {
            parser_id,
            structured,
            name,
            syntax,
        } => {
            writeln!(
                writer,
                "{prefix}resolved: function ({parser_id}, structured={structured})"
            )?;
            if let Some(name) = name {
                writeln!(writer, "{prefix}name: {name}")?;
            }
            if let Some(syntax) = syntax {
                write_identity(writer, syntax, indent + 1)?;
            }
        }
        ExpressionIdentityReport::Custom { parser_id } => {
            writeln!(writer, "{prefix}resolved: custom ({parser_id})")?;
        }
    }
    writeln!(writer, "{prefix}source: {:?}", expression.source)?;
    if let Some(return_type) = &expression.return_type {
        writeln!(writer, "{prefix}returnType: {return_type}")?;
    }
    if let Some(multiplicity) = expression.multiplicity {
        writeln!(writer, "{prefix}multiplicity: {multiplicity:?}")?;
    }
    if let Some(pattern) = &expression.pattern {
        writeln!(
            writer,
            "{prefix}pattern[{}]: {}",
            pattern.index, pattern.source
        )?;
    }
    if expression.truncated {
        writeln!(writer, "{prefix}elements: truncated")?;
    } else if !expression.arguments.is_empty() {
        writeln!(writer, "{prefix}arguments:")?;
        for argument in &expression.arguments {
            writeln!(writer, "{prefix}  {}:", argument.parameter_name)?;
            if let Some(supplied_name) = &argument.supplied_name {
                writeln!(writer, "{prefix}    suppliedName: {supplied_name}")?;
            }
            if argument.omitted {
                writeln!(writer, "{prefix}    omitted: true")?;
            } else {
                for (index, value) in argument.values.iter().enumerate() {
                    writeln!(writer, "{prefix}    value[{index}]:")?;
                    write_expression(writer, value, indent + 3)?;
                }
            }
        }
    } else if let Some(inner) = &expression.inner {
        writeln!(writer, "{prefix}inner:")?;
        write_expression(writer, inner, indent + 1)?;
    } else if !expression.elements.is_empty() {
        writeln!(writer, "{prefix}elements:")?;
        write_resolved_elements(writer, &expression.elements, indent + 1)?;
    }
    Ok(())
}

fn format_expected_types(expression: &TypeExpressionReport) -> String {
    if expression.alternatives.is_empty() {
        return "[]".to_owned();
    }
    expression
        .alternatives
        .iter()
        .map(|expected| {
            let plurality = if expected.plural { "[]" } else { "" };
            expected.java_class.as_ref().map_or_else(
                || format!("{}{}", expected.code_name, plurality),
                |class| format!("{}{} ({class})", expected.code_name, plurality),
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_reason_human_text_is_actionable() {
        assert_eq!(
            FailureReasonReport::TypeExpression {
                expected: vec!["string".to_owned(), "number".to_owned()],
            }
            .human(),
            "expected expression of type string or number"
        );
    }
}

use crate::{EventContext, OutputFormat};
use miette::{GraphicalReportHandler, GraphicalTheme, LabeledSpan, MietteDiagnostic, NamedSource};
use parser_wasm::host::WasmEffectParseResult;
use serde::Serialize;
use skript_parser::{
    CandidateMatch, ConditionNode, EffectCandidate, EffectCandidateFailure,
    ExpressionListConjunction, ExpressionNode, ExpressionNodeKind, ExpressionPublicData,
    FailureFrameRole, FailureTrace, MatchSpan, MatchSyntaxKind, ParseMarkCapture, ParseTagCapture,
    ParsedCapture, ParsedCaptureValue, PatternCapture, PatternFailure, PatternFailureReason,
    TextRange,
};
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::time::Duration;
use syntax_pattern_parser::syntax::{
    PatternElement, PatternTypeExpr, Span as PatternSpan, SpannedPatternElement, parse,
};
use syntaxes::{Catalog, CommonSyntax, Multiplicity, Syntax};

const REPORT_SCHEMA_VERSION: u32 = 5;
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
/// versions the utility's JSON contract separately from the parser. Reports
/// include structured Function calls, recursive captures, and the selected
/// syntax identity, including Section identities for one-line EffectSections.
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
        parse_duration: Duration,
        event_context: Option<&EventContext>,
    ) -> Self {
        let diagnostics = result
            .effects
            .diagnostics
            .into_iter()
            .map(|diagnostic| DiagnosticReport {
                code: diagnostic.code,
                message: diagnostic.message,
                severity: format!("{:?}", diagnostic.severity)
                    .rsplit("::")
                    .next()
                    .expect("split always returns one segment")
                    .to_ascii_lowercase(),
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
            let unknown = result.matches.unknown;
            let source = unknown
                .as_ref()
                .map_or_else(|| input.to_owned(), |unknown| unknown.source.clone());
            if let Some(candidate) = unknown
                .as_ref()
                .and_then(|unknown| unknown.failures.primary())
            {
                let mut syntax = syntax_identity_from_ids(
                    &candidate.matched.definition_id,
                    &candidate.matched.registration_id,
                    catalog,
                    effect_syntax_category(candidate.matched.kind),
                );
                if syntax.element_class.is_none() {
                    syntax.element_class = candidate
                        .element_class
                        .as_ref()
                        .map(|class| class.as_str().to_owned());
                }
                ParseResultReport::Incomplete {
                    effect: Box::new(IncompleteEffectReport {
                        syntax,
                        pattern_index: candidate.matched.pattern_index,
                        pattern: candidate.matched.pattern.clone(),
                        priority: candidate.matched.priority,
                        registration_order: candidate.matched.registration_order,
                        resolved_order: candidate.matched.resolved_order,
                        handler: candidate.handler.clone(),
                        metadata: candidate.metadata.clone(),
                    }),
                    source,
                    failure: candidate_failure_report(
                        candidate,
                        unknown
                            .as_ref()
                            .map_or(&[][..], |unknown| unknown.failures.candidates.as_slice()),
                        catalog,
                    ),
                }
            } else {
                ParseResultReport::Unknown {
                    source,
                    failure: unknown
                        .and_then(|unknown| unknown.failures.fallback)
                        .map(failure_trace_report),
                }
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
                context: AnalysisContextReport {
                    event: event_context.map(EventContextReport::from),
                },
                parse_duration_ns: u64::try_from(parse_duration.as_nanos()).unwrap_or(u64::MAX),
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
        String::from_utf8(self.render(OutputFormat::Json, false)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// Writes this report using the selected terminal or JSON representation.
    pub fn write(self, format: OutputFormat, writer: impl Write) -> io::Result<()> {
        self.write_with_color(format, writer, false)
    }

    pub(crate) fn write_with_color(
        self,
        format: OutputFormat,
        mut writer: impl Write,
        color: bool,
    ) -> io::Result<()> {
        writer.write_all(&self.render(format, color)?)
    }

    fn render(self, format: OutputFormat, color: bool) -> io::Result<Vec<u8>> {
        let worker = std::thread::Builder::new()
            .name("effectcommandcli-report".to_owned())
            .stack_size(32 * 1024 * 1024)
            .spawn(move || {
                let mut output = Vec::new();
                match format {
                    OutputFormat::Human => self.write_human(&mut output, color)?,
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

    fn write_human(&self, writer: &mut dyn Write, color: bool) -> io::Result<()> {
        writeln!(
            writer,
            "snapshot: {} (Skript {}, Minecraft {}, {} plugins)",
            self.data.snapshot.id,
            self.data.snapshot.skript_version,
            self.data.snapshot.minecraft_version,
            self.data.snapshot.plugin_count,
        )?;
        writeln!(
            writer,
            "parseTime: {}",
            format_parse_duration(self.data.parse_duration_ns)
        )?;
        if let Some(event) = &self.data.context.event {
            writeln!(writer, "context:")?;
            writeln!(writer, "  event: {}", event.input)?;
            writeln!(
                writer,
                "  class: {}",
                event.element_class.as_deref().unwrap_or("dynamic")
            )?;
            if let Some(addon) = &event.addon {
                writeln!(writer, "  addon: {} {}", addon.name, addon.version)?;
            }
            writeln!(writer, "  definitionId: {}", event.definition_id)?;
            writeln!(writer, "  registrationId: {}", event.registration_id)?;
            writeln!(writer, "  patternIndex: {}", event.pattern_index)?;
            writeln!(writer, "  pattern: {}", event.pattern)?;
            writeln!(writer, "  referenceEvents: {:?}", event.reference_events)?;
            if event.registered_reference_events != event.reference_events {
                writeln!(
                    writer,
                    "  registeredReferenceEvents: {:?}",
                    event.registered_reference_events
                )?;
            }
            if let Some(listening_behavior) = &event.listening_behavior {
                writeln!(writer, "  listeningBehavior: {listening_behavior}")?;
            }
            if let Some(priority) = &event.event_priority {
                writeln!(writer, "  eventPriority: {priority}")?;
            }
            writeln!(writer, "  eventValues: {}", event.event_values.len())?;
            for diagnostic in &event.diagnostics {
                writeln!(
                    writer,
                    "  diagnostic: [{}] {}: {}",
                    diagnostic.severity, diagnostic.code, diagnostic.message
                )?;
            }
            for failure in &event.component_failures {
                writeln!(
                    writer,
                    "  componentFailure: {}/{}: {}",
                    failure.component_id, failure.subscription_id, failure.message
                )?;
            }
        }
        match &self.data.result {
            ParseResultReport::Matched {
                effect,
                alternatives,
            } => {
                writeln!(
                    writer,
                    "source: {}",
                    render_matched_source(&self.data.input, effect, color)
                )?;
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
            ParseResultReport::Incomplete {
                effect,
                source,
                failure,
            } => {
                writeln!(writer, "effect:")?;
                writeln!(writer, "  status: incomplete")?;
                write_identity(writer, &effect.syntax, 1)?;
                if let Some(index) = effect.pattern_index {
                    writeln!(writer, "  patternIndex: {index}")?;
                }
                if let Some(pattern) = &effect.pattern {
                    writeln!(writer, "  pattern: {pattern}")?;
                }
                writeln!(writer, "  priority: {}", effect.priority)?;
                writeln!(writer, "  registrationOrder: {}", effect.registration_order)?;
                if let Some(order) = effect.resolved_order {
                    writeln!(writer, "  resolvedOrder: {order}")?;
                }
                if let Some(handler) = &effect.handler {
                    writeln!(writer, "  handler: {handler}")?;
                }
                if !effect.metadata.is_empty() {
                    writeln!(writer, "  metadata: {:?}", effect.metadata)?;
                }
                write_failure(
                    writer,
                    source,
                    failure,
                    color,
                    "Effect candidate is incomplete",
                )?;
            }
            ParseResultReport::Unknown { source, failure } => {
                writeln!(writer, "effect: unknown")?;
                if let Some(failure) = failure {
                    write_failure(writer, source, failure, color, "No Effect matched")?;
                } else {
                    writeln!(writer, "source: {source}")?;
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
    context: AnalysisContextReport,
    parse_duration_ns: u64,
    result: ParseResultReport,
    diagnostics: Vec<DiagnosticReport>,
    component_failures: Vec<ComponentFailureReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AnalysisContextReport {
    event: Option<EventContextReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EventContextReport {
    input: String,
    definition_id: String,
    registration_id: String,
    pattern_index: usize,
    pattern: String,
    element_class: Option<String>,
    reference_events: Vec<String>,
    registered_reference_events: Vec<String>,
    cancellable: Option<bool>,
    priority_supported: Option<bool>,
    listening_behavior: Option<String>,
    event_priority: Option<String>,
    addon: Option<EventAddonReport>,
    handler: Option<String>,
    event_metadata: BTreeMap<String, String>,
    structure_metadata: BTreeMap<String, String>,
    event_values: Vec<EventValueReport>,
    diagnostics: Vec<EventContextDiagnosticReport>,
    component_failures: Vec<EventContextComponentFailureReport>,
    capture_count: usize,
}

impl From<&EventContext> for EventContextReport {
    fn from(event: &EventContext) -> Self {
        Self {
            input: event.input.clone(),
            definition_id: event.definition_id.clone(),
            registration_id: event.registration_id.clone(),
            pattern_index: event.pattern_index,
            pattern: event.pattern.clone(),
            element_class: event.element_class.clone(),
            reference_events: event.reference_events.clone(),
            registered_reference_events: event.registered_reference_events.clone(),
            cancellable: event.cancellable,
            priority_supported: event.priority_supported,
            listening_behavior: event.listening_behavior.clone(),
            event_priority: event.event_priority.clone(),
            addon: event.addon.as_ref().map(|addon| EventAddonReport {
                name: addon.name.clone(),
                version: addon.version.clone(),
            }),
            handler: event.handler.clone(),
            event_metadata: event.event_metadata.clone(),
            structure_metadata: event.structure_metadata.clone(),
            event_values: event
                .event_values
                .iter()
                .map(|value| EventValueReport {
                    event_class: value.event_class.clone(),
                    value_class: value.value_class.clone(),
                    time: value.time,
                    exclude_error_message: value.exclude_error_message.clone(),
                    excludes: value.excludes.clone(),
                    resolution_order: value.resolution_order,
                    registration_order: value.registration_order,
                    registration_id: value.registration_id.clone(),
                    patterns: value.patterns.clone(),
                    accepted_changers: value.accepted_changers.as_ref().map(|changers| {
                        changers
                            .iter()
                            .map(|changer| EventValueChangerReport {
                                mode: changer.mode.clone(),
                                accepted_classes: changer.accepted_classes.clone(),
                            })
                            .collect()
                    }),
                    context_dependent: value.context_dependent,
                    has_custom_input_validator: value.has_custom_input_validator,
                    has_custom_event_validator: value.has_custom_event_validator,
                    addon: EventAddonReport {
                        name: value.addon.name.clone(),
                        version: value.addon.version.clone(),
                    },
                })
                .collect(),
            diagnostics: event
                .diagnostics
                .iter()
                .map(|diagnostic| EventContextDiagnosticReport {
                    code: diagnostic.code.clone(),
                    message: diagnostic.message.clone(),
                    severity: diagnostic.severity.clone(),
                })
                .collect(),
            component_failures: event
                .component_failures
                .iter()
                .map(|failure| EventContextComponentFailureReport {
                    component_id: failure.component_id.clone(),
                    subscription_id: failure.subscription_id.clone(),
                    message: failure.message.clone(),
                })
                .collect(),
            capture_count: event.captures().len(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EventAddonReport {
    name: String,
    version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EventValueReport {
    event_class: String,
    value_class: String,
    time: i32,
    exclude_error_message: Option<String>,
    excludes: Option<Vec<String>>,
    resolution_order: usize,
    registration_order: Option<usize>,
    registration_id: String,
    patterns: Option<Vec<String>>,
    accepted_changers: Option<Vec<EventValueChangerReport>>,
    context_dependent: Option<bool>,
    has_custom_input_validator: Option<bool>,
    has_custom_event_validator: Option<bool>,
    addon: EventAddonReport,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EventValueChangerReport {
    mode: String,
    accepted_classes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EventContextDiagnosticReport {
    code: String,
    message: String,
    severity: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EventContextComponentFailureReport {
    component_id: String,
    subscription_id: String,
    message: String,
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
    Incomplete {
        effect: Box<IncompleteEffectReport>,
        source: String,
        failure: FailureReport,
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
    #[serde(skip)]
    source_colors: Vec<SourceColorSpan>,
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    operands: Vec<ExpressionReport>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    items: Vec<ExpressionReport>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    embedded_expressions: Vec<ExpressionReport>,
    public_data: Vec<ExpressionPublicDataReport>,
    metadata: BTreeMap<String, String>,
    truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExpressionPublicDataReport {
    schema_id: String,
    schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    json: Option<Box<serde_json::value::RawValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw_json: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
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
    List {
        conjunction: ExpressionListConjunctionReport,
    },
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
    Arithmetic {
        operator: String,
        operation_registration_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        addon: Option<AddonReport>,
    },
    Custom {
        parser_id: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum ExpressionListConjunctionReport {
    And,
    Or,
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
    contexts: Vec<FailureContextReport>,
    related: Vec<FailureReport>,
    interpretations: Vec<FailureInterpretationReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FailureContextReport {
    syntax_kind: String,
    definition_id: String,
    registration_id: String,
    pattern_index: usize,
    pattern: String,
    role: FailureContextRoleReport,
    span: SpanReport,
    pattern_span: Option<SpanReport>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum FailureContextRoleReport {
    SemanticCandidate,
    PatternElement,
    ExpressionCapture { index: usize },
    ConditionCapture { index: usize },
    EffectCapture { index: usize },
}

impl FailureContextRoleReport {
    fn human(self, syntax_kind: &str) -> String {
        match self {
            Self::SemanticCandidate => format!("semantic check in {syntax_kind}"),
            Self::PatternElement => format!("typed capture in {syntax_kind}"),
            Self::ExpressionCapture { .. } => format!("expression capture in {syntax_kind}"),
            Self::ConditionCapture { .. } => format!("condition capture in {syntax_kind}"),
            Self::EffectCapture { .. } => format!("effect capture in {syntax_kind}"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FailureInterpretationReport {
    syntax: SyntaxIdentityReport,
    pattern_index: Option<usize>,
    pattern: Option<String>,
    span: SpanReport,
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum FailureReasonReport {
    Literal {
        expected: String,
    },
    Regex {
        pattern: String,
    },
    Expression,
    TypeExpression {
        expected: Vec<String>,
    },
    EventRestricted {
        supported: Vec<String>,
        current: Vec<String>,
    },
    TrailingInput,
    HookRejected {
        reason: String,
    },
}

impl FailureReasonReport {
    fn human(&self) -> String {
        match self {
            Self::Literal { expected } => format!("expected literal {expected:?}"),
            Self::Regex { pattern } => format!("expected regex <{pattern}>"),
            Self::Expression => "could not parse an expression".to_owned(),
            Self::TypeExpression { expected } => {
                format!("expected expression of type {}", expected.join(" or "))
            }
            Self::EventRestricted { supported, current } if current.is_empty() => {
                format!("requires event context: {}", supported.join(" or "))
            }
            Self::EventRestricted { supported, current } => format!(
                "only available in {}; current event is {}",
                supported.join(" or "),
                current.join(" or ")
            ),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceColor {
    Effect,
    Expression,
    Condition,
    Variable,
    Literal,
    TypeName,
    Alias,
    Function,
}

impl SourceColor {
    fn ansi_code(self) -> &'static str {
        match self {
            Self::Effect => "38;2;88;196;221",
            Self::Expression => "38;2;131;193;103",
            Self::Condition => "38;2;252;98;85",
            Self::Variable => "38;2;0;255;255",
            Self::Literal => "38;2;255;255;255",
            Self::TypeName => "38;2;255;134;47",
            Self::Alias => "38;2;240;172;95",
            Self::Function => "38;2;160;160;160",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SourceColorSpan {
    span: SpanReport,
    color: SourceColor,
    depth: usize,
    order: usize,
}

fn render_matched_source(input: &str, effect: &EffectReport, color: bool) -> String {
    if !color {
        return input.to_owned();
    }

    render_source_colors(input, &effect.source_colors)
}

fn collect_effect_colors(effect: &EffectCandidate, spans: &mut Vec<SourceColorSpan>, depth: usize) {
    push_source_color_span(
        spans,
        match_span(&effect.matched.matched.span),
        SourceColor::Effect,
        depth,
    );
    collect_parsed_capture_colors(&effect.parsed_captures, spans, depth + 1);
}

fn collect_condition_colors(
    condition: &ConditionNode,
    spans: &mut Vec<SourceColorSpan>,
    depth: usize,
) {
    push_source_color_span(
        spans,
        match_span(&condition.span),
        SourceColor::Condition,
        depth,
    );
    for expression in &condition.expressions {
        collect_expression_node_colors(expression, spans, depth + 1);
    }
    for child in &condition.children {
        collect_condition_colors(child, spans, depth + 1);
    }
}

fn collect_expression_node_colors(
    expression: &ExpressionNode,
    spans: &mut Vec<SourceColorSpan>,
    depth: usize,
) {
    let color = match &expression.kind {
        ExpressionNodeKind::Variable { .. } => SourceColor::Variable,
        ExpressionNodeKind::Literal { parser_id } => {
            literal_source_color(parser_id, &expression.metadata)
        }
        ExpressionNodeKind::Function { .. } => SourceColor::Function,
        _ => SourceColor::Expression,
    };
    push_source_color_span(spans, match_span(&expression.span), color, depth);

    if let Some(alias_span) = literal_alias_span(&expression.metadata, match_span(&expression.span))
    {
        push_source_color_span(spans, alias_span, SourceColor::Alias, depth + 1);
    }

    collect_parsed_capture_colors(&expression.parsed_captures(), spans, depth + 1);
}

fn collect_parsed_capture_colors(
    captures: &[ParsedCapture],
    spans: &mut Vec<SourceColorSpan>,
    depth: usize,
) {
    for capture in captures {
        match capture.result.value.as_ref() {
            Some(ParsedCaptureValue::Expression(expression)) => {
                collect_expression_node_colors(expression, spans, depth)
            }
            Some(ParsedCaptureValue::Condition(condition)) => {
                collect_condition_colors(condition, spans, depth)
            }
            Some(ParsedCaptureValue::Effect(effect)) => collect_effect_colors(effect, spans, depth),
            Some(ParsedCaptureValue::Section(section)) => {
                collect_parsed_capture_colors(&section.parsed_captures, spans, depth)
            }
            Some(ParsedCaptureValue::Event(_) | ParsedCaptureValue::Raw(_)) | None => {}
        }
    }
}

fn literal_alias_span(
    metadata: &BTreeMap<String, String>,
    fallback: SpanReport,
) -> Option<SpanReport> {
    (metadata_value(metadata, "literal-source") == Some("alias")).then(|| {
        metadata_value(metadata, "literal-range-start")
            .and_then(|start| start.parse().ok())
            .zip(metadata_value(metadata, "literal-range-end").and_then(|end| end.parse().ok()))
            .map(|(start, end)| SpanReport { start, end })
            .unwrap_or(fallback)
    })
}

fn literal_source_color(parser_id: &str, metadata: &BTreeMap<String, String>) -> SourceColor {
    if parser_id == "core.literal.class-info"
        || metadata_value(metadata, "type-code-name") == Some("classinfo")
    {
        SourceColor::TypeName
    } else {
        SourceColor::Literal
    }
}

fn metadata_value<'a>(metadata: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    metadata.get(key).map(String::as_str).or_else(|| {
        metadata.iter().find_map(|(candidate, value)| {
            candidate
                .strip_suffix(key)
                .is_some_and(|owner| owner.ends_with('/'))
                .then_some(value.as_str())
        })
    })
}

fn push_source_color_span(
    spans: &mut Vec<SourceColorSpan>,
    span: SpanReport,
    color: SourceColor,
    depth: usize,
) {
    if span.start < span.end {
        let order = spans.len();
        spans.push(SourceColorSpan {
            span,
            color,
            depth,
            order,
        });
    }
}

fn render_source_colors(input: &str, spans: &[SourceColorSpan]) -> String {
    let mut boundaries = vec![0, input.len()];
    for colored in spans {
        if input.get(colored.span.start..colored.span.end).is_some() {
            boundaries.push(colored.span.start);
            boundaries.push(colored.span.end);
        }
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut rendered = String::with_capacity(input.len() + spans.len() * 12);
    let mut active_color = None;
    for window in boundaries.windows(2) {
        let [start, end] = *window else {
            continue;
        };
        let Some(text) = input.get(start..end) else {
            continue;
        };
        let color = spans
            .iter()
            .filter(|colored| {
                colored.span.start <= start
                    && end <= colored.span.end
                    && input.get(colored.span.start..colored.span.end).is_some()
            })
            .max_by_key(|colored| (colored.depth, colored.order))
            .map(|colored| colored.color);
        if color != active_color {
            match color {
                Some(color) => {
                    rendered.push_str("\x1b[");
                    rendered.push_str(color.ansi_code());
                    rendered.push('m');
                }
                None => rendered.push_str("\x1b[0m"),
            }
            active_color = color;
        }
        rendered.push_str(text);
    }
    if active_color.is_some() {
        rendered.push_str("\x1b[0m");
    }
    rendered
}

fn effect_report(input: &str, candidate: EffectCandidate, catalog: &Catalog) -> EffectReport {
    let mut source_colors = Vec::new();
    collect_effect_colors(&candidate, &mut source_colors, 0);
    let EffectCandidate {
        matched,
        parsed_captures,
        handler,
        metadata,
        ..
    } = candidate;
    let expressions = parsed_captures
        .into_iter()
        .filter_map(|capture| match capture.result.value {
            Some(ParsedCaptureValue::Expression(expression)) => Some(expression),
            _ => None,
        })
        .collect::<Vec<_>>();
    let syntax = syntax_identity(&matched, catalog, effect_syntax_category(matched.kind));
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
        source_colors,
    }
}

fn candidate_summary(candidate: EffectCandidate, catalog: &Catalog) -> CandidateSummaryReport {
    CandidateSummaryReport {
        syntax: syntax_identity(
            &candidate.matched,
            catalog,
            effect_syntax_category(candidate.matched.kind),
        ),
        pattern_index: candidate.matched.pattern_index,
        pattern: candidate.matched.pattern,
    }
}

#[derive(Clone, Copy)]
enum SyntaxCategory {
    Effect,
    Expression,
    Section,
}

const fn effect_syntax_category(kind: MatchSyntaxKind) -> SyntaxCategory {
    match kind {
        MatchSyntaxKind::Section => SyntaxCategory::Section,
        _ => SyntaxCategory::Effect,
    }
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
            (SyntaxCategory::Section, Syntax::Section(value)) if value.effect_section => {
                Some(&value.common)
            }
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
                ..
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
                ..
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
        ExpressionNodeKind::List { conjunction } => (
            ExpressionIdentityReport::List {
                conjunction: match conjunction {
                    ExpressionListConjunction::And => ExpressionListConjunctionReport::And,
                    ExpressionListConjunction::Or => ExpressionListConjunctionReport::Or,
                },
            },
            None,
        ),
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
        ExpressionNodeKind::Arithmetic {
            operator,
            operation_registration_id,
        } => {
            let addon = catalog
                .operations()
                .values()
                .flatten()
                .find(|operation| operation.registration_id.as_str() == operation_registration_id)
                .map(|operation| AddonReport {
                    name: operation.addon.name.clone(),
                    version: operation.addon.version.clone(),
                });
            (
                ExpressionIdentityReport::Arithmetic {
                    operator: operator.clone(),
                    operation_registration_id: operation_registration_id.clone(),
                    addon,
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
    let operands = if !truncated && matches!(node.kind, ExpressionNodeKind::Arithmetic { .. }) {
        node.children
            .iter()
            .map(|child| expression_report(child, input, catalog, depth + 1))
            .collect()
    } else {
        Vec::new()
    };
    let items = if !truncated && matches!(node.kind, ExpressionNodeKind::List { .. }) {
        node.children
            .iter()
            .map(|child| expression_report(child, input, catalog, depth + 1))
            .collect()
    } else {
        Vec::new()
    };
    let embedded_expressions = if !truncated
        && matches!(
            node.kind,
            ExpressionNodeKind::Variable { .. }
                | ExpressionNodeKind::Literal { .. }
                | ExpressionNodeKind::Custom { .. }
        ) {
        node.children
            .iter()
            .map(|child| expression_report(child, input, catalog, depth + 1))
            .collect()
    } else {
        Vec::new()
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
        operands,
        items,
        embedded_expressions,
        public_data: expression_public_data(&node.public_data),
        metadata: node.metadata.clone(),
        truncated,
    }
}

fn expression_public_data(entries: &[ExpressionPublicData]) -> Vec<ExpressionPublicDataReport> {
    entries
        .iter()
        .map(
            |entry| match serde_json::value::RawValue::from_string(entry.json.clone()) {
                Ok(json) => ExpressionPublicDataReport {
                    schema_id: entry.schema_id.clone(),
                    schema_version: entry.schema_version,
                    json: Some(json),
                    raw_json: None,
                    error: None,
                },
                Err(error) => ExpressionPublicDataReport {
                    schema_id: entry.schema_id.clone(),
                    schema_version: entry.schema_version,
                    json: None,
                    raw_json: Some(entry.json.clone()),
                    error: Some(format!("invalid public data JSON: {error}")),
                },
            },
        )
        .collect()
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
    let span = match_span(&failure.span);
    FailureReport {
        offset: span.start,
        span,
        reasons: failure
            .reasons
            .into_iter()
            .map(|reason| match reason {
                PatternFailureReason::Literal { expected } => {
                    FailureReasonReport::Literal { expected }
                }
                PatternFailureReason::Regex { pattern } => FailureReasonReport::Regex { pattern },
                PatternFailureReason::Expression => FailureReasonReport::Expression,
                PatternFailureReason::TypeExpression { expected } => {
                    FailureReasonReport::TypeExpression { expected }
                }
                PatternFailureReason::EventRestricted { supported, current } => {
                    FailureReasonReport::EventRestricted { supported, current }
                }
                PatternFailureReason::TrailingInput => FailureReasonReport::TrailingInput,
                PatternFailureReason::HookRejected { reason } => {
                    FailureReasonReport::HookRejected { reason }
                }
            })
            .collect(),
        contexts: Vec::new(),
        related: Vec::new(),
        interpretations: Vec::new(),
    }
}

fn candidate_failure_report(
    candidate: &EffectCandidateFailure,
    candidates: &[EffectCandidateFailure],
    catalog: &Catalog,
) -> FailureReport {
    let trace = &candidate.matched.trace;
    let mut report = failure_trace_report(trace.clone());
    report.related = candidate
        .matched
        .related
        .iter()
        .cloned()
        .map(failure_trace_report)
        .collect();
    report.interpretations = candidates
        .iter()
        .filter(|alternative| {
            alternative.matched.registration_id != candidate.matched.registration_id
                || alternative.matched.pattern_index != candidate.matched.pattern_index
        })
        .filter(|alternative| alternative.matched.pattern.is_some())
        .take(3)
        .map(|alternative| FailureInterpretationReport {
            syntax: syntax_identity_from_ids(
                &alternative.matched.definition_id,
                &alternative.matched.registration_id,
                catalog,
                effect_syntax_category(alternative.matched.kind),
            ),
            pattern_index: alternative.matched.pattern_index,
            pattern: alternative.matched.pattern.clone(),
            span: match_span(&alternative.matched.trace.root_cause().failure.span),
        })
        .collect();
    report
}

fn failure_trace_report(trace: FailureTrace) -> FailureReport {
    let mut report = failure_report(trace.root_cause().failure.clone());
    report.contexts = failure_contexts(&trace);
    report
}

fn failure_contexts(trace: &FailureTrace) -> Vec<FailureContextReport> {
    let mut contexts = Vec::new();
    let mut current = Some(trace);
    while let Some(trace) = current {
        if let Some(frame) = &trace.frame {
            contexts.push(FailureContextReport {
                syntax_kind: match_syntax_kind(frame.kind).to_owned(),
                definition_id: frame.definition_id.clone(),
                registration_id: frame.registration_id.clone(),
                pattern_index: frame.pattern_index,
                pattern: frame.pattern.clone(),
                role: match frame.role {
                    FailureFrameRole::SemanticCandidate => {
                        FailureContextRoleReport::SemanticCandidate
                    }
                    FailureFrameRole::TypeExpressionCapture => {
                        FailureContextRoleReport::PatternElement
                    }
                    FailureFrameRole::ExpressionCapture { index } => {
                        FailureContextRoleReport::ExpressionCapture { index }
                    }
                    FailureFrameRole::ConditionCapture { index } => {
                        FailureContextRoleReport::ConditionCapture { index }
                    }
                    FailureFrameRole::EffectCapture { index } => {
                        FailureContextRoleReport::EffectCapture { index }
                    }
                },
                span: match_span(&frame.input_span),
                pattern_span: frame.pattern_span.map(pattern_span),
            });
        }
        current = trace.cause.as_deref();
    }
    contexts
}

fn match_syntax_kind(kind: MatchSyntaxKind) -> &'static str {
    match kind {
        MatchSyntaxKind::Event => "Event",
        MatchSyntaxKind::Condition => "Condition",
        MatchSyntaxKind::Effect => "Effect",
        MatchSyntaxKind::Expression => "Expression",
        MatchSyntaxKind::Type => "Type",
        MatchSyntaxKind::Function => "Function",
        MatchSyntaxKind::Section => "Section",
        MatchSyntaxKind::Structure => "Structure",
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

fn write_failure(
    writer: &mut dyn Write,
    source: &str,
    failure: &FailureReport,
    color: bool,
    message: &str,
) -> io::Result<()> {
    let primary_index = failure
        .reasons
        .iter()
        .position(|reason| matches!(reason, FailureReasonReport::TypeExpression { .. }))
        .or((!failure.reasons.is_empty()).then_some(0));
    let primary = primary_index
        .and_then(|index| failure.reasons.get(index))
        .map(FailureReasonReport::human)
        .unwrap_or_else(|| "Effect parsing stopped here".to_owned());
    let start = failure.span.start.min(source.len());
    let end = failure.span.end.min(source.len()).max(start);
    let mut labels = vec![LabeledSpan::new(Some(primary), start, end - start)];
    let mut labeled_spans = vec![(start, end)];
    for related in &failure.related {
        let related_start = related.span.start.min(source.len());
        let related_end = related.span.end.min(source.len()).max(related_start);
        if labeled_spans.contains(&(related_start, related_end)) {
            continue;
        }
        let label = related
            .reasons
            .iter()
            .find(|reason| matches!(reason, FailureReasonReport::TypeExpression { .. }))
            .or_else(|| related.reasons.first())
            .map(FailureReasonReport::human)
            .unwrap_or_else(|| "related parse failure".to_owned());
        labels.push(LabeledSpan::new(
            Some(label),
            related_start,
            related_end - related_start,
        ));
        labeled_spans.push((related_start, related_end));
    }
    for context in failure.contexts.iter().rev() {
        let context_start = context.span.start.min(source.len());
        let context_end = context.span.end.min(source.len()).max(context_start);
        if labeled_spans.contains(&(context_start, context_end)) {
            continue;
        }
        labels.push(LabeledSpan::new(
            Some(context.role.human(&context.syntax_kind)),
            context_start,
            context_end - context_start,
        ));
        labeled_spans.push((context_start, context_end));
        if labels.len() >= 8 {
            break;
        }
    }
    let mut diagnostic = MietteDiagnostic::new(message)
        .with_code("effectcommandcli::parse")
        .with_labels(labels);
    let has_type_failure = matches!(
        primary_index.and_then(|index| failure.reasons.get(index)),
        Some(FailureReasonReport::TypeExpression { .. })
    );
    let mut help = failure
        .reasons
        .iter()
        .enumerate()
        .filter(|(index, _)| Some(*index) != primary_index)
        .map(|(_, reason)| reason)
        .filter(|reason| {
            !has_type_failure
                || !matches!(
                    reason,
                    FailureReasonReport::Literal { .. } | FailureReasonReport::Regex { .. }
                )
        })
        .map(FailureReasonReport::human)
        .collect::<Vec<_>>();
    let mut patterns = Vec::new();
    for context in &failure.contexts {
        let value = format!("{} pattern: {}", context.syntax_kind, context.pattern);
        if !patterns.contains(&value) {
            patterns.push(value);
        }
    }
    help.extend(patterns);
    for interpretation in &failure.interpretations {
        if let Some(pattern) = &interpretation.pattern {
            help.push(format!(
                "also considered {} pattern: {pattern}",
                interpretation.syntax.display_name()
            ));
        }
    }
    let token = source.get(start..end).unwrap_or_default().trim();
    if !token.is_empty()
        && token
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        help.push(format!("if {token:?} is a variable, write {{{token}}}"));
    }
    if !help.is_empty() {
        diagnostic = diagnostic.with_help(help.join("\n"));
    }
    let report = miette::Report::new(diagnostic)
        .with_source_code(NamedSource::new("effect.sk", source.to_owned()));
    let theme = if color {
        GraphicalTheme::unicode()
    } else {
        GraphicalTheme::unicode_nocolor()
    };
    let mut rendered = String::new();
    GraphicalReportHandler::new_themed(theme)
        .with_urls(false)
        .render_report(&mut rendered, report.as_ref())
        .map_err(io::Error::other)?;
    write!(writer, "{rendered}")
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
        ExpressionIdentityReport::List { conjunction } => {
            writeln!(
                writer,
                "{prefix}resolved: expressionList ({})",
                match conjunction {
                    ExpressionListConjunctionReport::And => "and",
                    ExpressionListConjunctionReport::Or => "or",
                }
            )?;
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
        ExpressionIdentityReport::Arithmetic {
            operator,
            operation_registration_id,
            addon,
        } => {
            writeln!(writer, "{prefix}resolved: arithmetic ({operator})")?;
            writeln!(
                writer,
                "{prefix}operationRegistrationId: {operation_registration_id}"
            )?;
            if let Some(addon) = addon {
                writeln!(writer, "{prefix}addon: {} {}", addon.name, addon.version)?;
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
    if !expression.public_data.is_empty() {
        writeln!(writer, "{prefix}publicData:")?;
        for data in &expression.public_data {
            writeln!(writer, "{prefix}  - schemaId: {}", data.schema_id)?;
            writeln!(writer, "{prefix}    schemaVersion: {}", data.schema_version)?;
            if let Some(json) = &data.json {
                writeln!(writer, "{prefix}    json: {}", json.get())?;
            }
            if let Some(raw_json) = &data.raw_json {
                writeln!(writer, "{prefix}    rawJson: {raw_json:?}")?;
            }
            if let Some(error) = &data.error {
                writeln!(writer, "{prefix}    error: {error}")?;
            }
        }
    }
    if !expression.metadata.is_empty() {
        writeln!(writer, "{prefix}metadata:")?;
        for (key, value) in &expression.metadata {
            writeln!(writer, "{prefix}  {key}: {value}")?;
        }
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
    } else if !expression.operands.is_empty() {
        writeln!(writer, "{prefix}operands:")?;
        for (index, operand) in expression.operands.iter().enumerate() {
            let side = ["left", "right"].get(index).copied().unwrap_or("operand");
            writeln!(writer, "{prefix}  {side}:")?;
            write_expression(writer, operand, indent + 2)?;
        }
    } else if !expression.items.is_empty() {
        writeln!(writer, "{prefix}items:")?;
        for (index, item) in expression.items.iter().enumerate() {
            writeln!(writer, "{prefix}  item[{index}]:")?;
            write_expression(writer, item, indent + 2)?;
        }
    } else if !expression.embedded_expressions.is_empty() {
        writeln!(writer, "{prefix}embeddedExpressions:")?;
        for (index, embedded) in expression.embedded_expressions.iter().enumerate() {
            writeln!(writer, "{prefix}  expression[{index}]:")?;
            write_expression(writer, embedded, indent + 2)?;
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct IncompleteEffectReport {
    syntax: SyntaxIdentityReport,
    pattern_index: Option<usize>,
    pattern: Option<String>,
    priority: i32,
    registration_order: usize,
    resolved_order: Option<usize>,
    handler: Option<String>,
    metadata: BTreeMap<String, String>,
}

fn format_parse_duration(nanoseconds: u64) -> String {
    if nanoseconds >= 1_000_000 {
        format!("{:.3} ms", nanoseconds as f64 / 1_000_000.0)
    } else {
        format!("{nanoseconds} ns")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_data_json_keeps_large_numbers_raw() {
        let digits = "18446744073709551617.123456789012345678901234567890";
        let entries = [ExpressionPublicData {
            schema_id: "test.large-number".to_owned(),
            schema_version: 1,
            json: format!(r#"{{"value":{digits}}}"#),
        }];

        let output = serde_json::to_string(&expression_public_data(&entries)).unwrap();

        assert!(output.contains(digits));
    }

    #[test]
    fn invalid_public_data_json_is_explicit_in_report() {
        let entries = [ExpressionPublicData {
            schema_id: "test.invalid".to_owned(),
            schema_version: 1,
            json: "{invalid".to_owned(),
        }];

        let output = serde_json::to_string(&expression_public_data(&entries)).unwrap();

        assert!(output.contains(r#""rawJson":"{invalid"#));
        assert!(output.contains("invalid public data JSON"));
        assert!(!output.contains(r#""json":"{invalid"#));
    }

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

    #[test]
    fn event_restriction_distinguishes_missing_and_incompatible_contexts() {
        let supported = vec!["org.bukkit.event.player.PlayerJoinEvent".to_owned()];
        assert_eq!(
            FailureReasonReport::EventRestricted {
                supported: supported.clone(),
                current: Vec::new(),
            }
            .human(),
            "requires event context: org.bukkit.event.player.PlayerJoinEvent"
        );
        assert_eq!(
            FailureReasonReport::EventRestricted {
                supported,
                current: vec!["org.bukkit.event.player.PlayerQuitEvent".to_owned()],
            }
            .human(),
            "only available in org.bukkit.event.player.PlayerJoinEvent; current event is org.bukkit.event.player.PlayerQuitEvent"
        );
    }

    #[test]
    fn failure_renderer_supports_unicode_spans_and_color() {
        let source = "teleport あ";
        let start = source.find('あ').unwrap();
        let failure = FailureReport {
            offset: start,
            span: SpanReport {
                start,
                end: start + 'あ'.len_utf8(),
            },
            reasons: vec![FailureReasonReport::TypeExpression {
                expected: vec!["entity".to_owned()],
            }],
            contexts: Vec::new(),
            related: Vec::new(),
            interpretations: Vec::new(),
        };
        let mut output = Vec::new();

        write_failure(&mut output, source, &failure, true, "No Effect matched").unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("No Effect matched"));
        assert!(output.contains("teleport あ"));
        assert!(output.contains("expected expression of type entity"));
        assert!(output.contains('\x1b'));
    }

    #[test]
    fn failure_renderer_keeps_two_related_typed_capture_labels() {
        let source = "teleport a to location(b, 2, 3)";
        let failure = FailureReport {
            offset: 9,
            span: SpanReport { start: 9, end: 10 },
            reasons: vec![FailureReasonReport::TypeExpression {
                expected: vec!["entities".to_owned()],
            }],
            contexts: Vec::new(),
            related: vec![FailureReport {
                offset: 14,
                span: SpanReport { start: 14, end: 31 },
                reasons: vec![FailureReasonReport::TypeExpression {
                    expected: vec!["number".to_owned()],
                }],
                contexts: Vec::new(),
                related: Vec::new(),
                interpretations: Vec::new(),
            }],
            interpretations: Vec::new(),
        };
        let mut output = Vec::new();

        write_failure(&mut output, source, &failure, false, "No Effect matched").unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("expected expression of type entities"));
        assert!(output.contains("expected expression of type number"));
        assert!(output.contains("teleport a to location(b, 2, 3)"));
    }

    #[test]
    fn parse_duration_uses_the_smallest_useful_display_unit() {
        assert_eq!(format_parse_duration(999_999), "999999 ns");
        assert_eq!(format_parse_duration(1_000_000), "1.000 ms");
    }

    #[test]
    fn source_colors_use_deepest_nested_span() {
        let spans = [
            SourceColorSpan {
                span: SpanReport { start: 0, end: 6 },
                color: SourceColor::Effect,
                depth: 0,
                order: 0,
            },
            SourceColorSpan {
                span: SpanReport { start: 0, end: 6 },
                color: SourceColor::Function,
                depth: 1,
                order: 1,
            },
            SourceColorSpan {
                span: SpanReport { start: 4, end: 5 },
                color: SourceColor::Literal,
                depth: 2,
                order: 2,
            },
        ];

        assert_eq!(
            render_source_colors("sin(1)", &spans),
            "\x1b[38;2;160;160;160msin(\x1b[38;2;255;255;255m1\x1b[38;2;160;160;160m)\x1b[0m"
        );
    }

    #[test]
    fn source_colors_preserve_plain_output_when_disabled() {
        let effect = EffectReport {
            syntax: SyntaxIdentityReport {
                syntax_id: None,
                definition_id: String::new(),
                registration_id: String::new(),
                element_class: None,
                addon: None,
            },
            pattern: PatternReport {
                index: 0,
                source: String::new(),
                elements: Vec::new(),
            },
            span: SpanReport { start: 0, end: 6 },
            elements: Vec::new(),
            tags: Vec::new(),
            marks: Vec::new(),
            handler: None,
            metadata: BTreeMap::new(),
            source_colors: Vec::new(),
        };

        assert_eq!(render_matched_source("sin(1)", &effect, false), "sin(1)");
    }

    #[test]
    fn alias_color_uses_the_literal_range_inside_an_item_type() {
        let metadata = BTreeMap::from([
            (
                "nlaocs.core-library/literal-source".to_owned(),
                "alias".to_owned(),
            ),
            (
                "nlaocs.core-library/literal-range-start".to_owned(),
                "2".to_owned(),
            ),
            (
                "nlaocs.core-library/literal-range-end".to_owned(),
                "9".to_owned(),
            ),
        ]);

        let alias = literal_alias_span(&metadata, SpanReport { start: 0, end: 9 }).unwrap();
        assert_eq!(alias.start, 2);
        assert_eq!(alias.end, 9);
    }

    #[test]
    fn source_colors_distinguish_variables_and_literal_sources() {
        let no_metadata = BTreeMap::new();
        assert_eq!(
            literal_source_color("core.literal.string", &no_metadata),
            SourceColor::Literal
        );
        assert_eq!(
            literal_source_color("core.literal.number", &no_metadata),
            SourceColor::Literal
        );
        assert_eq!(
            literal_source_color("core.literal.type", &no_metadata),
            SourceColor::Literal
        );
        assert_eq!(
            literal_source_color("core.literal.entity-data", &no_metadata),
            SourceColor::Literal
        );
        assert_eq!(
            literal_source_color("core.literal.boolean", &no_metadata),
            SourceColor::Literal
        );
        assert_eq!(
            literal_source_color("core.literal.item-type", &no_metadata),
            SourceColor::Literal
        );
        assert_eq!(
            literal_source_color("core.literal.class-info", &no_metadata),
            SourceColor::TypeName
        );
        assert_eq!(
            literal_source_color(
                "core.literal.type",
                &BTreeMap::from([("type-code-name".to_owned(), "classinfo".to_owned())]),
            ),
            SourceColor::TypeName
        );
        assert_eq!(SourceColor::Variable.ansi_code(), "38;2;0;255;255");
        assert_eq!(SourceColor::Literal.ansi_code(), "38;2;255;255;255");
    }
}

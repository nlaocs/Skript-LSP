use super::*;
use parser_wasm::DocumentParseResult;
use parser_wasm::bindings::nlaocs::skript_parser_addon::types::{
    AstContextOrigin, DynamicMultiplicity, ExpressionPossibleReturnTypesState, HookDecision,
    MetadataEntry as WitMetadataEntry, TextRange as WitTextRange,
};
use parser_wasm::host::{
    AstMacroCall, AstNode, CaptureValue, HookCall, MappedSpan as WitMappedSpan,
    SyntaxKind as WitSyntaxKind, TextMacroCall, TreeMacroCall,
};
use parser_wasm::state::{
    NamespaceVisibility, StateNamespaceKey, StateReadWriteSet, StateRecordKey, StateScope,
};
use serde_json::Value;
use skript_parser::{
    ExpansionKind, MappedSpan, RawDiagnosticSeverity, RawTree, SectionBodyNode,
    SectionDiagnosticKind, SectionMatches, StructureBody, StructureDiagnosticKind,
    StructureDocumentNode, StructureMatches,
};
use std::collections::{HashMap, HashSet};

const DOCUMENT_REPORT_SCHEMA_VERSION: u32 = 1;
const MAX_HUMAN_TREE_DEPTH: usize = 128;

/// Stable report for one submitted multiline Skript document.
///
/// The tree is stored as an arena so deeply nested Expressions do not create a
/// recursively serialized Rust value. `roots` and every capture/child edge use
/// IDs from `nodes`.
#[derive(Debug, Clone)]
pub struct DocumentAnalysisReport {
    data: DocumentReportData,
    source_colors: Vec<SourceColorSpan>,
    virtual_source_colors: Vec<SourceColorSpan>,
}

impl DocumentAnalysisReport {
    pub(crate) fn from_result(
        input: &str,
        snapshot: &SnapshotDescription,
        result: DocumentParseResult,
        catalog: &Catalog,
        parse_duration: Duration,
    ) -> Self {
        debug_assert_eq!(input, result.source.original());
        let virtual_source = result.source.virtual_source().to_owned();
        let macro_pipeline = macro_pipeline(&result);
        let nodes = result
            .ast
            .nodes
            .iter()
            .map(|node| document_node(node, catalog))
            .collect::<Vec<_>>();
        let source_colors =
            document_source_colors(&result.ast.roots, &nodes, input, SourceSpace::Original);
        let virtual_source_colors = document_source_colors(
            &result.ast.roots,
            &nodes,
            &virtual_source,
            SourceSpace::Virtual,
        );
        let recoveries = recoveries(&result.syntax, &result.raw_tree, catalog);
        let mut diagnostics = raw_diagnostics(&result.raw_tree);
        collect_structure_diagnostics(&result.syntax.roots, &mut diagnostics);
        diagnostics.extend(result.effects.diagnostics.iter().map(hook_diagnostic));
        let component_failures = result
            .component_failures
            .iter()
            .map(|failure| ComponentFailureReport {
                component_id: failure.component_id.clone(),
                subscription_id: failure.subscription_id.clone(),
                message: failure.error.to_string(),
            })
            .collect::<Vec<_>>();
        let status = if recoveries.is_empty()
            && component_failures.is_empty()
            && diagnostics
                .iter()
                .all(|diagnostic| diagnostic.severity != "error")
        {
            "matched"
        } else {
            "incomplete"
        };
        Self {
            source_colors,
            virtual_source_colors,
            data: DocumentReportData {
                schema_version: DOCUMENT_REPORT_SCHEMA_VERSION,
                input: input.to_owned(),
                virtual_source,
                snapshot: SnapshotReport {
                    id: snapshot.snapshot_id.clone(),
                    minecraft_version: snapshot.minecraft_version.clone(),
                    skript_version: snapshot.skript_version.clone(),
                    plugin_count: snapshot.plugin_count,
                },
                parse_duration_ns: u64::try_from(parse_duration.as_nanos()).unwrap_or(u64::MAX),
                status: status.to_owned(),
                roots: result.ast.roots,
                nodes,
                recoveries,
                diagnostics,
                component_failures,
                functions: result.functions.len(),
                state: DocumentStateReport {
                    writes: result.state.writes,
                    reads: result.state.read_write_set.reads.len(),
                    written_records: result.state.read_write_set.writes.len(),
                    namespace_revisions: result.state.read_write_set.namespace_revisions.len(),
                },
                macro_pipeline,
            },
        }
    }

    /// Returns true when every source node was selected and no error diagnostic remains.
    pub fn matched(&self) -> bool {
        self.data.status == "matched"
    }

    /// Serializes the versioned document report as pretty JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&self.data)
    }

    /// Writes either the human tree or the JSON representation.
    pub fn write(&self, format: OutputFormat, writer: impl Write) -> io::Result<()> {
        self.write_with_color(format, writer, false)
    }

    pub(crate) fn write_with_color(
        &self,
        format: OutputFormat,
        mut writer: impl Write,
        color: bool,
    ) -> io::Result<()> {
        match format {
            OutputFormat::Json => {
                serde_json::to_writer_pretty(&mut writer, &self.data).map_err(io::Error::other)?;
                writeln!(writer)
            }
            OutputFormat::Human => self.write_human(&mut writer, color),
        }
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
        writeln!(writer, "document: {}", self.data.status)?;
        writeln!(writer, "source:")?;
        write_source_lines(writer, &self.data.input, &self.source_colors, color)?;
        if self.data.virtual_source != self.data.input {
            writeln!(writer, "expandedSource:")?;
            write_source_lines(
                writer,
                &self.data.virtual_source,
                &self.virtual_source_colors,
                color,
            )?;
        }
        writeln!(writer, "tree:")?;
        if self.data.roots.is_empty() {
            writeln!(writer, "  []")?;
        } else {
            let mut visited = HashSet::new();
            for root in &self.data.roots {
                self.write_node(writer, *root, 1, &mut visited)?;
            }
        }
        if !self.data.recoveries.is_empty() {
            writeln!(writer, "recoveries:")?;
            for recovery in &self.data.recoveries {
                writeln!(
                    writer,
                    "  - {} at {}..{}: {:?}",
                    recovery.kind,
                    display_range(&recovery.span).start,
                    display_range(&recovery.span).end,
                    recovery.source,
                )?;
                if let Some(failure) = &recovery.failure {
                    write_failure_named(
                        writer,
                        &self.data.input,
                        "repl.sk",
                        failure,
                        color,
                        &format!("{} candidate is incomplete", recovery.kind),
                    )?;
                }
            }
        }
        for diagnostic in &self.data.diagnostics {
            let diagnostic_range = display_range(&diagnostic.span);
            let has_detailed_section_failure = diagnostic.code == "parser.section.unclaimed"
                && self.data.recoveries.iter().any(|recovery| {
                    let recovery_range = display_range(&recovery.span);
                    recovery.kind == "Section"
                        && recovery.failure.is_some()
                        && recovery_range.start == diagnostic_range.start
                        && (recovery_range.end == diagnostic_range.end
                            || recovery_range.end.saturating_add(1) == diagnostic_range.end)
                });
            if has_detailed_section_failure {
                continue;
            }
            write_document_diagnostic(writer, &self.data.input, diagnostic, color)?;
        }
        if !self.data.component_failures.is_empty() {
            writeln!(writer, "componentFailures:")?;
            for failure in &self.data.component_failures {
                writeln!(
                    writer,
                    "  - {}/{}: {}",
                    failure.component_id, failure.subscription_id, failure.message
                )?;
            }
        }
        if self.data.macro_pipeline.has_activity() {
            writeln!(
                writer,
                "macros: text={} ({} calls), tree={} ({} calls), ast={} ({} calls), {} expansions",
                self.data.macro_pipeline.text.decision.kind,
                self.data.macro_pipeline.text.calls.len(),
                self.data.macro_pipeline.tree.decision.kind,
                self.data.macro_pipeline.tree.calls.len(),
                self.data.macro_pipeline.ast.decision.kind,
                self.data.macro_pipeline.ast.calls.len(),
                self.data.macro_pipeline.expansions.len(),
            )?;
        }
        writeln!(
            writer,
            "state: {} writes ({} records read, {} records written)",
            self.data.state.writes, self.data.state.reads, self.data.state.written_records
        )
    }

    fn write_node(
        &self,
        writer: &mut dyn Write,
        id: u64,
        depth: usize,
        visited: &mut HashSet<u64>,
    ) -> io::Result<()> {
        let indent = "  ".repeat(depth);
        if depth > MAX_HUMAN_TREE_DEPTH {
            return writeln!(writer, "{indent}- ... (tree depth limit reached)");
        }
        if !visited.insert(id) {
            return writeln!(writer, "{indent}- node#{id} (already shown)");
        }
        let Some(node) = self.data.nodes.iter().find(|node| node.id == id) else {
            return writeln!(writer, "{indent}- missing node#{id}");
        };
        let defaulted = node
            .semantics
            .as_ref()
            .and_then(|semantics| semantics.default_expression.as_ref())
            .is_some();
        let name = node
            .identity
            .as_ref()
            .map_or(node.syntax_id.as_str(), |identity| identity.display_name());
        writeln!(
            writer,
            "{indent}- {} {}{} at {}..{}: {:?}",
            node.kind,
            name,
            if defaulted { " (default)" } else { "" },
            node.span.virtual_range.start,
            node.span.virtual_range.end,
            node.text,
        )?;
        if let Some(pattern) = &node.pattern {
            writeln!(
                writer,
                "{indent}  pattern[{}]: {}",
                pattern.index, pattern.source
            )?;
        }
        if let Some(semantics) = &node.semantics {
            if let Some(return_type) = &semantics.return_type {
                writeln!(writer, "{indent}  returnType: {return_type}")?;
            }
            if let Some(multiplicity) = &semantics.multiplicity {
                writeln!(writer, "{indent}  multiplicity: {multiplicity}")?;
            }
            if let Some(default) = &semantics.default_expression {
                writeln!(
                    writer,
                    "{indent}  defaultProvider: {}/{} ({})",
                    default.component_id, default.provider_id, default.reason
                )?;
            }
        }
        for child in &node.children {
            self.write_node(writer, *child, depth + 1, visited)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentReportData {
    schema_version: u32,
    input: String,
    virtual_source: String,
    snapshot: SnapshotReport,
    parse_duration_ns: u64,
    status: String,
    roots: Vec<u64>,
    nodes: Vec<DocumentNodeReport>,
    recoveries: Vec<RecoveryReport>,
    diagnostics: Vec<DocumentDiagnosticReport>,
    component_failures: Vec<ComponentFailureReport>,
    functions: usize,
    state: DocumentStateReport,
    macro_pipeline: DocumentMacroPipelineReport,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentNodeReport {
    id: u64,
    kind: String,
    syntax_id: String,
    text: String,
    span: MappedSpanReport,
    syntax_context: u64,
    context_origin: String,
    identity: Option<SyntaxIdentityReport>,
    pattern: Option<PatternReport>,
    semantics: Option<DocumentSemanticsReport>,
    captures: Vec<DocumentCaptureReport>,
    children: Vec<u64>,
    metadata: Vec<DocumentMetadataReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentSemanticsReport {
    kind: String,
    return_type: Option<String>,
    possible_return_types: Vec<String>,
    possible_return_types_state: String,
    multiplicity: Option<String>,
    default_expression: Option<DefaultExpressionReport>,
    public_data: Vec<DocumentPublicDataReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DefaultExpressionReport {
    capture_index: u64,
    pattern_span: DocumentTextRange,
    expression: String,
    requested_type: DocumentExpectedType,
    type_definition_id: String,
    type_registration_id: String,
    provider_id: String,
    component_id: String,
    reason: String,
    event_classes: Vec<String>,
    section_scope_ids: Vec<u64>,
    catalog_references: Vec<DefaultCatalogReferenceReport>,
    is_literal: bool,
    time: i32,
    span: MappedSpanReport,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DefaultCatalogReferenceReport {
    role: String,
    definition_id: Option<String>,
    registration_id: Option<String>,
    source_digest: Option<String>,
    snapshot_id: Option<String>,
    document: Option<String>,
    index: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentExpectedType {
    class_name: String,
    plural: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentPublicDataReport {
    schema_id: String,
    schema_version: u32,
    value: Value,
}

#[derive(Debug, Clone, Serialize)]
struct DocumentCaptureReport {
    name: String,
    #[serde(flatten)]
    value: DocumentCaptureValueReport,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
enum DocumentCaptureValueReport {
    Text(String),
    Node(u64),
    Nodes(Vec<u64>),
    Span(MappedSpanReport),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentMetadataReport {
    key: String,
    value: String,
    owner_component_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct RecoveryReport {
    kind: String,
    source: String,
    span: MappedSpanReport,
    failure: Option<FailureReport>,
}

#[derive(Debug, Clone, Serialize)]
struct DocumentDiagnosticReport {
    code: String,
    message: String,
    severity: String,
    span: MappedSpanReport,
    related: Vec<DocumentRelatedDiagnosticReport>,
}

#[derive(Debug, Clone, Serialize)]
struct DocumentRelatedDiagnosticReport {
    message: String,
    span: MappedSpanReport,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MappedSpanReport {
    virtual_range: DocumentTextRange,
    origins: Vec<DocumentOriginReport>,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct DocumentTextRange {
    start: u64,
    end: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentOriginReport {
    original_range: DocumentTextRange,
    kind: String,
    expansion_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentStateReport {
    writes: usize,
    reads: usize,
    written_records: usize,
    namespace_revisions: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentMacroPipelineReport {
    text: DocumentMacroPhaseReport,
    tree: DocumentMacroPhaseReport,
    ast: DocumentMacroPhaseReport,
    expansions: Vec<DocumentExpansionReport>,
    syntax_calls: Vec<DocumentHookCallReport>,
}

impl DocumentMacroPipelineReport {
    fn has_activity(&self) -> bool {
        !self.text.calls.is_empty()
            || !self.tree.calls.is_empty()
            || !self.ast.calls.is_empty()
            || !self.expansions.is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
struct DocumentMacroPhaseReport {
    decision: DocumentHookDecisionReport,
    calls: Vec<DocumentMacroCallReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentHookDecisionReport {
    kind: String,
    rejection_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentMacroCallReport {
    component_id: String,
    subscription_id: String,
    target: Option<u64>,
    accepted: bool,
    expansion_id: Option<u64>,
    state_accesses: DocumentStateAccessReport,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentHookCallReport {
    component_id: String,
    subscription_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentExpansionReport {
    id: u64,
    kind: String,
    component_id: String,
    hook_id: String,
    call_sites: Vec<DocumentExpansionSiteReport>,
    definition_site: Option<DocumentExpansionSiteReport>,
    syntax_context: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentExpansionSiteReport {
    original_range: DocumentTextRange,
    expansion_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentStateAccessReport {
    reads: Vec<DocumentStateRecordReport>,
    writes: Vec<DocumentStateRecordReport>,
    namespace_revisions: Vec<DocumentNamespaceRevisionReport>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentStateRecordReport {
    scope: String,
    visibility: String,
    owner: Option<String>,
    namespace: String,
    key: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentNamespaceRevisionReport {
    scope: String,
    visibility: String,
    owner: Option<String>,
    namespace: String,
    revision: u64,
}

fn macro_pipeline(result: &DocumentParseResult) -> DocumentMacroPipelineReport {
    DocumentMacroPipelineReport {
        text: DocumentMacroPhaseReport {
            decision: hook_decision(&result.text_macro_decision),
            calls: result
                .text_macro_calls
                .iter()
                .map(text_macro_call)
                .collect(),
        },
        tree: DocumentMacroPhaseReport {
            decision: hook_decision(&result.tree_macro_decision),
            calls: result
                .tree_macro_calls
                .iter()
                .map(tree_macro_call)
                .collect(),
        },
        ast: DocumentMacroPhaseReport {
            decision: hook_decision(&result.ast_macro_decision),
            calls: result.ast_macro_calls.iter().map(ast_macro_call).collect(),
        },
        expansions: result
            .source
            .expansions()
            .iter()
            .map(|expansion| DocumentExpansionReport {
                id: u64::from(expansion.id.get()),
                kind: expansion_kind(expansion.kind).to_owned(),
                component_id: expansion.component.as_str().to_owned(),
                hook_id: expansion.hook.as_str().to_owned(),
                call_sites: expansion
                    .call_sites
                    .iter()
                    .map(|site| DocumentExpansionSiteReport {
                        original_range: native_range(site.original_range),
                        expansion_id: site.expansion.map(|id| u64::from(id.get())),
                    })
                    .collect(),
                definition_site: expansion.definition_site.map(|site| {
                    DocumentExpansionSiteReport {
                        original_range: native_range(site.original_range),
                        expansion_id: site.expansion.map(|id| u64::from(id.get())),
                    }
                }),
                syntax_context: u64::from(expansion.syntax_context.get()),
            })
            .collect(),
        syntax_calls: result.syntax_calls.iter().map(hook_call).collect(),
    }
}

fn hook_decision(decision: &HookDecision) -> DocumentHookDecisionReport {
    let (kind, rejection_reason) = match decision {
        HookDecision::ContinueProcessing => ("continueProcessing", None),
        HookDecision::NotApplicable => ("notApplicable", None),
        HookDecision::Handled => ("handled", None),
        HookDecision::Reject(rejection) => ("reject", Some(rejection.reason.clone())),
    };
    DocumentHookDecisionReport {
        kind: kind.to_owned(),
        rejection_reason,
    }
}

fn text_macro_call(call: &TextMacroCall) -> DocumentMacroCallReport {
    macro_call(
        &call.component_id,
        &call.subscription_id,
        None,
        call.accepted,
        call.expansion.map(|id| u64::from(id.get())),
        &call.state_accesses,
    )
}

fn tree_macro_call(call: &TreeMacroCall) -> DocumentMacroCallReport {
    macro_call(
        &call.component_id,
        &call.subscription_id,
        Some(call.target.get()),
        call.accepted,
        call.expansion.map(|id| u64::from(id.get())),
        &call.state_accesses,
    )
}

fn ast_macro_call(call: &AstMacroCall) -> DocumentMacroCallReport {
    macro_call(
        &call.component_id,
        &call.subscription_id,
        Some(call.target),
        call.accepted,
        call.expansion.map(|id| u64::from(id.get())),
        &call.state_accesses,
    )
}

fn macro_call(
    component_id: &str,
    subscription_id: &str,
    target: Option<u64>,
    accepted: bool,
    expansion_id: Option<u64>,
    state_accesses: &StateReadWriteSet,
) -> DocumentMacroCallReport {
    DocumentMacroCallReport {
        component_id: component_id.to_owned(),
        subscription_id: subscription_id.to_owned(),
        target,
        accepted,
        expansion_id,
        state_accesses: state_access_report(state_accesses),
    }
}

fn hook_call(call: &HookCall) -> DocumentHookCallReport {
    DocumentHookCallReport {
        component_id: call.component_id.clone(),
        subscription_id: call.subscription_id.clone(),
    }
}

fn state_access_report(accesses: &StateReadWriteSet) -> DocumentStateAccessReport {
    DocumentStateAccessReport {
        reads: accesses.reads.iter().map(state_record).collect(),
        writes: accesses.writes.iter().map(state_record).collect(),
        namespace_revisions: accesses
            .namespace_revisions
            .iter()
            .map(|(namespace, revision)| namespace_revision(namespace, *revision))
            .collect(),
    }
}

fn state_record(record: &StateRecordKey) -> DocumentStateRecordReport {
    DocumentStateRecordReport {
        scope: state_scope(record.scope).to_owned(),
        visibility: namespace_visibility(record.visibility).to_owned(),
        owner: record.owner.clone(),
        namespace: record.namespace.clone(),
        key: record.key.clone(),
    }
}

fn namespace_revision(
    namespace: &StateNamespaceKey,
    revision: u64,
) -> DocumentNamespaceRevisionReport {
    DocumentNamespaceRevisionReport {
        scope: state_scope(namespace.scope).to_owned(),
        visibility: namespace_visibility(namespace.visibility).to_owned(),
        owner: namespace.owner.clone(),
        namespace: namespace.namespace.clone(),
        revision,
    }
}

fn state_scope(scope: StateScope) -> &'static str {
    match scope {
        StateScope::Invocation => "invocation",
        StateScope::Parse => "parse",
        StateScope::Document => "document",
        StateScope::Project => "project",
        StateScope::PersistentProject => "persistentProject",
    }
}

fn namespace_visibility(visibility: NamespaceVisibility) -> &'static str {
    match visibility {
        NamespaceVisibility::Private => "private",
        NamespaceVisibility::Shared => "shared",
    }
}

fn expansion_kind(kind: ExpansionKind) -> &'static str {
    match kind {
        ExpansionKind::Text => "text",
        ExpansionKind::Tree => "tree",
        ExpansionKind::Ast => "ast",
    }
}

fn document_node(node: &AstNode, catalog: &Catalog) -> DocumentNodeReport {
    let category = syntax_category(&node.kind);
    let identity = node.summary.as_ref().and_then(|summary| {
        let definition_id = summary
            .definition_id
            .as_deref()
            .or_else(|| metadata_entry(&node.metadata, "type-parser-definition-id"))
            .or_else(|| {
                node.syntax_id
                    .starts_with("type:")
                    .then_some(node.syntax_id.as_str())
            })?;
        let registration_id = summary
            .registration_id
            .as_deref()
            .or_else(|| metadata_entry(&node.metadata, "type-parser-registration-id"))
            .unwrap_or(definition_id);
        Some(syntax_identity_from_ids(
            definition_id,
            registration_id,
            catalog,
            category,
        ))
    });
    let pattern = node.summary.as_ref().and_then(|summary| {
        let source = metadata_entry(&node.metadata, "parser.pattern")?;
        let index = usize::try_from(summary.pattern_index?).ok()?;
        Some(pattern_report(index, source, catalog, true))
    });
    let semantics = node
        .summary
        .as_ref()
        .map(|summary| DocumentSemanticsReport {
            kind: summary.kind.clone(),
            return_type: summary.return_type.clone(),
            possible_return_types: summary.possible_return_types.clone(),
            possible_return_types_state: possible_return_types_state(
                summary.possible_return_types_state,
            )
            .to_owned(),
            multiplicity: summary
                .multiplicity
                .map(dynamic_multiplicity)
                .map(str::to_owned),
            default_expression: summary.default_expression.as_ref().map(|info| {
                DefaultExpressionReport {
                    capture_index: info.capture_index,
                    pattern_span: wit_range(&info.pattern_span),
                    expression: info.expression.clone(),
                    requested_type: DocumentExpectedType {
                        class_name: info.requested_type.class_name.clone(),
                        plural: info.requested_type.plural,
                    },
                    type_definition_id: info.type_definition_id.clone(),
                    type_registration_id: info.type_registration_id.clone(),
                    provider_id: info.provider_id.clone(),
                    component_id: info.component_id.clone(),
                    reason: info.reason.clone(),
                    event_classes: info.event_classes.clone(),
                    section_scope_ids: info.section_scope_ids.clone(),
                    catalog_references: info
                        .catalog_references
                        .iter()
                        .map(|reference| DefaultCatalogReferenceReport {
                            role: reference.role.clone(),
                            definition_id: reference.definition_id.clone(),
                            registration_id: reference.registration_id.clone(),
                            source_digest: reference.source_digest.clone(),
                            snapshot_id: reference.snapshot_id.clone(),
                            document: reference.document.clone(),
                            index: reference.index,
                        })
                        .collect(),
                    is_literal: info.is_literal,
                    time: info.time,
                    span: wit_span(&info.span),
                }
            }),
            public_data: summary
                .public_data
                .iter()
                .map(|data| DocumentPublicDataReport {
                    schema_id: data.schema_id.clone(),
                    schema_version: data.schema_version,
                    value: serde_json::from_str(&data.json)
                        .unwrap_or_else(|_| Value::String(data.json.clone())),
                })
                .collect(),
        });
    DocumentNodeReport {
        id: node.id,
        kind: node_kind(node),
        syntax_id: node.syntax_id.clone(),
        text: node.text.clone(),
        span: wit_span(&node.span),
        syntax_context: node.syntax_context,
        context_origin: ast_context_origin(node.context_origin).to_owned(),
        identity,
        pattern,
        semantics,
        captures: node
            .captures
            .iter()
            .map(|capture| DocumentCaptureReport {
                name: capture.name.clone(),
                value: match &capture.value {
                    CaptureValue::Text(value) => DocumentCaptureValueReport::Text(value.clone()),
                    CaptureValue::Node(value) => DocumentCaptureValueReport::Node(*value),
                    CaptureValue::Nodes(value) => DocumentCaptureValueReport::Nodes(value.clone()),
                    CaptureValue::Span(value) => DocumentCaptureValueReport::Span(wit_span(value)),
                },
            })
            .collect(),
        children: node.children.clone(),
        metadata: node
            .metadata
            .iter()
            .map(|entry| DocumentMetadataReport {
                key: entry.key.clone(),
                value: entry.value.clone(),
                owner_component_id: entry.owner_component_id.clone(),
            })
            .collect(),
    }
}

fn node_kind(node: &AstNode) -> String {
    if node.syntax_id.starts_with("parser:structure-entry.") {
        "Entry".to_owned()
    } else {
        match node.kind {
            WitSyntaxKind::Event => "Event",
            WitSyntaxKind::Condition => "Condition",
            WitSyntaxKind::Effect => "Effect",
            WitSyntaxKind::Expression => "Expression",
            WitSyntaxKind::Type => "Type",
            WitSyntaxKind::Function => "Function",
            WitSyntaxKind::Section => "Section",
            WitSyntaxKind::Structure => "Structure",
        }
        .to_owned()
    }
}

fn syntax_category(kind: &WitSyntaxKind) -> SyntaxCategory {
    match kind {
        WitSyntaxKind::Event => SyntaxCategory::Event,
        WitSyntaxKind::Condition => SyntaxCategory::Condition,
        WitSyntaxKind::Effect => SyntaxCategory::Effect,
        WitSyntaxKind::Expression => SyntaxCategory::Expression,
        WitSyntaxKind::Type => SyntaxCategory::Type,
        WitSyntaxKind::Function => SyntaxCategory::Function,
        WitSyntaxKind::Section => SyntaxCategory::Section,
        WitSyntaxKind::Structure => SyntaxCategory::Structure,
    }
}

fn metadata_entry<'a>(entries: &'a [WitMetadataEntry], key: &str) -> Option<&'a str> {
    entries
        .iter()
        .find(|entry| entry.key == key || entry.key.ends_with(&format!("/{key}")))
        .map(|entry| entry.value.as_str())
}

fn recoveries(
    document: &skript_parser::StructureDocument,
    raw_tree: &RawTree,
    catalog: &Catalog,
) -> Vec<RecoveryReport> {
    let mut output = Vec::new();
    for root in &document.roots {
        match root {
            StructureDocumentNode::Structure(matches) => {
                collect_structure_recoveries(matches, raw_tree, catalog, &mut output);
            }
            StructureDocumentNode::Unclaimed(id) => {
                if let Some(node) = raw_tree.get(*id) {
                    output.push(RecoveryReport {
                        kind: "RawNode".to_owned(),
                        source: node.text.clone(),
                        span: native_span(node.code_span.as_ref().unwrap_or(&node.span)),
                        failure: None,
                    });
                }
            }
            StructureDocumentNode::Trivia(_) => {}
        }
    }
    output
}

fn collect_structure_recoveries(
    matches: &StructureMatches,
    raw_tree: &RawTree,
    catalog: &Catalog,
    output: &mut Vec<RecoveryReport>,
) {
    if let Some(selected) = &matches.selected {
        collect_body_recoveries(&selected.body, raw_tree, catalog, output);
    } else if let Some(unknown) = &matches.unknown {
        output.push(RecoveryReport {
            kind: "Structure".to_owned(),
            source: unknown.source.clone(),
            span: native_span(&unknown.span.mapped),
            failure: unknown.failure.clone().map(failure_trace_report),
        });
        if let Some(partial) = &unknown.partial {
            collect_body_recoveries(&partial.body, raw_tree, catalog, output);
        }
    }
}

fn collect_body_recoveries(
    body: &StructureBody,
    raw_tree: &RawTree,
    catalog: &Catalog,
    output: &mut Vec<RecoveryReport>,
) {
    if let StructureBody::Trigger(nodes) = body {
        collect_section_body_recoveries(nodes, raw_tree, catalog, output);
    }
}

fn collect_section_body_recoveries(
    nodes: &[SectionBodyNode],
    raw_tree: &RawTree,
    catalog: &Catalog,
    output: &mut Vec<RecoveryReport>,
) {
    for node in nodes {
        match node {
            SectionBodyNode::Section(matches) => {
                collect_section_recoveries(matches, raw_tree, catalog, output)
            }
            SectionBodyNode::Effect(matches) => {
                if let Some(unknown) = &matches.unknown {
                    let failure = unknown
                        .failures
                        .primary()
                        .map(|candidate| {
                            candidate_failure_report(
                                candidate,
                                &unknown.failures.candidates,
                                catalog,
                            )
                        })
                        .or_else(|| unknown.failures.fallback.clone().map(failure_trace_report));
                    output.push(RecoveryReport {
                        kind: "Effect".to_owned(),
                        source: unknown.source.clone(),
                        span: native_span(&unknown.span.mapped),
                        failure,
                    });
                }
            }
            SectionBodyNode::Condition { matches, .. } => {
                if let Some(unknown) = &matches.unknown {
                    output.push(RecoveryReport {
                        kind: "Condition".to_owned(),
                        source: unknown.source.clone(),
                        span: native_span(&unknown.span.mapped),
                        failure: unknown.failure.clone().map(failure_trace_report),
                    });
                }
            }
            SectionBodyNode::Unclaimed(id) => {
                if let Some(node) = raw_tree.get(*id) {
                    output.push(RecoveryReport {
                        kind: "RawNode".to_owned(),
                        source: node.text.clone(),
                        span: native_span(node.code_span.as_ref().unwrap_or(&node.span)),
                        failure: None,
                    });
                }
            }
            SectionBodyNode::Trivia(_) => {}
        }
    }
}

fn collect_section_recoveries(
    matches: &SectionMatches,
    raw_tree: &RawTree,
    catalog: &Catalog,
    output: &mut Vec<RecoveryReport>,
) {
    if let Some(selected) = &matches.selected {
        collect_section_body_recoveries(&selected.body, raw_tree, catalog, output);
    } else if let Some(unknown) = &matches.unknown {
        output.push(RecoveryReport {
            kind: "Section".to_owned(),
            source: unknown.source.clone(),
            span: native_span(&unknown.span.mapped),
            failure: unknown.failure.clone().map(failure_trace_report),
        });
        collect_section_body_recoveries(&unknown.body, raw_tree, catalog, output);
    }
}

fn raw_diagnostics(tree: &skript_parser::RawTree) -> Vec<DocumentDiagnosticReport> {
    tree.diagnostics
        .iter()
        .map(|diagnostic| DocumentDiagnosticReport {
            code: format!("parser.raw-tree.{}", diagnostic.code.as_str()),
            message: diagnostic.message.clone(),
            severity: raw_diagnostic_severity(diagnostic.severity).to_owned(),
            span: native_span(&diagnostic.span),
            related: diagnostic
                .related
                .iter()
                .map(|related| DocumentRelatedDiagnosticReport {
                    message: related.message.clone(),
                    span: native_span(&related.span),
                })
                .collect(),
        })
        .collect()
}

fn collect_structure_diagnostics(
    roots: &[StructureDocumentNode],
    output: &mut Vec<DocumentDiagnosticReport>,
) {
    for root in roots {
        let StructureDocumentNode::Structure(matches) = root else {
            continue;
        };
        output.extend(
            matches
                .diagnostics
                .iter()
                .map(|diagnostic| DocumentDiagnosticReport {
                    code: format!(
                        "parser.structure.{}",
                        structure_diagnostic_kind(diagnostic.kind)
                    ),
                    message: diagnostic.message.clone(),
                    severity: "error".to_owned(),
                    span: native_span(&diagnostic.span.mapped),
                    related: Vec::new(),
                }),
        );
        if let Some(selected) = &matches.selected {
            collect_body_diagnostics(&selected.body, output);
        }
    }
}

fn collect_body_diagnostics(body: &StructureBody, output: &mut Vec<DocumentDiagnosticReport>) {
    if let StructureBody::Trigger(nodes) = body {
        collect_section_body_diagnostics(nodes, output);
    }
}

fn collect_section_body_diagnostics(
    nodes: &[SectionBodyNode],
    output: &mut Vec<DocumentDiagnosticReport>,
) {
    for node in nodes {
        if let SectionBodyNode::Section(matches) = node {
            output.extend(
                matches
                    .diagnostics
                    .iter()
                    .map(|diagnostic| DocumentDiagnosticReport {
                        code: format!(
                            "parser.section.{}",
                            section_diagnostic_kind(diagnostic.kind)
                        ),
                        message: section_diagnostic_message(diagnostic.kind).to_owned(),
                        severity: "error".to_owned(),
                        span: native_span(&diagnostic.span.mapped),
                        related: Vec::new(),
                    }),
            );
            if let Some(selected) = &matches.selected {
                collect_section_body_diagnostics(&selected.body, output);
            }
        }
    }
}

fn hook_diagnostic(diagnostic: &parser_wasm::host::Diagnostic) -> DocumentDiagnosticReport {
    DocumentDiagnosticReport {
        code: diagnostic.code.clone(),
        message: diagnostic.message.clone(),
        severity: diagnostic_severity_name(diagnostic.severity).to_owned(),
        span: wit_span(&diagnostic.span),
        related: diagnostic
            .related
            .iter()
            .map(|related| DocumentRelatedDiagnosticReport {
                message: related.message.clone(),
                span: wit_span(&related.span),
            })
            .collect(),
    }
}

fn wit_span(span: &WitMappedSpan) -> MappedSpanReport {
    MappedSpanReport {
        virtual_range: wit_range(&span.virtual_range),
        origins: span
            .origins
            .iter()
            .map(|origin| DocumentOriginReport {
                original_range: wit_range(&origin.original_range),
                kind: wit_origin_kind_name(origin.kind).to_owned(),
                expansion_id: origin.expansion,
            })
            .collect(),
    }
}

fn wit_range(range: &WitTextRange) -> DocumentTextRange {
    DocumentTextRange {
        start: range.start,
        end: range.end,
    }
}

fn native_range(range: skript_parser::TextRange) -> DocumentTextRange {
    DocumentTextRange {
        start: u64::try_from(range.start).unwrap_or(u64::MAX),
        end: u64::try_from(range.end).unwrap_or(u64::MAX),
    }
}

fn native_span(span: &MappedSpan) -> MappedSpanReport {
    MappedSpanReport {
        virtual_range: DocumentTextRange {
            start: u64::try_from(span.virtual_range.start).unwrap_or(u64::MAX),
            end: u64::try_from(span.virtual_range.end).unwrap_or(u64::MAX),
        },
        origins: span
            .origins
            .iter()
            .map(|origin| DocumentOriginReport {
                original_range: DocumentTextRange {
                    start: u64::try_from(origin.original_range.start).unwrap_or(u64::MAX),
                    end: u64::try_from(origin.original_range.end).unwrap_or(u64::MAX),
                },
                kind: native_origin_kind_name(origin.kind).to_owned(),
                expansion_id: origin.expansion.map(|id| u64::from(id.get())),
            })
            .collect(),
    }
}

fn display_range(span: &MappedSpanReport) -> DocumentTextRange {
    span.origins
        .first()
        .map(|origin| origin.original_range)
        .unwrap_or(span.virtual_range)
}

#[derive(Clone, Copy)]
enum SourceSpace {
    Original,
    Virtual,
}

fn document_source_colors(
    roots: &[u64],
    nodes: &[DocumentNodeReport],
    source: &str,
    space: SourceSpace,
) -> Vec<SourceColorSpan> {
    let nodes_by_id = nodes.iter().map(|node| (node.id, node)).collect();
    let mut deepest_visit = HashMap::new();
    let mut spans = Vec::new();
    for root in roots {
        collect_document_source_colors(
            *root,
            0,
            &nodes_by_id,
            &mut deepest_visit,
            &mut spans,
            source,
            space,
        );
    }
    spans
}

fn collect_document_source_colors(
    id: u64,
    depth: usize,
    nodes: &HashMap<u64, &DocumentNodeReport>,
    deepest_visit: &mut HashMap<u64, usize>,
    spans: &mut Vec<SourceColorSpan>,
    source: &str,
    space: SourceSpace,
) {
    if depth > MAX_HUMAN_TREE_DEPTH
        || deepest_visit
            .get(&id)
            .is_some_and(|previous| *previous >= depth)
    {
        return;
    }
    deepest_visit.insert(id, depth);
    let Some(node) = nodes.get(&id) else {
        return;
    };
    if let Some(color) = document_node_color(node) {
        for span in document_node_spans(node, space) {
            push_source_color_span(spans, span, color, depth);
        }
    }
    for alias_span in document_alias_spans(node, space) {
        push_source_color_span(spans, alias_span, SourceColor::Alias, depth + 1);
    }
    if is_document_variable_string(node) {
        let child_spans = node
            .children
            .iter()
            .filter_map(|child| nodes.get(child))
            .flat_map(|child| document_node_spans(child, space))
            .collect::<Vec<_>>();
        for parent in document_node_spans(node, space) {
            for delimiter in
                interpolation_delimiter_spans(source, parent, child_spans.iter().copied())
            {
                push_source_color_span(
                    spans,
                    delimiter,
                    SourceColor::InterpolationDelimiter,
                    depth + 1,
                );
            }
        }
    }
    for child in &node.children {
        collect_document_source_colors(
            *child,
            depth + 1,
            nodes,
            deepest_visit,
            spans,
            source,
            space,
        );
    }
}

fn document_node_color(node: &DocumentNodeReport) -> Option<SourceColor> {
    match node.kind.as_str() {
        "Structure" | "Entry" => Some(SourceColor::Structure),
        "Section" => Some(SourceColor::Section),
        "Event" => Some(SourceColor::Event),
        "Condition" => Some(SourceColor::Condition),
        "Effect" => Some(SourceColor::Effect),
        "Expression" if node.syntax_id.ends_with("core.variable") => Some(SourceColor::Variable),
        "Expression" if is_document_variable_string(node) => Some(SourceColor::Literal),
        "Expression" => Some(SourceColor::Expression),
        "Type"
            if node.syntax_id == "core.literal.class-info"
                || document_metadata_value(node, "type-code-name") == Some("classinfo") =>
        {
            Some(SourceColor::TypeName)
        }
        "Type" => Some(SourceColor::Literal),
        "Function" => Some(SourceColor::Function),
        _ => None,
    }
}

fn is_document_variable_string(node: &DocumentNodeReport) -> bool {
    is_variable_string_syntax_id(&node.syntax_id)
        || document_metadata_value(node, "embedded-expression-count").is_some()
}

fn document_node_spans(node: &DocumentNodeReport, space: SourceSpace) -> Vec<SpanReport> {
    let ranges = match space {
        SourceSpace::Original if !node.span.origins.is_empty() => node
            .span
            .origins
            .iter()
            .map(|origin| origin.original_range)
            .collect::<Vec<_>>(),
        SourceSpace::Original | SourceSpace::Virtual => vec![node.span.virtual_range],
    };
    ranges
        .into_iter()
        .filter_map(span_report)
        .collect::<Vec<_>>()
}

fn span_report(range: DocumentTextRange) -> Option<SpanReport> {
    Some(SpanReport {
        start: usize::try_from(range.start).ok()?,
        end: usize::try_from(range.end).ok()?,
    })
}

fn document_alias_spans(node: &DocumentNodeReport, space: SourceSpace) -> Vec<SpanReport> {
    if document_metadata_value(node, "literal-source") != Some("alias") {
        return Vec::new();
    }
    let alias = document_metadata_value(node, "literal-range-start")
        .and_then(|start| start.parse().ok())
        .zip(document_metadata_value(node, "literal-range-end").and_then(|end| end.parse().ok()))
        .map(|(start, end)| SpanReport { start, end });
    if matches!(space, SourceSpace::Virtual) {
        return alias
            .into_iter()
            .chain(document_node_spans(node, space).into_iter().take(1))
            .take(1)
            .collect();
    }

    let Some(alias) = alias else {
        return document_node_spans(node, space);
    };
    let Some(virtual_span) = span_report(node.span.virtual_range) else {
        return Vec::new();
    };
    if alias.start < virtual_span.start || virtual_span.end < alias.end {
        return Vec::new();
    }
    node.span
        .origins
        .iter()
        .filter(|origin| origin.kind == "exact")
        .filter_map(|origin| {
            let original_span = span_report(origin.original_range)?;
            (virtual_span.end.saturating_sub(virtual_span.start)
                == original_span.end.saturating_sub(original_span.start))
            .then_some(SpanReport {
                start: original_span
                    .start
                    .saturating_add(alias.start - virtual_span.start),
                end: original_span
                    .start
                    .saturating_add(alias.end - virtual_span.start),
            })
        })
        .collect()
}

fn document_metadata_value<'a>(node: &'a DocumentNodeReport, key: &str) -> Option<&'a str> {
    node.metadata.iter().find_map(|entry| {
        (entry.key == key
            || entry
                .key
                .strip_suffix(key)
                .is_some_and(|owner| owner.ends_with('/')))
        .then_some(entry.value.as_str())
    })
}

fn write_source_lines(
    writer: &mut dyn Write,
    source: &str,
    spans: &[SourceColorSpan],
    color: bool,
) -> io::Result<()> {
    let mut line_start = 0;
    for (index, chunk) in source.split_inclusive('\n').enumerate() {
        let line = chunk.strip_suffix('\n').unwrap_or(chunk);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if color {
            let line_end = line_start + line.len();
            let line_spans = spans
                .iter()
                .filter_map(|span| {
                    let start = span.span.start.max(line_start);
                    let end = span.span.end.min(line_end);
                    (start < end).then(|| SourceColorSpan {
                        span: SpanReport {
                            start: start - line_start,
                            end: end - line_start,
                        },
                        color: span.color,
                        depth: span.depth,
                        order: span.order,
                    })
                })
                .collect::<Vec<_>>();
            writeln!(
                writer,
                "  {:>3} | {}",
                index + 1,
                render_source_colors(line, &line_spans)
            )?;
        } else {
            writeln!(writer, "  {:>3} | {line}", index + 1)?;
        }
        line_start += chunk.len();
    }
    Ok(())
}

fn possible_return_types_state(state: ExpressionPossibleReturnTypesState) -> &'static str {
    match state {
        ExpressionPossibleReturnTypesState::Complete => "complete",
        ExpressionPossibleReturnTypesState::Partial => "partial",
        ExpressionPossibleReturnTypesState::Unresolved => "unresolved",
    }
}

fn dynamic_multiplicity(multiplicity: DynamicMultiplicity) -> &'static str {
    match multiplicity {
        DynamicMultiplicity::Single => "single",
        DynamicMultiplicity::Multiple => "multiple",
        DynamicMultiplicity::Both => "both",
    }
}

fn ast_context_origin(origin: AstContextOrigin) -> &'static str {
    match origin {
        AstContextOrigin::Preserved => "preserved",
        AstContextOrigin::Macro => "macro",
        AstContextOrigin::CallSite => "callSite",
        AstContextOrigin::DefinitionSite => "definitionSite",
    }
}

fn raw_diagnostic_severity(severity: RawDiagnosticSeverity) -> &'static str {
    match severity {
        RawDiagnosticSeverity::Error => "error",
        RawDiagnosticSeverity::Warning => "warning",
    }
}

fn structure_diagnostic_kind(kind: StructureDiagnosticKind) -> &'static str {
    match kind {
        StructureDiagnosticKind::Unclaimed => "unclaimed",
        StructureDiagnosticKind::MultipleClaims => "multiple-claims",
        StructureDiagnosticKind::MissingRequiredEntry => "missing-required-entry",
        StructureDiagnosticKind::DuplicateEntry => "duplicate-entry",
        StructureDiagnosticKind::InvalidEntryValue => "invalid-entry-value",
        StructureDiagnosticKind::UnknownEntryData => "unknown-entry-data",
    }
}

fn section_diagnostic_kind(kind: SectionDiagnosticKind) -> &'static str {
    match kind {
        SectionDiagnosticKind::Unclaimed => "unclaimed",
        SectionDiagnosticKind::MultipleClaims => "multiple-claims",
    }
}

fn section_diagnostic_message(kind: SectionDiagnosticKind) -> &'static str {
    match kind {
        SectionDiagnosticKind::Unclaimed => "Section was not claimed by any registered syntax",
        SectionDiagnosticKind::MultipleClaims => {
            "Section was claimed by more than one registered syntax"
        }
    }
}

fn write_document_diagnostic(
    writer: &mut dyn Write,
    source: &str,
    diagnostic: &DocumentDiagnosticReport,
    color: bool,
) -> io::Result<()> {
    let primary = display_range(&diagnostic.span);
    let start = usize::try_from(primary.start)
        .unwrap_or(usize::MAX)
        .min(source.len());
    let end = usize::try_from(primary.end)
        .unwrap_or(usize::MAX)
        .min(source.len())
        .max(start);
    let mut labels = vec![LabeledSpan::new(
        Some(diagnostic.message.clone()),
        start,
        end - start,
    )];
    let mut seen_ranges = HashSet::from([(primary.start, primary.end)]);
    for origin in diagnostic.span.origins.iter().skip(1) {
        if !seen_ranges.insert((origin.original_range.start, origin.original_range.end)) {
            continue;
        }
        let origin_start = usize::try_from(origin.original_range.start)
            .unwrap_or(usize::MAX)
            .min(source.len());
        let origin_end = usize::try_from(origin.original_range.end)
            .unwrap_or(usize::MAX)
            .min(source.len())
            .max(origin_start);
        labels.push(LabeledSpan::new(
            Some("also originates here".to_owned()),
            origin_start,
            origin_end - origin_start,
        ));
    }
    for related in &diagnostic.related {
        let related_primary = display_range(&related.span);
        let related_start = usize::try_from(related_primary.start)
            .unwrap_or(usize::MAX)
            .min(source.len());
        let related_end = usize::try_from(related_primary.end)
            .unwrap_or(usize::MAX)
            .min(source.len())
            .max(related_start);
        labels.push(LabeledSpan::new(
            Some(related.message.clone()),
            related_start,
            related_end - related_start,
        ));
        for origin in related.span.origins.iter().skip(1) {
            if !seen_ranges.insert((origin.original_range.start, origin.original_range.end)) {
                continue;
            }
            let origin_start = usize::try_from(origin.original_range.start)
                .unwrap_or(usize::MAX)
                .min(source.len());
            let origin_end = usize::try_from(origin.original_range.end)
                .unwrap_or(usize::MAX)
                .min(source.len())
                .max(origin_start);
            labels.push(LabeledSpan::new(
                Some(format!("related origin: {}", related.message)),
                origin_start,
                origin_end - origin_start,
            ));
        }
    }
    let report = miette::Report::new(
        MietteDiagnostic::new(diagnostic.message.clone())
            .with_code(format!("skript-repl::{}", diagnostic.code))
            .with_labels(labels),
    )
    .with_source_code(NamedSource::new("repl.sk", source.to_owned()));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn source_node(
        id: u64,
        kind: &str,
        syntax_id: &str,
        start: u64,
        end: u64,
        children: Vec<u64>,
    ) -> DocumentNodeReport {
        DocumentNodeReport {
            id,
            kind: kind.to_owned(),
            syntax_id: syntax_id.to_owned(),
            text: String::new(),
            span: MappedSpanReport {
                virtual_range: DocumentTextRange { start, end },
                origins: vec![DocumentOriginReport {
                    original_range: DocumentTextRange { start, end },
                    kind: "exact".to_owned(),
                    expansion_id: None,
                }],
            },
            syntax_context: 0,
            context_origin: "preserved".to_owned(),
            identity: None,
            pattern: None,
            semantics: None,
            captures: Vec::new(),
            children,
            metadata: Vec::new(),
        }
    }

    #[test]
    fn document_source_uses_nested_syntax_colors_without_coloring_line_numbers() {
        let source = "on join:\n    send \"hello\" to console\n";
        let end = source.len() as u64;
        let nodes = vec![
            source_node(1, "Structure", "structure", 0, end, vec![2]),
            source_node(2, "Event", "event", 0, end, vec![3]),
            source_node(3, "Effect", "effect", 13, end - 1, vec![4]),
            source_node(4, "Type", "core.literal.string", 18, 25, Vec::new()),
        ];
        let spans = document_source_colors(&[1], &nodes, source, SourceSpace::Original);
        let mut colored = Vec::new();

        write_source_lines(&mut colored, source, &spans, true).unwrap();

        let colored = String::from_utf8(colored).unwrap();
        assert!(colored.contains("\x1b[38;2;197;95;115mon join:"));
        assert!(colored.contains("\x1b[38;2;88;196;221msend "));
        assert!(colored.contains("\x1b[38;2;255;255;255m\"hello\""));
        assert!(colored.contains("\x1b[0m\n    2 |"));

        let mut plain = Vec::new();
        write_source_lines(&mut plain, source, &spans, false).unwrap();
        assert!(!plain.contains(&b'\x1b'));
    }

    #[test]
    fn document_source_colors_all_origins_and_prefixed_variable_ids() {
        let mut variable = source_node(
            1,
            "Expression",
            "parser:expression.core.variable",
            0,
            1,
            Vec::new(),
        );
        variable.span.origins.push(DocumentOriginReport {
            original_range: DocumentTextRange { start: 3, end: 4 },
            kind: "replaced".to_owned(),
            expansion_id: Some(1),
        });

        assert_eq!(document_node_color(&variable), Some(SourceColor::Variable));
        let spans = document_source_colors(&[1], &[variable], "a  b", SourceSpace::Original);
        assert!(
            spans
                .iter()
                .any(|span| span.span.start == 0 && span.span.end == 1)
        );
        assert!(
            spans
                .iter()
                .any(|span| span.span.start == 3 && span.span.end == 4)
        );
    }

    #[test]
    fn document_source_colors_variable_string_delimiters_and_inner_expression() {
        let source = r#"send "hello %player%""#;
        let mut string = source_node(2, "Type", "type:skript:string", 5, 21, vec![3]);
        string.metadata.push(DocumentMetadataReport {
            key: "embedded-expression-count".to_owned(),
            value: "1".to_owned(),
            owner_component_id: Some("nlaocs.core-library".to_owned()),
        });
        let nodes = vec![
            source_node(1, "Effect", "effect", 0, 21, vec![2]),
            string,
            source_node(3, "Expression", "registered-expression", 13, 19, Vec::new()),
        ];
        let spans = document_source_colors(&[1], &nodes, source, SourceSpace::Original);

        assert_eq!(
            render_source_colors(source, &spans),
            concat!(
                "\x1b[38;2;88;196;221msend ",
                "\x1b[38;2;255;255;255m\"hello ",
                "\x1b[38;2;176;176;176m%",
                "\x1b[38;2;131;193;103mplayer",
                "\x1b[38;2;176;176;176m%",
                "\x1b[38;2;255;255;255m\"",
                "\x1b[0m"
            )
        );
    }

    #[test]
    fn document_source_colors_crlf_lines_independently() {
        let source = "a\r\nb\r\n";
        let spans = vec![
            SourceColorSpan {
                span: SpanReport { start: 0, end: 1 },
                color: SourceColor::Event,
                depth: 0,
                order: 0,
            },
            SourceColorSpan {
                span: SpanReport { start: 3, end: 4 },
                color: SourceColor::Effect,
                depth: 0,
                order: 1,
            },
        ];
        let mut rendered = Vec::new();

        write_source_lines(&mut rendered, source, &spans, true).unwrap();

        let rendered = String::from_utf8(rendered).unwrap();
        assert!(rendered.contains("   1 | \x1b[38;2;197;95;115ma\x1b[0m\n"));
        assert!(rendered.contains("   2 | \x1b[38;2;88;196;221mb\x1b[0m\n"));
        assert!(!rendered.contains('\r'));
    }

    #[test]
    fn human_diagnostics_render_every_distinct_original_origin() {
        let diagnostic = DocumentDiagnosticReport {
            code: "fixture.multi-origin".to_owned(),
            message: "generated syntax joins both ranges".to_owned(),
            severity: "error".to_owned(),
            span: MappedSpanReport {
                virtual_range: DocumentTextRange { start: 0, end: 1 },
                origins: vec![
                    DocumentOriginReport {
                        original_range: DocumentTextRange { start: 0, end: 1 },
                        kind: "replaced".to_owned(),
                        expansion_id: Some(1),
                    },
                    DocumentOriginReport {
                        original_range: DocumentTextRange { start: 4, end: 5 },
                        kind: "replaced".to_owned(),
                        expansion_id: Some(1),
                    },
                ],
            },
            related: Vec::new(),
        };
        let mut rendered = Vec::new();

        write_document_diagnostic(&mut rendered, "a + b", &diagnostic, false).unwrap();

        let rendered = String::from_utf8(rendered).unwrap();
        assert!(rendered.contains("generated syntax joins both ranges"));
        assert!(rendered.contains("also originates here"));
    }
}

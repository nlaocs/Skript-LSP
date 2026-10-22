use super::positions::LineIndex;
use lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location, NumberOrString, Range,
    Uri,
};
use parser_wasm::{DocumentParseResult, host};
use serde_json::json;
use skript_parser::{
    MappedSpan, RawDiagnosticSeverity, RawNodeId, SectionBodyNode, SectionDiagnosticKind,
    StructureBody, StructureDiagnosticKind, StructureDocumentNode, StructureEntryValue, TextRange,
};
use std::collections::BTreeSet;

pub(super) fn fatal(code: &str, message: String) -> Diagnostic {
    Diagnostic {
        range: Range::default(),
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(NumberOrString::String(code.into())),
        source: Some("skript-lsp".into()),
        message,
        ..Diagnostic::default()
    }
}

struct Collector<'a> {
    uri: &'a Uri,
    source: &'a str,
    lines: LineIndex,
    diagnostics: Vec<Diagnostic>,
    unclaimed: Vec<(Option<RawNodeId>, usize)>,
    explained_body_nodes: BTreeSet<RawNodeId>,
}
impl<'a> Collector<'a> {
    fn new(uri: &'a Uri, source: &'a str) -> Self {
        Self {
            uri,
            source,
            lines: LineIndex::new(source),
            diagnostics: Vec::new(),
            unclaimed: Vec::new(),
            explained_body_nodes: BTreeSet::new(),
        }
    }
    fn finish(self) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        let redundant = self
            .unclaimed
            .into_iter()
            .filter(|(node, _)| node.is_some_and(|node| self.explained_body_nodes.contains(&node)))
            .map(|(_, index)| index)
            .collect::<BTreeSet<_>>();
        for (index, diagnostic) in self.diagnostics.into_iter().enumerate() {
            if !redundant.contains(&index) && !diagnostics.contains(&diagnostic) {
                diagnostics.push(diagnostic);
            }
        }
        diagnostics
    }
    fn hook(&mut self, diagnostic: &host::Diagnostic) -> Result<(), String> {
        let related = diagnostic
            .related
            .iter()
            .map(|related| Ok((related.message.clone(), wit_origins(&related.span)?)))
            .collect::<Result<_, String>>()?;
        self.add(
            &diagnostic.code,
            &diagnostic.message,
            match diagnostic.severity {
                host::DiagnosticSeverity::Error => DiagnosticSeverity::ERROR,
                host::DiagnosticSeverity::Warning => DiagnosticSeverity::WARNING,
                host::DiagnosticSeverity::Information => DiagnosticSeverity::INFORMATION,
                host::DiagnosticSeverity::Hint => DiagnosticSeverity::HINT,
            },
            wit_origins(&diagnostic.span)?,
            related,
        )
    }
    fn macro_decision(&mut self, stage: &str, decision: &host::HookDecision) -> Result<(), String> {
        if let host::HookDecision::Reject(rejection) = decision {
            if rejection.diagnostics.is_empty() {
                self.add(
                    "parser.macro-rejected",
                    &format!("{stage} macro rejected the document: {}", rejection.reason),
                    DiagnosticSeverity::ERROR,
                    vec![TextRange::new(0, self.source.len())],
                    Vec::new(),
                )?;
            }
            for diagnostic in &rejection.diagnostics {
                self.hook(diagnostic)?;
            }
        }
        Ok(())
    }
    fn body_trace(
        &mut self,
        node: RawNodeId,
        trace: &skript_parser::FailureTrace,
    ) -> Result<(), String> {
        // Causal ownership survives macro mapping and narrower capture ranges.
        // A root/header failure or an unrelated overlapping diagnostic cannot
        // explain this child just because it has the same original position.
        if self.trace(trace)? {
            self.explained_body_nodes.insert(node);
        }
        Ok(())
    }
    fn trace(&mut self, trace: &skript_parser::FailureTrace) -> Result<bool, String> {
        let semantic = trace.semantic_diagnostics();
        let explained;
        if !semantic.is_empty() {
            explained = semantic.iter().any(|diagnostic| {
                diagnostic.severity == skript_parser::SemanticDiagnosticSeverity::Error
            });
            for diagnostic in semantic {
                let severity = match diagnostic.severity {
                    skript_parser::SemanticDiagnosticSeverity::Error => DiagnosticSeverity::ERROR,
                    skript_parser::SemanticDiagnosticSeverity::Warning => {
                        DiagnosticSeverity::WARNING
                    }
                    skript_parser::SemanticDiagnosticSeverity::Information => {
                        DiagnosticSeverity::INFORMATION
                    }
                    skript_parser::SemanticDiagnosticSeverity::Hint => DiagnosticSeverity::HINT,
                };
                self.add(
                    &diagnostic.code,
                    &diagnostic.message,
                    severity,
                    native_origins(&diagnostic.span),
                    diagnostic
                        .related
                        .iter()
                        .map(|related| (related.message.clone(), native_origins(&related.span)))
                        .collect(),
                )?;
            }
        } else {
            let mut deepest = trace;
            while let Some(cause) = deepest.cause.as_deref() {
                deepest = cause;
            }
            let reasons = deepest
                .failure
                .reasons
                .iter()
                .map(failure_reason)
                .collect::<Vec<_>>();
            explained = !reasons.is_empty();
            let message = if reasons.is_empty() {
                "No registered syntax accepted this input".to_owned()
            } else {
                reasons.join("; ")
            };
            let mismatch = deepest
                .failure
                .reasons
                .iter()
                .find_map(|reason| match reason {
                    skript_parser::PatternFailureReason::TypeMismatch { expected, actual } => {
                        Some((expected, actual))
                    }
                    _ => None,
                });
            self.add(
                if mismatch.is_some() {
                    "parser.type-mismatch"
                } else {
                    "parser.syntax-incomplete"
                },
                &message,
                DiagnosticSeverity::ERROR,
                native_origins(&deepest.failure.span.mapped),
                Vec::new(),
            )?;
            if let Some((expected, actual)) = mismatch {
                self.diagnostics
                    .last_mut()
                    .expect("just added diagnostic")
                    .data = Some(json!({
                    "expectedTypes": expected,
                    "actualTypes": actual,
                }));
            }
        }
        Ok(explained)
    }
    fn add(
        &mut self,
        code: &str,
        message: &str,
        severity: DiagnosticSeverity,
        origins: Vec<TextRange>,
        related: Vec<(String, Vec<TextRange>)>,
    ) -> Result<(), String> {
        let diagnostic = self.make_diagnostic(code, message, severity, origins, related)?;
        // Equal payloads can have different owners. Deduplicate only after
        // redundant summaries have been removed, so a concrete addon error
        // cannot disappear together with an identical parser summary.
        self.diagnostics.push(diagnostic);
        Ok(())
    }
    fn make_diagnostic(
        &self,
        code: &str,
        message: &str,
        severity: DiagnosticSeverity,
        origins: Vec<TextRange>,
        related: Vec<(String, Vec<TextRange>)>,
    ) -> Result<Diagnostic, String> {
        let primary = *origins
            .first()
            .ok_or("diagnostic has no original source origin")?;
        let mut information = Vec::new();
        for range in origins.iter().skip(1) {
            if *range != primary {
                information.push(DiagnosticRelatedInformation {
                    location: Location::new(
                        self.uri.clone(),
                        self.lines.range(self.source, *range)?,
                    ),
                    message: "Also originates here".into(),
                });
            }
        }
        for (message, ranges) in related {
            for range in ranges {
                information.push(DiagnosticRelatedInformation {
                    location: Location::new(
                        self.uri.clone(),
                        self.lines.range(self.source, range)?,
                    ),
                    message: message.clone(),
                });
            }
        }
        Ok(Diagnostic {
            range: self.lines.range(self.source, primary)?,
            severity: Some(severity),
            code: Some(NumberOrString::String(code.into())),
            source: Some("skript-lsp".into()),
            message: message.into(),
            related_information: (!information.is_empty()).then_some(information),
            ..Diagnostic::default()
        })
    }
    fn unclaimed(
        &mut self,
        node: Option<RawNodeId>,
        code: &str,
        message: &str,
        span: &MappedSpan,
    ) -> Result<(), String> {
        let diagnostic = self.make_diagnostic(
            code,
            message,
            DiagnosticSeverity::ERROR,
            native_origins(span),
            Vec::new(),
        )?;
        self.unclaimed.push((node, self.diagnostics.len()));
        self.diagnostics.push(diagnostic);
        Ok(())
    }
    fn structure(&mut self, diagnostic: &skript_parser::StructureDiagnostic) -> Result<(), String> {
        if diagnostic.kind == StructureDiagnosticKind::Unclaimed {
            return self.unclaimed(
                diagnostic.raw_node_id,
                "parser.structure.unclaimed",
                &diagnostic.message,
                &diagnostic.span.mapped,
            );
        }
        let kind = match diagnostic.kind {
            StructureDiagnosticKind::ExtensionFault => "extension-fault",
            StructureDiagnosticKind::Unclaimed => "unclaimed",
            StructureDiagnosticKind::MultipleClaims => "multiple-claims",
            StructureDiagnosticKind::MissingRequiredEntry => "missing-required-entry",
            StructureDiagnosticKind::DuplicateEntry => "duplicate-entry",
            StructureDiagnosticKind::InvalidEntryValue => "invalid-entry-value",
            StructureDiagnosticKind::UnknownEntryData => "unknown-entry-data",
        };
        self.add(
            &format!("parser.structure.{kind}"),
            &diagnostic.message,
            DiagnosticSeverity::ERROR,
            native_origins(&diagnostic.span.mapped),
            Vec::new(),
        )
    }
    fn body(&mut self, body: &StructureBody) -> Result<(), String> {
        match body {
            StructureBody::Trigger(nodes) => self.section_body(nodes)?,
            StructureBody::Entries(entries) => self.entries(entries)?,
            _ => {}
        }
        Ok(())
    }
    fn entries(&mut self, entries: &[skript_parser::StructureEntry]) -> Result<(), String> {
        for entry in entries {
            match &entry.value {
                StructureEntryValue::Trigger(nodes) => self.section_body(nodes)?,
                StructureEntryValue::Container(entries) => self.entries(entries)?,
                _ => {}
            }
        }
        Ok(())
    }
    fn section_body(&mut self, nodes: &[SectionBodyNode]) -> Result<(), String> {
        for node in nodes {
            if let SectionBodyNode::Condition {
                raw_node_id,
                matches,
            } = node
            {
                if let Some(trace) = matches
                    .unknown
                    .as_ref()
                    .and_then(|unknown| unknown.failure.as_ref())
                {
                    self.body_trace(*raw_node_id, trace)?;
                }
            }
            if let SectionBodyNode::Effect(matches) = node {
                if let Some(unknown) = &matches.unknown {
                    if let Some(primary) = unknown.failures.primary() {
                        self.body_trace(unknown.raw_node_id, &primary.matched.trace)?;
                        for related in &primary.matched.related {
                            self.body_trace(unknown.raw_node_id, related)?;
                        }
                    } else if let Some(fallback) = &unknown.failures.fallback {
                        self.body_trace(unknown.raw_node_id, fallback)?;
                    }
                }
            }
            if let SectionBodyNode::Section(matches) = node {
                if let Some(unknown) = &matches.unknown {
                    if let Some(trace) = &unknown.failure {
                        self.body_trace(unknown.raw_node_id, trace)?;
                    }
                }
                for diagnostic in &matches.diagnostics {
                    if diagnostic.kind == SectionDiagnosticKind::Unclaimed {
                        self.unclaimed(
                            Some(diagnostic.raw_node_id),
                            "parser.section.unclaimed",
                            "No registered syntax accepted this statement or section",
                            &diagnostic.span.mapped,
                        )?;
                        continue;
                    }
                    let (kind, message) = match diagnostic.kind {
                        SectionDiagnosticKind::ExtensionFault => (
                            "extension-fault",
                            "Extension execution failed; this region remains unresolved",
                        ),
                        SectionDiagnosticKind::Unclaimed => (
                            "unclaimed",
                            "No registered syntax accepted this statement or section",
                        ),
                        SectionDiagnosticKind::MultipleClaims => (
                            "multiple-claims",
                            "More than one syntax accepted this section",
                        ),
                    };
                    self.add(
                        &format!("parser.section.{kind}"),
                        message,
                        DiagnosticSeverity::ERROR,
                        native_origins(&diagnostic.span.mapped),
                        Vec::new(),
                    )?;
                }
                if let Some(selected) = &matches.selected {
                    self.section_body(&selected.body)?;
                } else if let Some(unknown) = &matches.unknown {
                    self.section_body(&unknown.body)?;
                }
            }
        }
        Ok(())
    }
}

fn failure_reason(reason: &skript_parser::PatternFailureReason) -> String {
    use skript_parser::PatternFailureReason as Reason;
    match reason {
        Reason::Literal { expected } => format!("Expected {expected:?}"),
        Reason::Regex { .. } => "Input does not match the registered text pattern".into(),
        Reason::Expression => "Expected an expression".into(),
        Reason::TypeExpression { expected } => {
            format!("Expected expression type: {}", expected.join(", "))
        }
        Reason::TypeMismatch { expected, actual } => {
            format!(
                "Expected {}, found {}",
                expected.join(" or "),
                actual.join(" or ")
            )
        }
        Reason::DefaultExpression { reason, .. }
        | Reason::TypeParserUnresolved { reason, .. }
        | Reason::HookRejected { reason } => reason.clone(),
        Reason::EventRestricted { supported, current } => format!(
            "Available in events [{}], current events [{}]",
            supported.join(", "),
            current.join(", ")
        ),
        Reason::TrailingInput => "Unexpected trailing input".into(),
    }
}

fn native_origins(span: &MappedSpan) -> Vec<TextRange> {
    span.origins
        .iter()
        .map(|origin| origin.original_range)
        .collect()
}
fn wit_origins(span: &host::MappedSpan) -> Result<Vec<TextRange>, String> {
    span.origins
        .iter()
        .map(|origin| {
            Ok(TextRange::new(
                usize::try_from(origin.original_range.start)
                    .map_err(|_| "source offset overflow")?,
                usize::try_from(origin.original_range.end).map_err(|_| "source offset overflow")?,
            ))
        })
        .collect()
}

pub(super) fn from_result(
    uri: &Uri,
    result: &DocumentParseResult,
) -> Result<Vec<Diagnostic>, String> {
    let mut collector = Collector::new(uri, result.source.original());
    for diagnostic in &result.raw_tree.diagnostics {
        collector.add(
            &format!("parser.raw-tree.{}", diagnostic.code.as_str()),
            &diagnostic.message,
            match diagnostic.severity {
                RawDiagnosticSeverity::Error => DiagnosticSeverity::ERROR,
                RawDiagnosticSeverity::Warning => DiagnosticSeverity::WARNING,
            },
            native_origins(&diagnostic.span),
            diagnostic
                .related
                .iter()
                .map(|related| (related.message.clone(), native_origins(&related.span)))
                .collect(),
        )?;
    }
    for diagnostic in &result.syntax.diagnostics {
        collector.structure(diagnostic)?;
    }
    for root in &result.syntax.roots {
        if let StructureDocumentNode::Structure(matches) = root {
            if let Some(trace) = matches
                .unknown
                .as_ref()
                .and_then(|unknown| unknown.failure.as_ref())
            {
                collector.trace(trace)?;
            }
            for diagnostic in &matches.diagnostics {
                collector.structure(diagnostic)?;
            }
            if let Some(selected) = &matches.selected {
                collector.body(&selected.body)?;
            } else if let Some(partial) = matches
                .unknown
                .as_ref()
                .and_then(|unknown| unknown.partial.as_ref())
            {
                collector.body(&partial.body)?;
            }
        }
    }
    for diagnostic in &result.effects.diagnostics {
        collector.hook(diagnostic)?;
    }
    // Rejection diagnostics belong to the decision, rather than HookEffects.
    // Keep them even though the macro's edits and speculative effects rolled back.
    for (stage, decision) in [
        ("text", &result.text_macro_decision),
        ("tree", &result.tree_macro_decision),
        ("AST", &result.ast_macro_decision),
    ] {
        collector.macro_decision(stage, decision)?;
    }
    for region in &result.fault_regions {
        collector.add(
            &region.fault.code,
            &format!("{}; this region remains unresolved", region.fault),
            DiagnosticSeverity::ERROR,
            wit_origins(&region.span)?,
            Vec::new(),
        )?;
    }
    // Even failures without a mapped affected span must remain visible.
    for failure in &result.component_failures {
        let origins = failure
            .affected_span
            .as_ref()
            .map(wit_origins)
            .transpose()?
            .unwrap_or_else(|| vec![TextRange::empty(0)]);
        collector.add(
            "parser.extension-fault",
            &format!(
                "Extension {}/{} failed: {}",
                failure.component_id, failure.subscription_id, failure.error
            ),
            DiagnosticSeverity::ERROR,
            origins,
            Vec::new(),
        )?;
    }
    let mut diagnostics = collector.finish();
    for region in &result.dependency_unavailable {
        let ranges = wit_origins(&region.span)?
            .into_iter()
            .map(|range| {
                LineIndex::new(result.source.original()).range(result.source.original(), range)
            })
            .collect::<Result<Vec<_>, _>>()?;
        diagnostics.retain(|diagnostic| {
            !matches!(&diagnostic.code, Some(NumberOrString::String(code)) if matches!(code.as_str(),
                "parser.structure.unclaimed" | "parser.section.unclaimed" | "parser.syntax-incomplete"))
                || !ranges.iter().any(|range| range.start <= diagnostic.range.start && diagnostic.range.end <= range.end)
        });
        for range in ranges {
            diagnostics.push(Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::INFORMATION),
                code: Some(NumberOrString::String(
                    "parser.function-namespace-incomplete".into(),
                )),
                source: Some("skript-lsp".into()),
                message: format!(
                    "Function {} remains unresolved because project discovery is incomplete",
                    region.name
                ),
                ..Default::default()
            });
        }
    }
    let status = if result.execution_status == parser_wasm::DocumentExecutionStatus::Partial {
        "partial"
    } else {
        "complete"
    };
    for diagnostic in &mut diagnostics {
        if let Some(serde_json::Value::Object(data)) = &mut diagnostic.data {
            data.insert("executionStatus".into(), json!(status));
        } else {
            diagnostic.data = Some(json!({"executionStatus":status}));
        }
    }
    Ok(diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::Position;

    fn failure(
        source: &str,
        range: TextRange,
        reasons: Vec<skript_parser::PatternFailureReason>,
    ) -> skript_parser::FailureTrace {
        skript_parser::FailureTrace::leaf(skript_parser::PatternFailure {
            span: skript_parser::MatchSpan {
                local_range: range,
                mapped: skript_parser::MappedSource::identity(source)
                    .map_range(range)
                    .unwrap(),
            },
            reasons,
        })
    }

    fn summary(collector: &mut Collector<'_>, node: Option<RawNodeId>, range: TextRange) {
        let span = skript_parser::MappedSource::identity(collector.source)
            .map_range(range)
            .unwrap();
        collector
            .unclaimed(
                node,
                "parser.structure.unclaimed",
                "Unresolved child",
                &span,
            )
            .unwrap();
    }

    #[test]
    fn type_mismatch_diagnostic_preserves_the_bad_expression_and_structured_types() {
        let source = "😀 teleport player to player's name\n";
        let uri = "file:///project/example.sk".parse().unwrap();
        let start = source.find("player's").unwrap();
        let range = TextRange::new(start, start + "player's name".len());
        let mut collector = Collector::new(&uri, source);
        collector
            .trace(&failure(
                source,
                range,
                vec![skript_parser::PatternFailureReason::TypeMismatch {
                    expected: vec!["org.bukkit.Location".into()],
                    actual: vec!["java.lang.String".into()],
                }],
            ))
            .unwrap();
        let diagnostics = collector.finish();
        assert_eq!(diagnostics.len(), 1);
        let diagnostic = &diagnostics[0];
        assert_eq!(
            diagnostic.code,
            Some(NumberOrString::String("parser.type-mismatch".into()))
        );
        assert_eq!(
            diagnostic.message,
            "Expected org.bukkit.Location, found java.lang.String"
        );
        assert_eq!(
            diagnostic.range,
            LineIndex::new(source).range(source, range).unwrap()
        );
        let wire = serde_json::to_value(diagnostic).unwrap();
        assert_eq!(
            wire["data"],
            json!({"expectedTypes": ["org.bukkit.Location"], "actualTypes": ["java.lang.String"]})
        );
        assert_eq!(wire["range"]["start"]["character"], json!(22));
    }

    #[test]
    fn from_result_keeps_native_type_evidence_with_execution_status() {
        // Teleport's bypassed direction capture needs its authentic initializer.
        // The old schema3 corpus has no DefaultExpression descriptors.
        let catalog = crate::lsp::test_support::runtime_catalog();
        let mut host = crate::new_parser_host(parser_wasm::HostConfig {
            syntax_catalog: Some(catalog),
            runtime_profile: parser_wasm::RuntimeProfile {
                skript_version: Some("2.15.4".into()),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
        let source: std::sync::Arc<str> =
            std::sync::Arc::from("on join:\n    teleport player to player's name\n");
        let result = host
            .parse_document(
                parser_wasm::DocumentParseRequest::new(
                    "file:///project",
                    "native-type-diagnostic",
                    1,
                    std::sync::Arc::clone(&source),
                ),
                Default::default(),
            )
            .unwrap();
        assert_eq!(
            result.execution_status,
            parser_wasm::DocumentExecutionStatus::Complete
        );
        assert!(
            result.component_failures.is_empty(),
            "{:?}",
            result.component_failures
        );
        assert!(
            result.fault_regions.is_empty(),
            "{:?}",
            result.fault_regions
        );
        let uri = "file:///project/native-type.sk".parse().unwrap();
        let diagnostics = from_result(&uri, &result).unwrap();
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.code == Some(NumberOrString::String("parser.type-mismatch".into()))
            })
            .unwrap_or_else(|| {
                panic!(
                    "real native typed mismatch missing: {diagnostics:#?}; native syntax: {:#?}",
                    result.syntax
                )
            });
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
        let start = source.find("player's name").unwrap();
        assert_eq!(
            diagnostic.range,
            LineIndex::new(&source)
                .range(
                    &source,
                    TextRange::new(start, start + "player's name".len())
                )
                .unwrap()
        );
        let wire = serde_json::to_value(diagnostic).unwrap();
        assert_eq!(
            wire["data"]["expectedTypes"],
            json!(["org.bukkit.Location"]),
            "wrong containing typed slot: {diagnostics:#?}; native syntax: {:#?}",
            result.syntax
        );
        // Actual PlayerClassInfo2.15.4 NAME uses Player::name, Component.class;
        // the synthetic native matcher fixtures intentionally resolve String.
        assert_eq!(
            wire["data"]["actualTypes"],
            json!(["net.kyori.adventure.text.Component"])
        );
        assert_eq!(wire["data"]["executionStatus"], json!("complete"));
    }

    #[test]
    fn real_type_diagnostics_distinguish_returns_parameters_unknown_and_incomplete_input() {
        let catalog = crate::lsp::test_support::runtime_catalog();
        let mut host = crate::new_parser_host(parser_wasm::HostConfig {
            syntax_catalog: Some(std::sync::Arc::clone(&catalog)),
            runtime_profile: parser_wasm::RuntimeProfile {
                skript_version: Some("2.15.4".into()),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
        let header = "function label() :: text:\r\n    return \"hello\"\r\nfunction numeric() :: number:\r\n    return 1\r\nfunction needs_number(amount: number) :: text:\r\n    return \"hello\"\r\non join:\r\n    ";
        let cases = [
            ("teleport player to location of player", None),
            (
                "teleport player to player's name",
                Some((
                    "player's name",
                    "org.bukkit.Location",
                    "net.kyori.adventure.text.Component",
                )),
            ),
            (
                "teleport player to \"here\"",
                Some(("\"here\"", "org.bukkit.Location", "java.lang.String")),
            ),
            (
                "teleport player to label()",
                Some(("label()", "org.bukkit.Location", "java.lang.String")),
            ),
            (
                "teleport player to (label())",
                Some(("label()", "org.bukkit.Location", "java.lang.String")),
            ),
            (
                "teleport label() to location of player",
                Some(("label()", "org.bukkit.entity.Entity", "java.lang.String")),
            ),
            (
                "set {_target} to \"here\"\r\n    teleport player to {_target}",
                Some(("{_target}", "org.bukkit.Location", "java.lang.String")),
            ),
            (
                "send needs_number(player)",
                Some(("player", "java.lang.Number", "org.bukkit.entity.Player")),
            ),
            (
                "send \"\u{1f98a}\" to numeric()",
                Some((
                    "numeric()",
                    "net.kyori.adventure.audience.Audience",
                    "java.lang.Number",
                )),
            ),
            (
                "teleport player to \"value %player%\"",
                Some((
                    "\"value %player%\"",
                    "org.bukkit.Location",
                    "java.lang.String",
                )),
            ),
            (
                "teleport player to \"value %not_a_valid_expression%\"",
                None,
            ),
            ("send \"message\" to label()", None),
            ("teleport player to {_unknown}", None),
            ("teleport player to", None),
            ("teleport player to label(", None),
        ];
        let uri = "file:///project/typed-diagnostics.sk".parse().unwrap();
        for (index, (body, expected)) in cases.into_iter().enumerate() {
            let source: std::sync::Arc<str> = format!("{header}{body}\r\n").into();
            let result = host
                .parse_document(
                    parser_wasm::DocumentParseRequest::new(
                        "file:///project",
                        "real-typed-diagnostics",
                        index as u64 + 1,
                        std::sync::Arc::clone(&source),
                    ),
                    Default::default(),
                )
                .unwrap();
            assert_eq!(
                result.execution_status,
                parser_wasm::DocumentExecutionStatus::Complete,
                "{body}"
            );
            assert!(
                result.component_failures.is_empty(),
                "{body}: {:?}",
                result.component_failures
            );
            assert!(
                result.fault_regions.is_empty(),
                "{body}: {:?}",
                result.fault_regions
            );
            let mut diagnostics = from_result(&uri, &result).unwrap();
            let semantic = super::super::semantic::Document::from_result(
                &result,
                std::sync::Arc::clone(&source),
                uri.clone(),
                std::sync::Arc::clone(&catalog),
            )
            .unwrap();
            diagnostics.extend(semantic.diagnostics(&result).unwrap());
            let typed=diagnostics.iter().filter(|diagnostic|matches!(&diagnostic.code,
                Some(NumberOrString::String(code)) if code=="parser.type-mismatch" || code=="semantic.variable-type-mismatch")).collect::<Vec<_>>();
            if let Some((bad, expected, actual)) = expected {
                assert_eq!(typed.len(), 1, "{body}: {diagnostics:#?}");
                let diagnostic = typed[0];
                let start = header.len() + body.rfind(bad).unwrap();
                assert_eq!(
                    diagnostic.range,
                    LineIndex::new(&source)
                        .range(&source, TextRange::new(start, start + bad.len()))
                        .unwrap(),
                    "{body}"
                );
                let data = diagnostic.data.as_ref().unwrap();
                assert_eq!(
                    data["expectedTypes"],
                    json!([expected]),
                    "{body}: {diagnostics:#?}"
                );
                assert_eq!(
                    data["actualTypes"],
                    json!([actual]),
                    "{body}: {diagnostics:#?}"
                );
            } else {
                assert!(typed.is_empty(), "{body}: {diagnostics:#?}");
                if !body.contains("not_a_valid_expression")
                    && !body.ends_with(" to")
                    && !body.ends_with("label(")
                {
                    assert!(diagnostics.is_empty(), "{body}: {diagnostics:#?}");
                }
            }
        }
        // A rejected intrinsic must not widen a restricted parse mode, replace
        // a compatible union, or retain nested parse receipts/context effects.
        use skript_parser::{
            ExpressionExpectedType, ExpressionParseRequest, ExpressionParserConfig,
            ExpressionRootMode as Mode, MappedSource,
        };
        let document = "file:///project/diagnostic-mode.sk";
        let transaction = host.begin_parse("file:///project", document, 900).unwrap();
        let modes = [
            (
                "\"here\"",
                vec!["org.bukkit.Location"],
                Mode::All,
                true,
                false,
            ),
            (
                "\"here\"",
                vec!["org.bukkit.Location"],
                Mode::ExpressionsOnly,
                false,
                false,
            ),
            (
                "\"here\"",
                vec!["org.bukkit.Location"],
                Mode::LiteralsOnly,
                false,
                false,
            ),
            ("\"here\"", vec!["java.lang.Object"], Mode::All, false, true),
            (
                "\"here\"",
                vec!["org.bukkit.Location", "java.lang.String"],
                Mode::All,
                false,
                true,
            ),
            (
                "\"here\"",
                vec!["fixture.UnknownType"],
                Mode::All,
                false,
                false,
            ),
            (
                "\"value %player%\"",
                vec!["org.bukkit.Location"],
                Mode::All,
                true,
                false,
            ),
            (
                "\"value %not_a_valid_expression%\"",
                vec!["org.bukkit.Location"],
                Mode::All,
                false,
                false,
            ),
        ];
        for (index, (text, expected, mode, mismatch, accepted)) in modes.into_iter().enumerate() {
            let root_expected = expected
                .iter()
                .map(|class| (*class).to_owned())
                .collect::<Vec<_>>();
            let source = MappedSource::identity(text);
            let result = host
                .parse_expression_in_parse(
                    &transaction,
                    parser_wasm::host::InvocationContext {
                        invocation_id: index as u64 + 1,
                        subscription_id: String::new(),
                        document_id: document.into(),
                        document_revision: 900,
                        expansion: None,
                        syntax_context: 0,
                    },
                    ExpressionParseRequest {
                        source: &source,
                        range: TextRange::new(0, text.len()),
                        expected_types: expected
                            .into_iter()
                            .map(|class| ExpressionExpectedType {
                                class_name: syntaxes::ClassName(class.into()),
                                plural: false,
                            })
                            .collect(),
                        context: skript_parser::ExpressionParseContext {
                            event_classes: vec![syntaxes::ClassName(
                                "org.bukkit.event.player.PlayerJoinEvent".into(),
                            )],
                            ..Default::default()
                        },
                    },
                    ExpressionParserConfig {
                        root_mode: mode,
                        ..Default::default()
                    },
                )
                .unwrap();
            assert!(result.failures.is_empty(), "{text}: {:?}", result.failures);
            assert_eq!(
                result.matches.selected.is_some(),
                accepted,
                "{text} {mode:?}: {result:#?}"
            );
            let found = result
                .matches
                .failure
                .as_ref()
                .and_then(|failure| failure.trace.as_ref())
                .is_some_and(|trace| {
                    let mut current=trace;
                    loop {
                        if current.frame.as_ref().is_some_and(|frame|frame.role!=skript_parser::FailureFrameRole::SemanticCandidate){return false;}
                        let Some(cause)=current.cause.as_deref()else{break;};current=cause;
                    }
                    trace.root_cause().failure.reasons.iter().any(|reason| {
                        matches!(
                            reason,
                            skript_parser::PatternFailureReason::TypeMismatch {expected,actual}
                                if expected==&root_expected && actual==&vec!["java.lang.String".to_owned()]
                        )
                    })
                });
            assert_eq!(found, mismatch, "{text} {mode:?}: {result:#?}");
            if !accepted {
                assert!(result.effects.parse_results.is_empty(), "{text}");
                assert!(result.effects.parse_requests.is_empty(), "{text}");
                assert!(result.effects.context_updates.is_empty(), "{text}");
            }
        }
        transaction.cancel().unwrap();
    }

    #[test]
    fn a_specific_child_error_replaces_its_summary_at_equal_or_different_positions() {
        let source = "on load:\n    broken statement\n";
        let uri = "file:///project/example.sk".parse().unwrap();
        for cause_range in [TextRange::new(13, 29), TextRange::new(20, 29)] {
            let mut collector = Collector::new(&uri, source);
            summary(
                &mut collector,
                Some(RawNodeId::new(1)),
                TextRange::new(13, 29),
            );
            collector
                .body_trace(
                    RawNodeId::new(1),
                    &failure(
                        source,
                        cause_range,
                        vec![skript_parser::PatternFailureReason::TrailingInput],
                    ),
                )
                .unwrap();
            let diagnostics = collector.finish();
            assert_eq!(diagnostics.len(), 1);
            assert_eq!(diagnostics[0].message, "Unexpected trailing input");
            assert_eq!(
                diagnostics[0].range,
                LineIndex::new(source).range(source, cause_range).unwrap()
            );
        }
    }

    #[test]
    fn child_ownership_preserves_overlapping_other_nodes_and_cause_free_siblings() {
        let source = "0123456789\nsecond child\n";
        let uri = "file:///project/example.sk".parse().unwrap();
        let mut collector = Collector::new(&uri, source);
        for node in [Some(RawNodeId::new(1)), Some(RawNodeId::new(2)), None] {
            summary(&mut collector, node, TextRange::new(0, 10));
        }
        summary(
            &mut collector,
            Some(RawNodeId::new(3)),
            TextRange::new(11, 23),
        );
        collector
            .body_trace(
                RawNodeId::new(1),
                &failure(
                    source,
                    TextRange::new(0, 10),
                    vec![skript_parser::PatternFailureReason::TrailingInput],
                ),
            )
            .unwrap();
        // A root/header trace is intentionally not a body cause.
        collector
            .trace(&failure(
                source,
                TextRange::new(11, 23),
                vec![skript_parser::PatternFailureReason::Expression],
            ))
            .unwrap();
        let diagnostics = collector.finish();
        assert_eq!(
            diagnostics.len(),
            4,
            "the equal-position summaries deduplicate only after each owner's cause is checked"
        );
        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.message == "Unresolved child")
                .count(),
            2
        );
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message == "Expected an expression")
        );
    }

    #[test]
    fn generic_or_warning_only_traces_do_not_erase_the_unresolved_error() {
        let source = "broken";
        let uri = "file:///project/example.sk".parse().unwrap();
        for severity in [
            None,
            Some(skript_parser::SemanticDiagnosticSeverity::Warning),
        ] {
            let mut collector = Collector::new(&uri, source);
            summary(
                &mut collector,
                Some(RawNodeId::new(1)),
                TextRange::new(0, 6),
            );
            let mut trace = failure(source, TextRange::new(0, 6), Vec::new());
            if let Some(severity) = severity {
                trace
                    .semantic_diagnostics
                    .push(skript_parser::SemanticDiagnostic {
                        code: "fixture.warning".into(),
                        message: "Additional context".into(),
                        severity,
                        span: trace.failure.span.mapped.clone(),
                        related: Vec::new(),
                    });
            }
            collector.body_trace(RawNodeId::new(1), &trace).unwrap();
            let diagnostics = collector.finish();
            assert_eq!(diagnostics.len(), 2);
            assert_eq!(diagnostics[0].message, "Unresolved child");
            assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
        }
    }

    #[test]
    fn a_semantic_error_equal_to_a_summary_survives_summary_removal() {
        let source = "broken";
        let uri = "file:///project/example.sk".parse().unwrap();
        let mut collector = Collector::new(&uri, source);
        summary(
            &mut collector,
            Some(RawNodeId::new(1)),
            TextRange::new(0, 6),
        );
        let mut trace = failure(source, TextRange::new(0, 6), Vec::new());
        trace
            .semantic_diagnostics
            .push(skript_parser::SemanticDiagnostic {
                code: "parser.structure.unclaimed".into(),
                message: "Unresolved child".into(),
                severity: skript_parser::SemanticDiagnosticSeverity::Error,
                span: trace.failure.span.mapped.clone(),
                related: Vec::new(),
            });
        collector.body_trace(RawNodeId::new(1), &trace).unwrap();
        let diagnostics = collector.finish();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message, "Unresolved child");
        assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn a_macro_rejection_without_diagnostics_keeps_its_reason() {
        let uri = "file:///project/example.sk".parse().unwrap();
        let mut collector = Collector::new(&uri, "日本😀");
        collector
            .macro_decision(
                "AST",
                &host::HookDecision::Reject(host::Rejection {
                    reason: "rejected by the extension".into(),
                    diagnostics: Vec::new(),
                }),
            )
            .unwrap();
        assert_eq!(collector.diagnostics.len(), 1);
        let diagnostic = &collector.diagnostics[0];
        assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(
            diagnostic.range,
            Range::new(Position::new(0, 0), Position::new(0, 4))
        );
        assert!(diagnostic.message.contains("AST"));
        assert!(diagnostic.message.contains("rejected by the extension"));
    }
    #[test]
    fn original_positions_cover_utf16_crlf_empty_lines_and_eof() {
        let source = "日本😀x\r\n\r\n終";
        let index = LineIndex::new(source);
        assert_eq!(
            index.position(source, "日本😀".len()).unwrap(),
            Position::new(0, 4)
        );
        assert_eq!(
            index.position(source, "日本😀x\r".len()).unwrap(),
            Position::new(0, 5)
        );
        assert_eq!(
            index.position(source, "日本😀x\r\n".len()).unwrap(),
            Position::new(1, 0)
        );
        assert_eq!(
            index.position(source, source.len()).unwrap(),
            Position::new(2, 1)
        );
        assert!(index.position(source, 1).is_err());
        assert!(index.position(source, source.len() + 1).is_err());
        assert_eq!(
            LineIndex::new("").position("", 0).unwrap(),
            Position::new(0, 0)
        );
    }
    #[test]
    fn macro_origins_use_original_ranges_and_preserve_other_locations() {
        let uri = "file:///project/example.sk".parse().unwrap();
        let mut collector = Collector::new(&uri, "日本😀\n定義");
        collector
            .add(
                "fixture",
                "expanded failure",
                DiagnosticSeverity::ERROR,
                vec![TextRange::new(6, 10), TextRange::new(11, 17)],
                Vec::new(),
            )
            .unwrap();
        let diagnostic = &collector.diagnostics[0];
        assert_eq!(
            diagnostic.range,
            Range::new(Position::new(0, 2), Position::new(0, 4))
        );
        assert_eq!(
            diagnostic.related_information.as_ref().unwrap()[0]
                .location
                .range,
            Range::new(Position::new(1, 0), Position::new(1, 2))
        );
        assert!(
            collector
                .add(
                    "fixture",
                    "bad",
                    DiagnosticSeverity::ERROR,
                    vec![],
                    Vec::new()
                )
                .is_err()
        );
    }
}

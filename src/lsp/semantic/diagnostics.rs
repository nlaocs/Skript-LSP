//! Post-parse feedback from proven local-variable flow facts. These warnings
//! describe the accepted AST; they do not change native parser acceptance.
use super::*;
use lsp_types::{Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, NumberOrString};
use serde_json::json;

impl Document {
    pub fn diagnostics(&self, result: &DocumentParseResult) -> Result<Vec<Diagnostic>, String> {
        let blocked = result
            .unresolved_spans()
            .flat_map(|span| span.origins.iter())
            .map(|origin| {
                Ok(TextRange::new(
                    usize::try_from(origin.original_range.start)
                        .map_err(|_| "semantic fault range overflow")?,
                    usize::try_from(origin.original_range.end)
                        .map_err(|_| "semantic fault range overflow")?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let witnesses = result.semantic_witnesses();
        self.inferred_diagnostics(
            &result.ast,
            result.source.original(),
            &blocked,
            |node| witnesses.native_valid(node),
            |node| {
                witnesses
                    .unique_issuer(node)
                    .is_some_and(|issuer| witnesses.provider_valid(node, issuer))
            },
        )
    }

    fn inferred_diagnostics(
        &self,
        ast: &AstTree,
        source: &str,
        blocked: &[TextRange],
        native_valid: impl Fn(&AstNode) -> bool,
        identity_valid: impl Fn(&AstNode) -> bool,
    ) -> Result<Vec<Diagnostic>, String> {
        if source != self.source.as_ref() {
            return Err("inferred diagnostics source differs from semantic revision".into());
        }
        let nodes = ast
            .nodes
            .iter()
            .map(|node| (node.id, node))
            .collect::<HashMap<_, _>>();
        if nodes.len() != ast.nodes.len() {
            return Err("duplicate inferred-diagnostic AST node ID".into());
        }
        let mut seen = HashSet::new();
        let mut pending = ast.roots.clone();
        let mut diagnostics = Vec::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let node = nodes
                .get(&id)
                .ok_or("missing inferred-diagnostic AST node")?;
            pending.extend(node.children.iter().copied());
            if !native_valid(node)
                || !source_geometry::pattern_capture_geometry_is_current(node)
                || !current(node, source, blocked)
            {
                continue;
            }
            for capture in &node.captures {
                let Some(evidence) = source_geometry::pattern_expected_types(node, capture) else {
                    continue;
                };
                let index = evidence.capture_index;
                let codes = evidence.type_codes;
                let Some(expected) = codes
                    .iter()
                    .map(|code| {
                        let ty = self.catalog.type_by_code_name(code)?;
                        self.catalog.class(ty.original_class.as_str())?;
                        Some(ty.original_class.as_str().to_owned())
                    })
                    .collect::<Option<BTreeSet<_>>>()
                else {
                    continue;
                };
                let Some(child_id) = capture_node(node, index, &nodes) else {
                    continue;
                };
                let child = nodes[&child_id];
                if !identity_valid(child)
                    || child.syntax_context != node.syntax_context
                    || !current(child, source, blocked)
                {
                    continue;
                }
                let Some(child_range) = exact_span(&child.span) else {
                    continue;
                };
                let native = format!("{}{index}", source_geometry::PATTERN_CAPTURE_PREFIX);
                let Some(slot) = node
                    .captures
                    .iter()
                    .find(|capture| capture.name == native)
                    .and_then(|capture| match &capture.value {
                        CaptureValue::Span(span) => exact_span(span),
                        _ => None,
                    })
                else {
                    continue;
                };
                if !slot.contains(child_range) {
                    continue;
                }
                let Some(summary) = &child.summary else {
                    continue;
                };
                if summary.kind != "variable" {
                    continue;
                }
                let mut identities = summary
                    .public_data
                    .iter()
                    .filter(|record| record.schema_id == IDENTITY);
                let Some(record) = identities
                    .next()
                    .filter(|record| record.schema_version == 1)
                else {
                    continue;
                };
                if identities.next().is_some() {
                    continue;
                }
                let Ok(identity) = serde_json::from_str::<IdentityData>(&record.json) else {
                    continue;
                };
                if identity.resolution != "static" || identity.scope != "local" {
                    continue;
                }
                let mut occurrences = self.occurrences.iter().filter(|occurrence| {
                    occurrence.range == child_range && matches!(&occurrence.reference, Reference::Variable { identity: candidate, .. }
                        if candidate.key.local && candidate.key.context == child.syntax_context && candidate.key.name == identity.name)
                });
                let Some(occurrence) = occurrences.next() else {
                    continue;
                };
                if occurrences.next().is_some() {
                    continue;
                }
                let Some((classes, _)) = occurrence.ty.proven_present() else {
                    continue;
                };
                if classes.is_empty()
                    || classes.iter().any(|actual| {
                        actual == "java.lang.Object"
                            || self.catalog.class(actual).is_none()
                            || expected
                                .iter()
                                .any(|expected| self.catalog.can_convert(actual, expected))
                    })
                {
                    continue;
                }
                let expected = expected.into_iter().collect::<Vec<_>>();
                let actual = classes.iter().cloned().collect::<Vec<_>>();
                let Reference::Variable { reaching, .. } = &occurrence.reference else {
                    unreachable!();
                };
                let related = reaching
                    .iter()
                    .filter(|range| !blocked.iter().any(|blocked| overlaps(**range, *blocked)))
                    .map(|range| {
                        Ok(DiagnosticRelatedInformation {
                            location: Location::new(
                                self.uri.clone(),
                                self.lines.range(&self.source, *range)?,
                            ),
                            message: "Variable type established by this write".into(),
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let diagnostic = Diagnostic {
                    range: self.lines.range(&self.source, child_range)?,
                    severity: Some(DiagnosticSeverity::WARNING),
                    code: Some(NumberOrString::String(
                        "semantic.variable-type-mismatch".into(),
                    )),
                    source: Some("skript-lsp".into()),
                    message: format!(
                        "Expected {}, found inferred variable type {}. The parser accepted this reference, but none of its proven reaching types can be converted to an expected type.",
                        expected.join(" or "),
                        actual.join(" or ")
                    ),
                    related_information: (!related.is_empty()).then_some(related),
                    data: Some(
                        json!({"expectedTypes": expected,"actualTypes": actual,"parserAccepted": true}),
                    ),
                    ..Diagnostic::default()
                };
                if !diagnostics.contains(&diagnostic) {
                    diagnostics.push(diagnostic);
                }
            }
        }
        Ok(diagnostics)
    }
}

fn current(node: &AstNode, source: &str, blocked: &[TextRange]) -> bool {
    let Some(range) = exact_span(&node.span) else {
        return false;
    };
    source.get(range.start..range.end) == Some(node.text.as_str())
        && !blocked.iter().any(|blocked| overlaps(*blocked, range))
        && !node
            .metadata
            .iter()
            .any(|entry| entry.key == "parser.editor-recovery")
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;

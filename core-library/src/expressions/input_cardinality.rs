//! Applies only positive, conditional SSG input-dependency evidence.
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ParseResultStatus, RegisteredExpressionPayload, RegisteredSyntaxHandler,
    RegisteredSyntaxHandlerTarget, SyntaxKind,
};
use serde::Deserialize;

pub(crate) const SOURCE_KEY: &str = parser_wasm::REGISTERED_METADATA_MULTIPLICITY_SOURCE;
const HANDLER_ID: &str = "core.expression.input-cardinality";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    // Receiving a property family does not imply a source dependency: multiplicity()
    // below requires the host's positive, conditional SSG evidence for that registration.
    let targets = [
        "ch.njol.skript.expressions.base.PropertyExpression",
        "ch.njol.skript.expressions.base.SimplePropertyExpression",
    ]
    .into_iter()
    .map(|base| RegisteredSyntaxHandlerTarget::SuperClass(base.to_owned()))
    .collect();
    handlers.push(RegisteredSyntaxHandler {
        handler_id: HANDLER_ID.to_owned(),
        kind: SyntaxKind::Expression,
        phase: crate::nlaocs::skript_parser_addon::types::HookPhase::Expression,
        targets,
        pattern_indices: Vec::new(),
        pattern_sources: Vec::new(),
        required_tags: Vec::new(),
        forbidden_tags: Vec::new(),
        marks: Vec::new(),
        capture_parsers: Vec::new(),
        context_requirements: Vec::new(),
    });
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Source {
    version: u32,
    capture_index: u64,
    fully_parsed_input: bool,
}

pub(crate) fn multiplicity(payload: &RegisteredExpressionPayload) -> Option<DynamicMultiplicity> {
    let value = payload
        .metadata
        .iter()
        .find(|entry| entry.key == SOURCE_KEY && entry.owner_component_id.is_none())?;
    let source: Source = serde_json::from_str(&value.value).ok()?;
    if source.version != 1 || !source.fully_parsed_input {
        return None;
    }
    let capture = payload
        .parsed_captures
        .iter()
        .find(|capture| capture.capture_index == source.capture_index)?;
    if capture.status != ParseResultStatus::Success {
        return None;
    }
    let summary = capture.summary.as_ref()?;
    if summary.default_expression.is_some() {
        return None;
    }
    summary.multiplicity
}

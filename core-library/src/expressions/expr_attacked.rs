use super::{
    SemanticResolution, matches, metadata, metadata_value, register_handler_with_context,
    resolved_with_possible_types,
};
use crate::nlaocs::skript_parser_addon::types::{
    CaptureParserBinding, DynamicMultiplicity, ExpressionPossibleReturnTypesState, MetadataEntry,
    ParseResultStatus, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprAttacked";
const HANDLER_ID: &str = "core.expression.expr-attacked";
const ENTITY: &str = "org.bukkit.entity.Entity";
const ENTITY_DATA: &str = "ch.njol.skript.entity.EntityData";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler_with_context(
        handlers,
        HANDLER_ID,
        CLASS_SUFFIX,
        vec![entity_data_binding()],
        Vec::new(),
    );
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_attacked(payload))
}

fn resolve_attacked(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let parsed = payload.parsed_captures.iter().find(|capture| {
        capture.capture_index == 0
            && capture.parser_id == "host.expression"
            && capture.status == ParseResultStatus::Success
    });
    let return_type = if payload.regex_captures.is_empty() {
        ENTITY.to_owned()
    } else {
        let Some(summary) = parsed.and_then(|capture| capture.summary.as_ref()) else {
            return SemanticResolution::Reject(
                "attacked Expression contains an invalid entity type".to_owned(),
            );
        };
        let Some(entity_class) = metadata_value(&summary.metadata, "entity-class") else {
            return SemanticResolution::Unresolved {
                reason:
                    "attacked entity type was parsed, but its represented Bukkit class is unknown"
                        .to_owned(),
                metadata: vec![metadata("semantic-mode", "attacked-entity")],
            };
        };
        entity_class.to_owned()
    };
    resolved_with_possible_types(
        return_type.clone(),
        vec![return_type],
        ExpressionPossibleReturnTypesState::Complete,
        DynamicMultiplicity::Single,
        vec![metadata("semantic-mode", "attacked-entity")],
    )
}

fn entity_data_binding() -> CaptureParserBinding {
    CaptureParserBinding {
        capture_index: 0,
        parser_id: "host.expression".to_owned(),
        required: true,
        options: vec![entry("expression.expected-types", ENTITY_DATA)],
    }
}

fn entry(key: &str, value: &str) -> MetadataEntry {
    MetadataEntry {
        key: key.to_owned(),
        value: value.to_owned(),
        owner_component_id: None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn entity_capture_uses_the_registered_entity_data_parser() {
        let binding = super::entity_data_binding();
        assert_eq!(binding.capture_index, 0);
        assert!(binding.options.iter().any(|entry| {
            entry.key == "expression.expected-types"
                && entry.value == "ch.njol.skript.entity.EntityData"
        }));
    }
}

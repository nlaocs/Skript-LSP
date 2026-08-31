use super::{SemanticResolution, matches, metadata, metadata_value, register_handler_with_context};
use crate::nlaocs::skript_parser_addon::types::{
    CaptureParserBinding, DynamicMultiplicity, MetadataEntry, ParseResultStatus,
    RegisteredExpressionPayload, RegisteredSyntaxHandler,
};
use parser_wasm::REGISTERED_CONTEXT_ALL_TYPE_OPTIONS;

const CLASS_SUFFIX: &str = ".ExprEntity";
const HANDLER_ID: &str = "core.expression.expr-entity";
const ENTITY_DATA: &str = "ch.njol.skript.entity.EntityData";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler_with_context(
        handlers,
        HANDLER_ID,
        CLASS_SUFFIX,
        vec![CaptureParserBinding {
            capture_index: 0,
            parser_id: "host.expression".to_owned(),
            required: true,
            options: vec![entry("expression.expected-types", ENTITY_DATA)],
        }],
        vec![REGISTERED_CONTEXT_ALL_TYPE_OPTIONS.to_owned()],
    );
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_entity(payload))
}

fn resolve_entity(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let Some(summary) = payload
        .parsed_captures
        .iter()
        .find(|capture| {
            capture.capture_index == 0
                && capture.parser_id == "host.expression"
                && capture.status == ParseResultStatus::Success
        })
        .and_then(|capture| capture.summary.as_ref())
    else {
        return SemanticResolution::Reject(
            "event entity Expression contains an invalid entity type".to_owned(),
        );
    };
    if metadata_value(&summary.metadata, "entity-plural") == Some("true") {
        return SemanticResolution::Reject(
            "event entity Expression requires a singular entity type".to_owned(),
        );
    }
    let Some(return_type) = metadata_value(&summary.metadata, "entity-class") else {
        return SemanticResolution::Reject(
            "event entity Expression requires a statically parsed entity type".to_owned(),
        );
    };
    with_entity_metadata(
        super::event_value_expression::resolve_target(
            payload,
            return_type,
            Some(DynamicMultiplicity::Single),
        ),
        return_type,
    )
}

fn with_entity_metadata(resolution: SemanticResolution, return_type: &str) -> SemanticResolution {
    let values = [
        metadata("semantic-mode", "event-entity"),
        metadata("entity-class", return_type),
    ];
    match resolution {
        SemanticResolution::Resolved {
            return_type,
            possible_return_types,
            possible_return_types_state,
            multiplicity,
            mut metadata,
        } => {
            metadata.extend(values);
            SemanticResolution::Resolved {
                return_type,
                possible_return_types,
                possible_return_types_state,
                multiplicity,
                metadata,
            }
        }
        SemanticResolution::Unresolved {
            reason,
            mut metadata,
        } => {
            metadata.extend(values);
            SemanticResolution::Unresolved { reason, metadata }
        }
        rejected @ SemanticResolution::Reject(_) => rejected,
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
    use super::{ENTITY_DATA, register};

    #[test]
    fn entity_name_is_reparsed_as_entity_data() {
        let mut handlers = Vec::new();
        register(&mut handlers);
        assert!(handlers[0].capture_parsers.iter().any(|binding| {
            binding.parser_id == "host.expression"
                && binding.options.iter().any(|entry| {
                    entry.key == "expression.expected-types" && entry.value == ENTITY_DATA
                })
        }));
    }
}

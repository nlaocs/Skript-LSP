use super::{SemanticResolution, matches, metadata, metadata_value, property, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".PropExprValueOf";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, CLASS_SUFFIX).then(|| {
        if payload.property_options.is_empty() {
            return SemanticResolution::Reject(
                "source type has no registered typed value property".to_owned(),
            );
        }
        if let Some(target) = payload
            .children
            .iter()
            .find_map(|child| metadata_value(&child.metadata, "target-class"))
        {
            return SemanticResolution::Resolved {
                return_type: target.to_owned(),
                multiplicity: property::source_multiplicity(payload),
                metadata: vec![metadata("semantic-mode", "typed-value-target")],
            };
        }
        property::resolve(payload, "typed-value-property")
    })
}

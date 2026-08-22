use super::{SemanticResolution, matches, metadata, metadata_value, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprEntities";
const HANDLER_ID: &str = "core.expression.expr-entities";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        let Some(entity_data) = payload.children.first() else {
            return SemanticResolution::Reject(
                "entities Expression requires an entity data literal".to_owned(),
            );
        };
        let plural = metadata_value(&entity_data.metadata, "entity-plural")
            .or_else(|| metadata_value(&entity_data.metadata, "literal-plural"));
        if plural != Some("true") {
            return SemanticResolution::Reject(
                "entities Expression requires a plural entity data literal".to_owned(),
            );
        }
        let Some(return_type) = metadata_value(&entity_data.metadata, "entity-class")
            .or_else(|| metadata_value(&entity_data.metadata, "literal-represented-class"))
        else {
            return SemanticResolution::Reject(
                "entity data literal has no runtime entity class".to_owned(),
            );
        };
        SemanticResolution::Resolved {
            return_type: return_type.to_owned(),
            multiplicity: DynamicMultiplicity::Multiple,
            metadata: vec![metadata("semantic-mode", "entities-literal-type")],
        }
    })
}

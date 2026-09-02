use super::{SemanticResolution, matches, metadata, register_handler, resolved_with_metadata};
use crate::nlaocs::skript_parser_addon::types::{
    RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprLength";
const HANDLER_ID: &str = "core.expression.expr-length";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        let Some(source) = payload.children.first() else {
            return SemanticResolution::Reject(
                "length Expression requires a source Expression".to_owned(),
            );
        };
        let Some(multiplicity) = source.multiplicity else {
            return SemanticResolution::Unresolved {
                reason: "length Expression source multiplicity is unresolved".to_owned(),
                metadata: vec![metadata("semantic-mode", "length")],
            };
        };
        let Some(return_type) = payload.declared_return_type.as_ref() else {
            return SemanticResolution::Unresolved {
                reason: "length Expression return type is unresolved".to_owned(),
                metadata: vec![metadata("semantic-mode", "length")],
            };
        };
        resolved_with_metadata(
            return_type.clone(),
            multiplicity,
            vec![metadata("semantic-mode", "length")],
        )
    })
}

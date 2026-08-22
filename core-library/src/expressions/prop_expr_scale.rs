use super::{SemanticResolution, matches, property, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".PropExprScale";
const HANDLER_ID: &str = "core.expression.prop-expr-scale";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| property::resolve(payload, "scale-property"))
}

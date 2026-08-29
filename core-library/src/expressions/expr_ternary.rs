use super::{SemanticResolution, capture_parser, matches, register_handler, resolved};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ParseResultStatus, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprTernary";
const HANDLER_ID: &str = "core.expression.expr-ternary";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(
        handlers,
        HANDLER_ID,
        CLASS_SUFFIX,
        vec![capture_parser(1, "host.condition")],
    );
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        if !payload.parsed_captures.iter().any(|capture| {
            capture.capture_index == 1
                && capture.parser_id == "host.condition"
                && capture.status == ParseResultStatus::Success
        }) {
            return SemanticResolution::Reject(
                "ternary Expression requires one parsed Condition".to_owned(),
            );
        }
        if payload.children.len() != 2 {
            return SemanticResolution::Reject(
                "ternary Expression requires two result Expressions".to_owned(),
            );
        }
        let Some(return_type) = payload.common_child_return_type.as_deref() else {
            return SemanticResolution::Reject(
                "ternary result Expressions have no common return type".to_owned(),
            );
        };
        resolved(
            return_type,
            if payload
                .children
                .iter()
                .all(|child| matches!(child.multiplicity, Some(DynamicMultiplicity::Single)))
            {
                DynamicMultiplicity::Single
            } else {
                DynamicMultiplicity::Multiple
            },
            "ternary-condition",
        )
    })
}

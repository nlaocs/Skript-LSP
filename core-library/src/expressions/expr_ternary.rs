use super::{SemanticResolution, matches, register_handler, resolved};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredCaptureKind, RegisteredExpressionPayload,
    RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprTernary";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(
        handlers,
        CLASS_SUFFIX,
        vec![RegisteredCaptureKind::Condition],
    );
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, CLASS_SUFFIX).then(|| {
        if payload.conditions.len() != 1 || payload.conditions[0].capture_index != 0 {
            return SemanticResolution::Reject(
                "ternary Expression requires one parsed Condition".to_owned(),
            );
        }
        if payload.children.len() != 2 {
            return SemanticResolution::Reject(
                "ternary Expression requires two result Expressions".to_owned(),
            );
        }
        if payload.children.iter().any(|child| {
            child
                .element_class
                .as_deref()
                .is_some_and(|class| class.ends_with(CLASS_SUFFIX))
        }) {
            return SemanticResolution::Reject("ternary Expressions may not be nested".to_owned());
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

use super::{SemanticResolution, matches, register_handler, resolved};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredCaptureKind, RegisteredExpressionPayload,
    RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprWhether";

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
                "whether Expression requires one parsed Condition".to_owned(),
            );
        }
        resolved(
            "java.lang.Boolean",
            DynamicMultiplicity::Single,
            "whether-condition",
        )
    })
}

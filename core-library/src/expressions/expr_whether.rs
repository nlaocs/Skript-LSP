use super::{SemanticResolution, capture_parser, matches, register_handler, resolved};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ParseResultStatus, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprWhether";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(
        handlers,
        CLASS_SUFFIX,
        vec![capture_parser(0, "host.condition")],
    );
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, CLASS_SUFFIX).then(|| {
        if !payload.parsed_captures.iter().any(|capture| {
            capture.capture_index == 0
                && capture.parser_id == "host.condition"
                && capture.status == ParseResultStatus::Success
        }) {
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

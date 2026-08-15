use super::{SemanticResolution, matches, metadata, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprElement";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, CLASS_SUFFIX).then(|| {
        let Some(source_type) = payload
            .children
            .last()
            .and_then(|child| child.return_type.as_deref())
        else {
            return SemanticResolution::Reject(
                "element Expression requires a typed source Expression".to_owned(),
            );
        };
        let queue = source_type == "org.skriptlang.skript.lang.util.SkriptQueue";
        let multiple = payload.pattern.contains("%integer% elements")
            || payload.pattern.contains("random [:distinct] elements")
            || payload.pattern.contains("elements (from|between)");

        SemanticResolution::Resolved {
            return_type: if queue {
                "java.lang.Object".to_owned()
            } else {
                source_type.to_owned()
            },
            multiplicity: if multiple {
                DynamicMultiplicity::Multiple
            } else {
                DynamicMultiplicity::Single
            },
            metadata: vec![metadata("semantic-mode", "element-selection")],
        }
    })
}

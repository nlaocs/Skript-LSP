use super::{SemanticResolution, matches, metadata, metadata_value, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprSets";
const HANDLER_ID: &str = "core.expression.expr-sets";
const CLASS_INFO: &str = "ch.njol.skript.classes.ClassInfo";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        let Some(class_info) = payload
            .children
            .iter()
            .find(|child| child.return_type.as_deref() == Some(CLASS_INFO))
        else {
            return SemanticResolution::Reject(
                "sets Expression requires a ClassInfo child".to_owned(),
            );
        };

        let start = usize::try_from(payload.span.virtual_range.start).ok();
        let end = usize::try_from(payload.span.virtual_range.end).ok();
        let source = start
            .zip(end)
            .and_then(|(start, end)| payload.input.get(start..end));
        let plural = metadata_value(&class_info.metadata, "type-plural") == Some("true");
        if !plural && !source.is_some_and(|source| source.starts_with("every")) {
            return SemanticResolution::Reject(
                "sets Expression requires a plural ClassInfo unless the source starts with every"
                    .to_owned(),
            );
        }
        if metadata_value(&class_info.metadata, "has-supplier") != Some("true") {
            return SemanticResolution::Reject(
                "ClassInfo child has no supplier for sets Expression".to_owned(),
            );
        }
        let Some(target) = metadata_value(&class_info.metadata, "target-class") else {
            return SemanticResolution::Reject(
                "ClassInfo child has no target class for sets Expression".to_owned(),
            );
        };
        if target.is_empty() {
            return SemanticResolution::Reject(
                "ClassInfo child has an empty target class for sets Expression".to_owned(),
            );
        }

        SemanticResolution::Resolved {
            return_type: target.to_owned(),
            multiplicity: DynamicMultiplicity::Multiple,
            metadata: vec![metadata("semantic-mode", "sets-type")],
        }
    })
}

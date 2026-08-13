use super::{SemanticResolution, matches, metadata, property, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".PropExprWXYZ";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, CLASS_SUFFIX).then(|| {
        let Some(axis) = payload.tags.iter().find_map(|tag| {
            matches!(tag.value.as_str(), "w" | "x" | "y" | "z").then_some(tag.value.as_str())
        }) else {
            return SemanticResolution::Reject(
                "WXYZ Expression requires a selected axis".to_owned(),
            );
        };
        let options = payload
            .property_options
            .iter()
            .filter(|option| {
                option
                    .supported_axes
                    .iter()
                    .any(|supported| supported.eq_ignore_ascii_case(axis))
            })
            .cloned()
            .collect::<Vec<_>>();
        match property::resolve_options(
            &options,
            property::source_multiplicity(payload),
            "wxyz-property",
        ) {
            SemanticResolution::Resolved {
                return_type,
                multiplicity,
                metadata: mut entries,
            } => {
                entries.push(metadata("wxyz-axis", axis));
                SemanticResolution::Resolved {
                    return_type,
                    multiplicity,
                    metadata: entries,
                }
            }
            SemanticResolution::Reject(_) => SemanticResolution::Reject(format!(
                "source type has no registered {axis} axis component"
            )),
        }
    })
}

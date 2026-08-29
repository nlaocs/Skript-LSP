use super::{SemanticResolution, matches, metadata, property, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".PropExprWXYZ";
const HANDLER_ID: &str = "core.expression.prop-expr-wxyz";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        let Some(axis) = payload.tags.iter().find_map(|tag| {
            matches!(tag.value.as_str(), "w" | "x" | "y" | "z").then_some(tag.value.as_str())
        }) else {
            return SemanticResolution::Reject(
                "WXYZ Expression requires a selected axis".to_owned(),
            );
        };
        let options = match property::selected_options(payload) {
            Ok(options) => options,
            Err(reason) => return SemanticResolution::Reject(reason),
        };
        let options = options
            .iter()
            .filter(|option| {
                option
                    .supported_axes
                    .iter()
                    .any(|supported| supported.eq_ignore_ascii_case(axis))
            })
            .cloned()
            .collect::<Vec<_>>();
        let source = property::source_child_for_options(payload, &options);
        match property::resolve_options(
            &payload.registration_id,
            &options,
            source,
            source
                .and_then(|child| child.multiplicity)
                .unwrap_or(DynamicMultiplicity::Both),
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

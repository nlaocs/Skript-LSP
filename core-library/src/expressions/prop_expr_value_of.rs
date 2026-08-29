use super::{SemanticResolution, matches, metadata_value, property, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".PropExprValueOf";
const HANDLER_ID: &str = "core.expression.prop-expr-value-of";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        let options = match property::selected_options(payload) {
            Ok(options) if !options.is_empty() => options,
            Ok(_) => {
                return SemanticResolution::Reject(
                    "source type has no registered typed value property".to_owned(),
                );
            }
            Err(reason) => return SemanticResolution::Reject(reason),
        };
        if let Some(target) = payload
            .children
            .iter()
            .find_map(|child| metadata_value(&child.metadata, "target-class"))
        {
            let source = property::source_child_for_options(payload, &options);
            return match property::resolve_options(
                &payload.registration_id,
                &options,
                source,
                source
                    .and_then(|child| child.multiplicity)
                    .unwrap_or(DynamicMultiplicity::Both),
                "typed-value-target",
            ) {
                SemanticResolution::Resolved {
                    multiplicity,
                    metadata,
                    ..
                } => SemanticResolution::Resolved {
                    return_type: target.to_owned(),
                    multiplicity,
                    metadata,
                },
                rejected => rejected,
            };
        }
        property::resolve(payload, "typed-value-property")
    })
}

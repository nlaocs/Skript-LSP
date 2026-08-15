use super::{SemanticResolution, matches, metadata, metadata_value, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprRandom";
const CLASS_INFO: &str = "ch.njol.skript.classes.ClassInfo";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, CLASS_SUFFIX).then(|| {
        let Some(class_info) = payload
            .children
            .iter()
            .find(|child| child.return_type.as_deref() == Some(CLASS_INFO))
        else {
            return SemanticResolution::Reject(
                "random Expression requires a ClassInfo child".to_owned(),
            );
        };
        let Some(source_type) = payload.children.iter().find_map(|child| {
            (child.return_type.as_deref() != Some(CLASS_INFO))
                .then_some(child.return_type.as_deref())
                .flatten()
        }) else {
            return SemanticResolution::Reject(
                "random Expression requires a typed source Expression".to_owned(),
            );
        };

        let mut output_metadata = vec![metadata("semantic-mode", "random-element")];
        let selection_class = metadata_value(&class_info.metadata, "target-class");
        if let Some(selection_class) = selection_class {
            output_metadata.push(metadata("selection-class", selection_class));
        }
        SemanticResolution::Resolved {
            return_type: selection_class.unwrap_or(source_type).to_owned(),
            multiplicity: DynamicMultiplicity::Single,
            metadata: output_metadata,
        }
    })
}

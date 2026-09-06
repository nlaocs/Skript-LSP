use super::{SemanticResolution, matches, metadata, property, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionChild, RegisteredExpressionPayload,
    RegisteredSyntaxHandler,
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
        if options.is_empty() {
            return SemanticResolution::Reject(format!(
                "source type has no registered {axis} axis component"
            ));
        }
        let Some(source) = property::source_child_for_options(payload, &options) else {
            return SemanticResolution::Unresolved {
                reason: "WXYZ property source is unresolved".to_owned(),
                metadata: vec![metadata("semantic-mode", "wxyz-property")],
            };
        };
        let Some(multiplicity) = wxyz_source_multiplicity(Some(source)) else {
            return SemanticResolution::Unresolved {
                reason: "WXYZ property source multiplicity is unresolved".to_owned(),
                metadata: vec![metadata("semantic-mode", "wxyz-property")],
            };
        };
        match property::resolve_options(
            &payload.registration_id,
            &options,
            Some(source),
            multiplicity,
            "wxyz-property",
        ) {
            SemanticResolution::Resolved {
                return_type,
                possible_return_types,
                possible_return_types_state,
                multiplicity,
                metadata: mut entries,
            } => {
                entries.push(metadata("wxyz-axis", axis));
                SemanticResolution::Resolved {
                    return_type,
                    possible_return_types,
                    possible_return_types_state,
                    multiplicity,
                    metadata: entries,
                }
            }
            unresolved @ SemanticResolution::Unresolved { .. } => unresolved,
            rejection @ SemanticResolution::Reject(_) => rejection,
        }
    })
}

fn wxyz_source_multiplicity(
    source: Option<&RegisteredExpressionChild>,
) -> Option<DynamicMultiplicity> {
    // PropExprWXYZ inherits the property expression's source cardinality.
    // An explicit Both is meaningful; missing child data is unresolved.
    source.and_then(|child| child.multiplicity)
}

#[cfg(test)]
mod tests {
    use super::wxyz_source_multiplicity;
    use crate::nlaocs::skript_parser_addon::types::{
        DynamicMultiplicity, ExpressionPossibleReturnTypesState, RegisteredExpressionChild,
    };

    fn child(multiplicity: Option<DynamicMultiplicity>) -> RegisteredExpressionChild {
        RegisteredExpressionChild {
            default_expression: None,
            text: "value".to_owned(),
            kind: "expression".to_owned(),
            parser_id: None,
            definition_id: None,
            registration_id: None,
            pattern_index: None,
            element_class: None,
            return_type: None,
            possible_return_types: Vec::new(),
            possible_return_types_state: ExpressionPossibleReturnTypesState::Unresolved,
            multiplicity,
            public_data: Vec::new(),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn wxyz_property_requires_source_multiplicity() {
        assert_eq!(wxyz_source_multiplicity(None), None);
        assert_eq!(
            wxyz_source_multiplicity(Some(&child(Some(DynamicMultiplicity::Both)))),
            Some(DynamicMultiplicity::Both)
        );
    }
}

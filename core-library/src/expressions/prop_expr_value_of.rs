use super::{
    SemanticResolution, matches, metadata, metadata_value, property, register_handler,
    resolved_with_metadata,
};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionChild, RegisteredExpressionPayload,
    RegisteredSyntaxHandler,
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
            let Some(source) = source else {
                return SemanticResolution::Unresolved {
                    reason: "typed value property source is unresolved".to_owned(),
                    metadata: vec![metadata("semantic-mode", "typed-value-target")],
                };
            };
            let Some(multiplicity) = typed_value_multiplicity(Some(source)) else {
                return SemanticResolution::Unresolved {
                    reason: "typed value property source multiplicity is unresolved".to_owned(),
                    metadata: vec![metadata("semantic-mode", "typed-value-target")],
                };
            };
            return match property::resolve_options(
                &payload.registration_id,
                &options,
                Some(source),
                multiplicity,
                "typed-value-target",
            ) {
                SemanticResolution::Resolved {
                    multiplicity,
                    metadata,
                    ..
                } => resolved_with_metadata(target.to_owned(), multiplicity, metadata),
                rejected => rejected,
            };
        }
        property::resolve(payload, "typed-value-property")
    })
}

fn typed_value_multiplicity(
    source: Option<&RegisteredExpressionChild>,
) -> Option<DynamicMultiplicity> {
    // PropExprValueOf is a PropertyBaseExpression; it delegates isSingle()
    // to the source expression. Both is valid when that source explicitly
    // reports it, but absent metadata cannot be treated as Both.
    source.and_then(|child| child.multiplicity)
}

#[cfg(test)]
mod tests {
    use super::typed_value_multiplicity;
    use crate::nlaocs::skript_parser_addon::types::{
        DynamicMultiplicity, ExpressionPossibleReturnTypesState, RegisteredExpressionChild,
    };

    fn child(multiplicity: Option<DynamicMultiplicity>) -> RegisteredExpressionChild {
        RegisteredExpressionChild {
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
            metadata: Vec::new(),
        }
    }

    #[test]
    fn typed_value_property_does_not_invent_source_multiplicity() {
        assert_eq!(
            typed_value_multiplicity(Some(&child(None))),
            None,
            "missing source metadata must remain unresolved"
        );
    }

    #[test]
    fn typed_value_property_preserves_explicit_both() {
        assert_eq!(
            typed_value_multiplicity(Some(&child(Some(DynamicMultiplicity::Both)))),
            Some(DynamicMultiplicity::Both)
        );
    }
}

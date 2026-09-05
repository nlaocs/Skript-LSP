use super::{
    SemanticResolution, matches, metadata, register_handler, resolved_with_possible_types,
};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionChild, RegisteredExpressionPayload,
    RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprAnyOf";
const HANDLER_ID: &str = "core.expression.expr-any-of";

/// Registers the semantic override for Skript's ExprAnyOf wrapper.
pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        if payload.children.len() != 1 {
            return SemanticResolution::Reject(
                "any-of Expression requires exactly one wrapped Expression".to_owned(),
            );
        }

        match resolve_wrapped_expression(&payload.children[0]) {
            Some((
                return_type,
                possible_return_types,
                possible_return_types_state,
                multiplicity,
            )) => resolved_with_possible_types(
                return_type,
                possible_return_types,
                possible_return_types_state,
                // WrapperExpression::isSingle() is unconditionally true in Skript,
                // even when the wrapped expression itself returns a list.
                multiplicity,
                vec![metadata("semantic-mode", "any-of-wrapper")],
            ),
            None => SemanticResolution::Reject(
                "any-of Expression requires a typed wrapped Expression".to_owned(),
            ),
        }
    })
}

fn resolve_wrapped_expression(
    child: &RegisteredExpressionChild,
) -> Option<(
    String,
    Vec<String>,
    crate::nlaocs::skript_parser_addon::types::ExpressionPossibleReturnTypesState,
    DynamicMultiplicity,
)> {
    child
        .return_type
        .as_deref()
        .filter(|return_type| !return_type.is_empty())
        .map(str::to_owned)
        .map(|return_type| {
            let possible = if child.possible_return_types.is_empty() {
                vec![return_type.clone()]
            } else {
                child.possible_return_types.clone()
            };
            (
                return_type,
                possible,
                child.possible_return_types_state,
                DynamicMultiplicity::Single,
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn child(return_type: Option<&str>) -> RegisteredExpressionChild {
        RegisteredExpressionChild {
            text: "value".to_owned(),
            kind: "custom".to_owned(),
            parser_id: None,
            definition_id: None,
            registration_id: None,
            pattern_index: None,
            element_class: None,
            return_type: return_type.map(str::to_owned),
            possible_return_types: return_type.into_iter().map(str::to_owned).collect(),
            possible_return_types_state: if return_type.is_some() {
                crate::nlaocs::skript_parser_addon::types::ExpressionPossibleReturnTypesState::Complete
            } else {
                crate::nlaocs::skript_parser_addon::types::ExpressionPossibleReturnTypesState::Unresolved
            },
            multiplicity: Some(DynamicMultiplicity::Multiple),
            public_data: Vec::new(),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn keeps_the_wrapped_return_type_but_forces_single_multiplicity() {
        let wrapped = child(Some("org.bukkit.entity.Player"));

        assert_eq!(
            resolve_wrapped_expression(&wrapped),
            Some((
                "org.bukkit.entity.Player".to_owned(),
                vec!["org.bukkit.entity.Player".to_owned()],
                crate::nlaocs::skript_parser_addon::types::ExpressionPossibleReturnTypesState::Complete,
                DynamicMultiplicity::Single
            ))
        );
    }

    #[test]
    fn refuses_to_guess_a_missing_wrapped_return_type() {
        assert_eq!(resolve_wrapped_expression(&child(None)), None);
        assert_eq!(resolve_wrapped_expression(&child(Some(""))), None);
    }
}

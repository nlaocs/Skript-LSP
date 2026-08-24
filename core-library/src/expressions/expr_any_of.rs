use super::{SemanticResolution, matches, metadata, register_handler};
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
            Some((return_type, multiplicity)) => SemanticResolution::Resolved {
                return_type,
                // WrapperExpression::isSingle() is unconditionally true in Skript,
                // even when the wrapped expression itself returns a list.
                multiplicity,
                metadata: vec![metadata("semantic-mode", "any-of-wrapper")],
            },
            None => SemanticResolution::Reject(
                "any-of Expression requires a typed wrapped Expression".to_owned(),
            ),
        }
    })
}

fn resolve_wrapped_expression(
    child: &RegisteredExpressionChild,
) -> Option<(String, DynamicMultiplicity)> {
    child
        .return_type
        .as_deref()
        .filter(|return_type| !return_type.is_empty())
        .map(str::to_owned)
        .map(|return_type| (return_type, DynamicMultiplicity::Single))
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
            multiplicity: Some(DynamicMultiplicity::Multiple),
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

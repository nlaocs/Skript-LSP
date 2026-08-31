use super::{
    SemanticResolution, matches, metadata, metadata_value, register_handler,
    resolved_with_possible_types,
};
use crate::nlaocs::skript_parser_addon::types::{
    ExpressionPossibleReturnTypesState, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprKeyed";
const HANDLER_ID: &str = "core.expression.expr-keyed";
const KEY_PROVIDER: &str = "expression.capability.key-provider";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_keyed(payload))
}

fn resolve_keyed(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let Some(source) = payload.children.first() else {
        return SemanticResolution::Reject("keyed Expression requires a source".to_owned());
    };
    if metadata_value(&source.metadata, KEY_PROVIDER) != Some("true") {
        return SemanticResolution::Unresolved {
            reason: "whether the source can provide keys is unresolved".to_owned(),
            metadata: vec![metadata("semantic-mode", "keyed-wrapper")],
        };
    }
    // ExprKeyed is Skript's transparent WrapperExpression. Its Object
    // registration is only a declaration placeholder; the wrapped expression
    // supplies the actual return type and multiplicity.
    let Some(return_type) = source.return_type.as_deref() else {
        return SemanticResolution::Unresolved {
            reason: "keyed Expression return type is unresolved".to_owned(),
            metadata: vec![metadata("semantic-mode", "keyed-wrapper")],
        };
    };
    let Some(multiplicity) = source.multiplicity else {
        return SemanticResolution::Unresolved {
            reason: "keyed Expression multiplicity is unresolved".to_owned(),
            metadata: vec![metadata("semantic-mode", "keyed-wrapper")],
        };
    };
    resolved_with_possible_types(
        return_type.to_owned(),
        delegated_possible_return_types(
            return_type,
            &source.possible_return_types,
            source.possible_return_types_state,
        ),
        source.possible_return_types_state,
        multiplicity,
        vec![
            metadata("semantic-mode", "keyed-wrapper"),
            metadata(KEY_PROVIDER, "true"),
        ],
    )
}

fn delegated_possible_return_types(
    return_type: &str,
    possible_return_types: &[String],
    state: ExpressionPossibleReturnTypesState,
) -> Vec<String> {
    if !possible_return_types.is_empty() {
        possible_return_types.to_vec()
    } else if state == ExpressionPossibleReturnTypesState::Complete {
        // Expression's default possibleReturnTypes() is [getReturnType()].
        vec![return_type.to_owned()]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{KEY_PROVIDER, delegated_possible_return_types};
    use crate::nlaocs::skript_parser_addon::types::ExpressionPossibleReturnTypesState;

    #[test]
    fn keyed_wrapper_publishes_the_shared_capability_key() {
        assert_eq!(KEY_PROVIDER, "expression.capability.key-provider");
    }

    #[test]
    fn keyed_wrapper_uses_the_expression_default_only_when_complete() {
        assert_eq!(
            delegated_possible_return_types(
                "java.lang.String",
                &[],
                ExpressionPossibleReturnTypesState::Complete,
            ),
            ["java.lang.String"]
        );
        assert!(
            delegated_possible_return_types(
                "java.lang.String",
                &[],
                ExpressionPossibleReturnTypesState::Unresolved,
            )
            .is_empty()
        );
    }

    #[test]
    fn keyed_wrapper_preserves_explicit_possible_types() {
        let possible = vec!["a.Type".to_owned(), "b.Type".to_owned()];
        assert_eq!(
            delegated_possible_return_types(
                "java.lang.Object",
                &possible,
                ExpressionPossibleReturnTypesState::Partial,
            ),
            possible
        );
    }
}

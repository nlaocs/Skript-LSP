use super::{
    SemanticResolution, matches, metadata, optional_integer_amount_multiplicity, register_handler,
    resolved_with_metadata,
};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprRandomCharacter";
const HANDLER_ID: &str = "core.expression.expr-random-character";
const STRING: &str = "java.lang.String";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        let Some(multiplicity) = optional_integer_amount_multiplicity(&payload.children) else {
            return SemanticResolution::Reject(
                "random character Expression has an unexpected child layout".to_owned(),
            );
        };
        let alphanumeric = payload.tags.iter().any(|tag| tag.value == "alphanumeric");
        resolution(alphanumeric, multiplicity, payload.children.len() == 2)
    })
}

fn resolution(
    alphanumeric: bool,
    multiplicity: DynamicMultiplicity,
    implicit_amount: bool,
) -> SemanticResolution {
    resolved_with_metadata(
        STRING.to_owned(),
        multiplicity,
        vec![
            metadata("semantic-mode", "random-character"),
            metadata("alphanumeric", if alphanumeric { "true" } else { "false" }),
            metadata(
                "amount-source",
                if implicit_amount {
                    "implicit-one"
                } else {
                    "expression"
                },
            ),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nlaocs::skript_parser_addon::types::RegisteredExpressionChild;

    fn child(text: &str, kind: &str, return_type: &str) -> RegisteredExpressionChild {
        RegisteredExpressionChild {
            default_expression: None,
            text: text.to_owned(),
            kind: kind.to_owned(),
            parser_id: None,
            definition_id: None,
            registration_id: None,
            pattern_index: None,
            element_class: None,
            return_type: Some(return_type.to_owned()),
            possible_return_types: vec![return_type.to_owned()],
            possible_return_types_state:
                crate::nlaocs::skript_parser_addon::types::ExpressionPossibleReturnTypesState::Complete,
            multiplicity: Some(DynamicMultiplicity::Single),
            public_data: Vec::new(),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn omitted_amount_is_single() {
        let children = [
            child("from", "literal", STRING),
            child("to", "literal", STRING),
        ];
        assert_eq!(
            optional_integer_amount_multiplicity(&children),
            Some(DynamicMultiplicity::Single)
        );
    }

    #[test]
    fn explicit_literal_one_is_single() {
        let children = [
            child("1", "literal", "java.lang.Long"),
            child("from", "literal", STRING),
            child("to", "literal", STRING),
        ];
        assert_eq!(
            optional_integer_amount_multiplicity(&children),
            Some(DynamicMultiplicity::Single)
        );
    }

    #[test]
    fn nonliteral_or_non_one_amount_is_multiple() {
        for amount in [
            child("1", "variable", "java.lang.Long"),
            child("3", "literal", "java.lang.Long"),
        ] {
            let children = [
                amount,
                child("from", "literal", STRING),
                child("to", "literal", STRING),
            ];
            assert_eq!(
                optional_integer_amount_multiplicity(&children),
                Some(DynamicMultiplicity::Multiple)
            );
        }
    }

    #[test]
    fn unknown_child_layout_is_not_guessed() {
        assert_eq!(optional_integer_amount_multiplicity(&[]), None);
    }

    #[test]
    fn resolution_reports_string_type_and_alphanumeric_metadata() {
        let SemanticResolution::Resolved {
            return_type,
            multiplicity,
            metadata,
            ..
        } = resolution(true, DynamicMultiplicity::Single, true)
        else {
            panic!("random character resolution must succeed");
        };
        assert_eq!(return_type, STRING);
        assert_eq!(multiplicity, DynamicMultiplicity::Single);
        assert_eq!(metadata[0].value, "random-character");
        assert_eq!(metadata[1].value, "true");
        assert_eq!(metadata[2].value, "implicit-one");
    }
}

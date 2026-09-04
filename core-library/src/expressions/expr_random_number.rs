use super::{
    SemanticResolution, matches, metadata, optional_integer_amount_multiplicity, register_handler,
    resolved_with_metadata,
};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprRandomNumber";
const HANDLER_ID: &str = "core.expression.expr-random-number";
const INTEGER: &str = "java.lang.Long";
const NUMBER: &str = "java.lang.Double";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        let Some(multiplicity) = optional_integer_amount_multiplicity(&payload.children) else {
            return SemanticResolution::Reject(
                "random number Expression has an unexpected child layout".to_owned(),
            );
        };
        // Skript 2.7 replaced the numeric parse mark (`1¦integer`) with the
        // named `:integer` tag. The registration itself is the compatibility boundary.
        let Some(integer) = integer_variant(
            crate::runtime::skript_at_least(2, 7),
            payload.mark,
            payload.tags.iter().any(|tag| tag.value == "integer"),
        ) else {
            return SemanticResolution::Unresolved {
                reason: "Skript version is unavailable, so random number syntax is unresolved"
                    .to_owned(),
                metadata: vec![metadata("semantic-mode", "random-number")],
            };
        };
        resolution(integer, multiplicity, payload.children.len() == 2)
    })
}

fn integer_variant(
    version_at_least_27: Option<bool>,
    mark: i32,
    has_integer_tag: bool,
) -> Option<bool> {
    match version_at_least_27 {
        Some(true) => Some(has_integer_tag),
        Some(false) => Some(mark == 1),
        None => None,
    }
}

fn resolution(
    integer: bool,
    multiplicity: DynamicMultiplicity,
    implicit_amount: bool,
) -> SemanticResolution {
    resolved_with_metadata(
        if integer { INTEGER } else { NUMBER }.to_owned(),
        multiplicity,
        vec![
            metadata("semantic-mode", "random-number"),
            metadata("numeric-kind", if integer { "integer" } else { "number" }),
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
            child("lower", "variable", NUMBER),
            child("upper", "variable", NUMBER),
        ];
        assert_eq!(
            optional_integer_amount_multiplicity(&children),
            Some(DynamicMultiplicity::Single)
        );
    }

    #[test]
    fn explicit_literal_one_is_single() {
        for integer_class in ["java.lang.Long", "java.lang.Integer"] {
            let children = [
                child("1", "literal", integer_class),
                child("lower", "variable", NUMBER),
                child("upper", "variable", NUMBER),
            ];
            assert_eq!(
                optional_integer_amount_multiplicity(&children),
                Some(DynamicMultiplicity::Single)
            );
        }
    }

    #[test]
    fn nonliteral_or_non_one_amount_is_multiple() {
        for amount in [
            child("1", "variable", INTEGER),
            child("2", "literal", INTEGER),
        ] {
            let children = [
                amount,
                child("lower", "variable", NUMBER),
                child("upper", "variable", NUMBER),
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
    fn resolution_reports_integer_type_and_amount_metadata() {
        let SemanticResolution::Resolved {
            return_type,
            multiplicity,
            metadata,
            ..
        } = resolution(true, DynamicMultiplicity::Multiple, false)
        else {
            panic!("random number resolution must succeed");
        };
        assert_eq!(return_type, INTEGER);
        assert_eq!(multiplicity, DynamicMultiplicity::Multiple);
        assert_eq!(metadata[0].value, "random-number");
        assert_eq!(metadata[1].value, "integer");
        assert_eq!(metadata[2].value, "expression");
    }

    #[test]
    fn unknown_version_does_not_choose_mark_or_tag_syntax() {
        assert_eq!(integer_variant(None, 1, true), None);
        assert_eq!(integer_variant(Some(false), 1, false), Some(true));
        assert_eq!(integer_variant(Some(true), 0, true), Some(true));
    }
}

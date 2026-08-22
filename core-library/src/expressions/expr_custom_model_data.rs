use super::{SemanticResolution, matches, metadata, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprCustomModelData";
const HANDLER_ID: &str = "core.expression.expr-custom-model-data";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        let (return_type, multiplicity, mode) = match payload.mark {
            0 => {
                let Some(multiplicity) = payload
                    .children
                    .first()
                    .and_then(|child| child.multiplicity)
                else {
                    return SemanticResolution::Reject(
                        "custom model data Expression requires an item source".to_owned(),
                    );
                };
                ("java.lang.Integer", multiplicity, "legacy-integer")
            }
            1 => ("java.lang.Float", DynamicMultiplicity::Multiple, "floats"),
            2 => ("java.lang.Boolean", DynamicMultiplicity::Multiple, "flags"),
            3 => ("java.lang.String", DynamicMultiplicity::Multiple, "strings"),
            4 => (
                "ch.njol.skript.util.Color",
                DynamicMultiplicity::Multiple,
                "colors",
            ),
            5 => (
                "java.lang.Object",
                DynamicMultiplicity::Multiple,
                "complete",
            ),
            _ => {
                return SemanticResolution::Reject(
                    "custom model data Expression has an unknown parse mark".to_owned(),
                );
            }
        };
        SemanticResolution::Resolved {
            return_type: return_type.to_owned(),
            multiplicity,
            metadata: vec![
                metadata("semantic-mode", "custom-model-data"),
                metadata("custom-model-data-kind", mode),
            ],
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nlaocs::skript_parser_addon::types::{
        DynamicMultiplicity, ExpressionPossibleReturnTypesState, ExpressionReturnTypeState,
        MappedSpan, OriginKind, RegisteredExpressionChild, RegisteredExpressionPayload,
        SourceOrigin, TextRange,
    };

    fn payload(
        mark: i32,
        source_multiplicity: Option<DynamicMultiplicity>,
    ) -> RegisteredExpressionPayload {
        let range = TextRange { start: 0, end: 1 };
        RegisteredExpressionPayload {
            input: "custom model data".to_owned(),
            definition_id: "expression:test".to_owned(),
            registration_id: "expression:test:0".to_owned(),
            element_class: "ch.njol.skript.expressions.ExprCustomModelData".to_owned(),
            related_property: None,
            pattern_index: 0,
            pattern: "custom model data".to_owned(),
            span: MappedSpan {
                virtual_range: range,
                origins: vec![SourceOrigin {
                    original_range: range,
                    kind: OriginKind::Exact,
                    expansion: None,
                }],
            },
            expected_types: Vec::new(),
            declared_return_type: None,
            declared_multiplicity: None,
            return_type_state: ExpressionReturnTypeState::Dynamic,
            possible_return_types: Vec::new(),
            possible_return_types_state: ExpressionPossibleReturnTypesState::Unresolved,
            regex_captures: Vec::new(),
            tags: Vec::new(),
            mark,
            children: source_multiplicity.map_or_else(Vec::new, |multiplicity| {
                vec![RegisteredExpressionChild {
                    text: "item".to_owned(),
                    kind: "custom".to_owned(),
                    parser_id: None,
                    element_class: None,
                    return_type: Some("org.bukkit.inventory.ItemStack".to_owned()),
                    multiplicity: Some(multiplicity),
                    metadata: Vec::new(),
                }]
            }),
            parsed_captures: Vec::new(),
            common_child_return_type: None,
            type_options: Vec::new(),
            property_options: Vec::new(),
            effective_return_type: None,
            effective_multiplicity: None,
            metadata: Vec::new(),
        }
    }

    #[test]
    fn mark_zero_delegates_source_multiplicity() {
        let result = resolve(&payload(0, Some(DynamicMultiplicity::Single)));

        assert!(matches!(
            result,
            Some(SemanticResolution::Resolved {
                return_type,
                multiplicity: DynamicMultiplicity::Single,
                ..
            }) if return_type == "java.lang.Integer"
        ));
    }

    #[test]
    fn marks_select_the_documented_component_types() {
        for (mark, expected_type) in [
            (1, "java.lang.Float"),
            (2, "java.lang.Boolean"),
            (3, "java.lang.String"),
            (4, "ch.njol.skript.util.Color"),
            (5, "java.lang.Object"),
        ] {
            let Some(SemanticResolution::Resolved { return_type, .. }) =
                resolve(&payload(mark, None))
            else {
                panic!("custom model data mark {mark} must resolve");
            };
            assert_eq!(return_type, expected_type);
        }
    }

    #[test]
    fn non_legacy_marks_are_multiple() {
        assert!(matches!(
            resolve(&payload(3, None)),
            Some(SemanticResolution::Resolved {
                multiplicity: DynamicMultiplicity::Multiple,
                ..
            })
        ));
    }

    #[test]
    fn mark_zero_requires_a_source_and_unknown_marks_are_rejected() {
        assert!(matches!(
            resolve(&payload(0, None)),
            Some(SemanticResolution::Reject(_))
        ));
        assert!(matches!(
            resolve(&payload(6, None)),
            Some(SemanticResolution::Reject(_))
        ));
    }
}

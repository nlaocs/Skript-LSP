use super::{SemanticResolution, matches, metadata, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprShuffledList";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, CLASS_SUFFIX).then(|| {
        let Some(return_type) = payload
            .children
            .first()
            .and_then(|child| child.return_type.as_deref())
        else {
            return SemanticResolution::Reject(
                "shuffled list Expression requires a typed source Expression".to_owned(),
            );
        };
        SemanticResolution::Resolved {
            return_type: return_type.to_owned(),
            multiplicity: DynamicMultiplicity::Multiple,
            metadata: vec![metadata("semantic-mode", "shuffled-list")],
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nlaocs::skript_parser_addon::types::{
        ExpressionPossibleReturnTypesState, ExpressionReturnTypeState, MappedSpan, OriginKind,
        RegisteredExpressionChild, SourceOrigin, TextRange,
    };

    fn payload(child_type: Option<&str>) -> RegisteredExpressionPayload {
        let range = TextRange { start: 0, end: 1 };
        RegisteredExpressionPayload {
            input: "shuffled players".to_owned(),
            definition_id: "expression:test".to_owned(),
            registration_id: "expression:test:0".to_owned(),
            element_class: "ch.njol.skript.expressions.ExprShuffledList".to_owned(),
            related_property: None,
            pattern_index: 0,
            pattern: "shuffled %objects%".to_owned(),
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
            mark: 0,
            children: child_type.map_or_else(Vec::new, |return_type| {
                vec![RegisteredExpressionChild {
                    text: "players".to_owned(),
                    kind: "custom".to_owned(),
                    parser_id: None,
                    element_class: None,
                    return_type: Some(return_type.to_owned()),
                    multiplicity: Some(DynamicMultiplicity::Single),
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
    fn delegates_the_child_type_and_always_returns_multiple() {
        let result = resolve(&payload(Some("org.bukkit.entity.Player")));

        assert!(matches!(
            result,
            Some(SemanticResolution::Resolved {
                return_type,
                multiplicity: DynamicMultiplicity::Multiple,
                ..
            }) if return_type == "org.bukkit.entity.Player"
        ));
    }

    #[test]
    fn rejects_a_missing_child_type() {
        assert!(matches!(
            resolve(&payload(None)),
            Some(SemanticResolution::Reject(_))
        ));
    }
}

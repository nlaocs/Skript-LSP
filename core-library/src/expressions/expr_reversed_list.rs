use super::{SemanticResolution, matches, metadata, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprReversedList";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, CLASS_SUFFIX).then(|| {
        resolve_ordered_list(
            payload,
            "reversed-list",
            "A single object cannot be reversed",
        )
    })
}

fn resolve_ordered_list(
    payload: &RegisteredExpressionPayload,
    mode: &str,
    single_error: &str,
) -> SemanticResolution {
    let Some(child) = payload.children.first() else {
        return SemanticResolution::Reject(
            "ordered list Expression requires a source Expression".to_owned(),
        );
    };
    if child.multiplicity == Some(DynamicMultiplicity::Single) {
        return SemanticResolution::Reject(single_error.to_owned());
    }
    let Some(return_type) = child.return_type.as_deref() else {
        return SemanticResolution::Reject(
            "ordered list Expression requires a typed source Expression".to_owned(),
        );
    };
    SemanticResolution::Resolved {
        return_type: return_type.to_owned(),
        multiplicity: DynamicMultiplicity::Multiple,
        metadata: vec![metadata("semantic-mode", mode)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nlaocs::skript_parser_addon::types::{
        ExpressionPossibleReturnTypesState, ExpressionReturnTypeState, MappedSpan, OriginKind,
        RegisteredExpressionChild, RegisteredExpressionPayload, SourceOrigin, TextRange,
    };

    fn payload(multiplicity: DynamicMultiplicity) -> RegisteredExpressionPayload {
        let range = TextRange { start: 0, end: 1 };
        RegisteredExpressionPayload {
            input: "reversed players".to_owned(),
            definition_id: "expression:test".to_owned(),
            registration_id: "expression:test:0".to_owned(),
            element_class: "ch.njol.skript.expressions.ExprReversedList".to_owned(),
            related_property: None,
            pattern_index: 0,
            pattern: "reversed %objects%".to_owned(),
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
            children: vec![RegisteredExpressionChild {
                text: "players".to_owned(),
                kind: "custom".to_owned(),
                parser_id: None,
                element_class: None,
                return_type: Some("org.bukkit.entity.Player".to_owned()),
                multiplicity: Some(multiplicity),
                metadata: Vec::new(),
            }],
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
    fn delegates_the_child_type_and_returns_multiple() {
        let result = resolve(&payload(DynamicMultiplicity::Multiple));

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
    fn rejects_a_single_source_expression() {
        assert!(matches!(
            resolve(&payload(DynamicMultiplicity::Single)),
            Some(SemanticResolution::Reject(_))
        ));
    }
}

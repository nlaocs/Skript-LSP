use super::{
    SemanticResolution, matches, metadata, register_handler, resolved_with_possible_types,
};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionPossibleReturnTypesState, RegisteredExpressionPayload,
    RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprDefaultValue";
const HANDLER_ID: &str = "core.expression.expr-default-value";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    if !matches(payload, HANDLER_ID) {
        return None;
    }

    if payload.children.len() != 2 {
        return Some(SemanticResolution::Reject(
            "default-value Expression requires two child Expressions".to_owned(),
        ));
    }
    let (possible_return_types, possible_return_types_state) = child_possible_return_types(payload);
    // ExprDefaultValue asks Utils.getSuperType() for every possibleReturnTypes()
    // entry, not only the children' current getReturnType() representatives.
    let return_type = crate::catalog::common_assignable_class(&possible_return_types)
        .ok()
        .flatten()
        .or_else(|| payload.common_child_return_type.clone());
    let Some(return_type) = return_type else {
        return Some(SemanticResolution::Reject(
            "default-value Expression has no common child return type".to_owned(),
        ));
    };
    let Some(multiplicity) = child_multiplicity(payload) else {
        return Some(SemanticResolution::Reject(
            "default-value Expression has unresolved child multiplicity".to_owned(),
        ));
    };
    Some(resolved_with_possible_types(
        return_type,
        possible_return_types,
        possible_return_types_state,
        multiplicity,
        vec![metadata("semantic-mode", "default-value")],
    ))
}

fn child_possible_return_types(
    payload: &RegisteredExpressionPayload,
) -> (Vec<String>, ExpressionPossibleReturnTypesState) {
    let mut types = payload
        .children
        .iter()
        .flat_map(|child| {
            if child.possible_return_types.is_empty() {
                child.return_type.iter().cloned().collect::<Vec<_>>()
            } else {
                child.possible_return_types.clone()
            }
        })
        .collect::<Vec<_>>();
    types.sort();
    types.dedup();
    let unresolved = payload.children.iter().any(|child| {
        child.possible_return_types_state == ExpressionPossibleReturnTypesState::Unresolved
    });
    let partial = payload.children.iter().any(|child| {
        child.possible_return_types_state == ExpressionPossibleReturnTypesState::Partial
    });
    let state = if unresolved && types.is_empty() {
        ExpressionPossibleReturnTypesState::Unresolved
    } else if unresolved || partial {
        ExpressionPossibleReturnTypesState::Partial
    } else {
        ExpressionPossibleReturnTypesState::Complete
    };
    (types, state)
}

fn child_multiplicity(payload: &RegisteredExpressionPayload) -> Option<DynamicMultiplicity> {
    let [values, default_values] = payload.children.as_slice() else {
        return None;
    };
    match (values.multiplicity, default_values.multiplicity) {
        (Some(DynamicMultiplicity::Single), Some(DynamicMultiplicity::Single)) => {
            Some(DynamicMultiplicity::Single)
        }
        (Some(DynamicMultiplicity::Multiple), _) | (_, Some(DynamicMultiplicity::Multiple)) => {
            Some(DynamicMultiplicity::Multiple)
        }
        (Some(DynamicMultiplicity::Both), Some(DynamicMultiplicity::Single))
        | (Some(DynamicMultiplicity::Single), Some(DynamicMultiplicity::Both))
        | (Some(DynamicMultiplicity::Both), Some(DynamicMultiplicity::Both)) => {
            Some(DynamicMultiplicity::Both)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nlaocs::skript_parser_addon::types::{
        DynamicMultiplicity, ExpressionExpectedType, ExpressionPossibleReturnTypesState,
        ExpressionReturnTypeState, MappedSpan, OriginKind, RegisteredExpressionChild,
        RegisteredExpressionPropertyOption, RegisteredExpressionTag, SourceOrigin, TextRange,
    };

    fn payload(
        possible_return_types: Vec<&str>,
        state: ExpressionPossibleReturnTypesState,
        left: Option<DynamicMultiplicity>,
        right: Option<DynamicMultiplicity>,
    ) -> RegisteredExpressionPayload {
        let range = TextRange { start: 0, end: 1 };
        RegisteredExpressionPayload {
            context: crate::nlaocs::skript_parser_addon::types::ParseContext {
                syntax_context: 0,
                event_classes: Vec::new(),
                values: Vec::new(),
            },
            input: "value otherwise fallback".to_owned(),
            definition_id: "expression:test".to_owned(),
            registration_id: "expression:test:0".to_owned(),
            element_class: CLASS_SUFFIX.to_owned(),
            related_property: None,
            pattern_index: 0,
            pattern: "%objects% otherwise %objects%".to_owned(),
            span: MappedSpan {
                virtual_range: range,
                origins: vec![SourceOrigin {
                    original_range: range,
                    kind: OriginKind::Exact,
                    expansion: None,
                }],
            },
            expected_types: vec![ExpressionExpectedType {
                class_name: "java.lang.Object".to_owned(),
                plural: false,
            }],
            declared_return_type: Some("java.lang.Object".to_owned()),
            declared_multiplicity: Some(DynamicMultiplicity::Both),
            return_type_state: ExpressionReturnTypeState::Dynamic,
            possible_return_types: possible_return_types
                .into_iter()
                .map(str::to_owned)
                .collect(),
            possible_return_types_state: state,
            time: 0,
            regex_captures: Vec::new(),
            tags: Vec::<RegisteredExpressionTag>::new(),
            mark: 0,
            children: vec![
                child("value", "java.lang.String", left),
                child("fallback", "java.lang.String", right),
            ],
            parsed_captures: Vec::new(),
            common_child_return_type: Some("java.lang.String".to_owned()),
            type_options: Vec::new(),
            property_options: Vec::<RegisteredExpressionPropertyOption>::new(),
            selected_property_option_indices: Vec::new(),
            effective_return_type: Some("java.lang.Object".to_owned()),
            effective_possible_return_types: Vec::new(),
            effective_possible_return_types_state: ExpressionPossibleReturnTypesState::Unresolved,
            effective_multiplicity: Some(DynamicMultiplicity::Both),
            public_data: Vec::new(),
            metadata: Vec::new(),
        }
    }

    fn child(
        text: &str,
        return_type: &str,
        multiplicity: Option<DynamicMultiplicity>,
    ) -> RegisteredExpressionChild {
        RegisteredExpressionChild {
            text: text.to_owned(),
            kind: "custom".to_owned(),
            parser_id: None,
            definition_id: None,
            registration_id: None,
            pattern_index: None,
            element_class: None,
            return_type: Some(return_type.to_owned()),
            possible_return_types: vec![return_type.to_owned()],
            possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
            multiplicity,
            public_data: Vec::new(),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn resolves_from_the_selected_children_common_type() {
        let value = payload(
            vec!["java.lang.String"],
            ExpressionPossibleReturnTypesState::Complete,
            Some(DynamicMultiplicity::Single),
            Some(DynamicMultiplicity::Single),
        );

        let Some(SemanticResolution::Resolved {
            return_type,
            multiplicity,
            ..
        }) = resolve(&value)
        else {
            panic!("complete default-value payload must resolve");
        };
        assert_eq!(return_type, "java.lang.String");
        assert_eq!(multiplicity, DynamicMultiplicity::Single);
    }

    #[test]
    fn preserves_unknown_single_or_multiple_state() {
        let value = payload(
            vec!["java.lang.String"],
            ExpressionPossibleReturnTypesState::Complete,
            Some(DynamicMultiplicity::Both),
            Some(DynamicMultiplicity::Single),
        );

        let Some(SemanticResolution::Resolved { multiplicity, .. }) = resolve(&value) else {
            panic!("default-value multiplicity must resolve");
        };
        assert_eq!(multiplicity, DynamicMultiplicity::Both);
    }

    #[test]
    fn possible_types_choose_a_more_specific_common_type_than_the_representative() {
        let mut value = payload(
            vec!["org.bukkit.entity.Player"],
            ExpressionPossibleReturnTypesState::Complete,
            Some(DynamicMultiplicity::Single),
            Some(DynamicMultiplicity::Single),
        );
        for child in &mut value.children {
            child.return_type = Some("java.lang.Object".to_owned());
            child.possible_return_types = vec!["org.bukkit.entity.Player".to_owned()];
        }
        value.common_child_return_type = Some("java.lang.Object".to_owned());

        let Some(SemanticResolution::Resolved { return_type, .. }) = resolve(&value) else {
            panic!("default-value possible types must resolve");
        };
        assert_eq!(return_type, "org.bukkit.entity.Player");
    }
}

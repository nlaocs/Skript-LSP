use super::{SemanticResolution, matches, metadata, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
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
    let Some(return_type) = payload.common_child_return_type.clone() else {
        return Some(SemanticResolution::Reject(
            "default-value Expression has no common child return type".to_owned(),
        ));
    };
    let Some(multiplicity) = child_multiplicity(payload) else {
        return Some(SemanticResolution::Reject(
            "default-value Expression has unresolved child multiplicity".to_owned(),
        ));
    };
    Some(SemanticResolution::Resolved {
        return_type,
        multiplicity,
        metadata: vec![metadata("semantic-mode", "default-value")],
    })
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
            effective_return_type: Some("java.lang.Object".to_owned()),
            effective_multiplicity: Some(DynamicMultiplicity::Both),
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
            element_class: None,
            return_type: Some(return_type.to_owned()),
            multiplicity,
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
}

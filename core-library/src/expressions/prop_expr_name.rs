use super::{
    SemanticResolution, matches, metadata, property, register_handler, resolved_with_metadata,
};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".PropExprName";
const HANDLER_ID: &str = "core.expression.prop-expr-name";
const LEGACY_CLASS_SUFFIX: &str = ".ExprName";
const LEGACY_HANDLER_ID: &str = "core.expression.expr-name";
const STRING: &str = "java.lang.String";
const OBJECT: &str = "java.lang.Object";
const COMPONENT: &str = "net.kyori.adventure.text.Component";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
    register_handler(handlers, LEGACY_HANDLER_ID, LEGACY_CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    if matches(payload, HANDLER_ID) {
        return Some(property::resolve(payload, "name-property"));
    }
    matches(payload, LEGACY_HANDLER_ID).then(|| resolve_legacy(payload))
}

fn resolve_legacy(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let Some(source) = payload.children.first() else {
        return SemanticResolution::Reject(
            "legacy ExprName requires a source Expression".to_owned(),
        );
    };
    let Some(multiplicity) = legacy_source_multiplicity(source.multiplicity) else {
        return SemanticResolution::Unresolved {
            reason: "legacy name property source multiplicity is unresolved".to_owned(),
            metadata: vec![metadata("semantic-mode", "legacy-name-property")],
        };
    };
    let Some(return_type) = legacy_return_type(payload) else {
        return SemanticResolution::Unresolved {
            reason: "legacy name property return type is unresolved".to_owned(),
            metadata: vec![metadata("semantic-mode", "legacy-name-property")],
        };
    };
    resolved_with_metadata(
        return_type.to_owned(),
        multiplicity,
        vec![
            metadata("semantic-mode", "legacy-name-property"),
            metadata("legacy-name-return-type", return_type),
        ],
    )
}

fn legacy_source_multiplicity(
    multiplicity: Option<DynamicMultiplicity>,
) -> Option<DynamicMultiplicity> {
    // ExprName inherits SimplePropertyExpression.isSingle(), so an explicit
    // Both is a real delegated result; only missing child metadata is unknown.
    multiplicity
}

fn legacy_return_type(payload: &RegisteredExpressionPayload) -> Option<&'static str> {
    legacy_return_type_for_version(payload, crate::runtime::skript_at_least(2, 15))
}

fn legacy_return_type_for_version(
    payload: &RegisteredExpressionPayload,
    modern_version: Option<bool>,
) -> Option<&'static str> {
    match payload.declared_return_type.as_deref() {
        Some(STRING) => Some(STRING),
        // ExprName changed its registration declaration to Object in 2.15.0,
        // but its default instance return type is Adventure Component. The
        // parser can later request its explicit String conversion when needed.
        Some(OBJECT) => {
            if expects_string(payload) {
                Some(STRING)
            } else {
                Some(COMPONENT)
            }
        }
        _ => match modern_version {
            Some(true) => Some(COMPONENT),
            Some(false) => Some(STRING),
            None => None,
        },
    }
}

fn expects_string(payload: &RegisteredExpressionPayload) -> bool {
    payload
        .expected_types
        .iter()
        .any(|expected| expected.class_name == STRING && !expected.plural)
}

#[cfg(test)]
mod tests {
    use super::{
        COMPONENT, OBJECT, STRING, expects_string, legacy_return_type,
        legacy_return_type_for_version, legacy_source_multiplicity,
    };
    use crate::nlaocs::skript_parser_addon::types::{
        DynamicMultiplicity, ExpressionExpectedType, ExpressionPossibleReturnTypesState,
        ExpressionReturnTypeState, MappedSpan, OriginKind, ParseContext, RegisteredExpressionChild,
        RegisteredExpressionPayload, SourceOrigin, TextRange,
    };

    fn payload(
        declared_return_type: Option<&str>,
        expected_string: bool,
    ) -> RegisteredExpressionPayload {
        let range = TextRange { start: 0, end: 1 };
        RegisteredExpressionPayload {
            context: ParseContext {
                syntax_context: 0,
                event_classes: Vec::new(),
                section_stack: Vec::new(),
                values: Vec::new(),
            },
            input: "name".to_owned(),
            definition_id: "expression:test".to_owned(),
            registration_id: "expression:test:registration".to_owned(),
            element_class: "test.ExprName".to_owned(),
            related_property: None,
            pattern_index: 0,
            pattern: "name of %objects%".to_owned(),
            span: MappedSpan {
                virtual_range: range,
                origins: vec![SourceOrigin {
                    original_range: range,
                    kind: OriginKind::Exact,
                    expansion: None,
                }],
            },
            expected_types: if expected_string {
                vec![ExpressionExpectedType {
                    class_name: STRING.to_owned(),
                    plural: false,
                }]
            } else {
                Vec::new()
            },
            declared_return_type: declared_return_type.map(str::to_owned),
            declared_multiplicity: None,
            return_type_state: ExpressionReturnTypeState::Static,
            possible_return_types: Vec::new(),
            possible_return_types_state: ExpressionPossibleReturnTypesState::Unresolved,
            time: 0,
            regex_captures: Vec::new(),
            tags: Vec::new(),
            mark: 0,
            children: vec![RegisteredExpressionChild {
                default_expression: None,
                text: "objects".to_owned(),
                kind: "expression".to_owned(),
                parser_id: None,
                definition_id: None,
                registration_id: None,
                pattern_index: None,
                element_class: None,
                return_type: Some(OBJECT.to_owned()),
                possible_return_types: vec![OBJECT.to_owned()],
                possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
                multiplicity: Some(DynamicMultiplicity::Single),
                public_data: Vec::new(),
                metadata: Vec::new(),
            }],
            parsed_captures: Vec::new(),
            common_child_return_type: None,
            type_options: Vec::new(),
            property_options: Vec::new(),
            selected_property_option_indices: Vec::new(),
            effective_return_type: declared_return_type.map(str::to_owned),
            effective_possible_return_types: Vec::new(),
            effective_possible_return_types_state: ExpressionPossibleReturnTypesState::Unresolved,
            effective_multiplicity: None,
            public_data: Vec::new(),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn old_registration_keeps_string_return_type() {
        let payload = payload(Some(STRING), false);
        assert_eq!(legacy_return_type(&payload), Some(STRING));
    }

    #[test]
    fn modern_object_registration_defaults_to_component() {
        let payload = payload(Some(OBJECT), false);
        assert_eq!(legacy_return_type(&payload), Some(COMPONENT));
    }

    #[test]
    fn modern_object_registration_uses_string_conversion_when_requested() {
        let payload = payload(Some(OBJECT), true);
        assert!(expects_string(&payload));
        assert_eq!(legacy_return_type(&payload), Some(STRING));
    }

    #[test]
    fn missing_registration_and_version_metadata_stays_unresolved() {
        let payload = payload(None, false);
        assert_eq!(legacy_return_type_for_version(&payload, None), None);
    }

    #[test]
    fn legacy_name_delegation_does_not_invent_multiplicity() {
        assert_eq!(legacy_source_multiplicity(None), None);
        assert_eq!(
            legacy_source_multiplicity(Some(DynamicMultiplicity::Both)),
            Some(DynamicMultiplicity::Both)
        );
    }
}

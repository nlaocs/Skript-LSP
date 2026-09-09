use super::{
    SemanticResolution, matches, metadata, metadata_value, register_handler,
    resolved_with_possible_types,
};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, MetadataEntry, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprReversedList";
const HANDLER_ID: &str = "core.expression.expr-reversed-list";
const KEY_PROVIDER: &str = "expression.capability.key-provider";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
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
    resolved_with_possible_types(
        return_type.to_owned(),
        if child.possible_return_types.is_empty() {
            vec![return_type.to_owned()]
        } else {
            child.possible_return_types.clone()
        },
        child.possible_return_types_state,
        DynamicMultiplicity::Multiple,
        key_preserving_metadata(mode, &child.metadata),
    )
}

fn key_preserving_metadata(mode: &str, source: &[MetadataEntry]) -> Vec<MetadataEntry> {
    let mut output = vec![metadata("semantic-mode", mode)];
    if metadata_value(source, KEY_PROVIDER) == Some("true") {
        output.push(metadata(KEY_PROVIDER, "true"));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nlaocs::skript_parser_addon::types::{
        ExpressionPossibleReturnTypesState, ExpressionReturnTypeState, MappedSpan, OriginKind,
        RegisteredExpressionChild, RegisteredExpressionPayload, SourceOrigin, TextRange,
    };

    fn payload(multiplicity: DynamicMultiplicity, keyed: bool) -> RegisteredExpressionPayload {
        let range = TextRange { start: 0, end: 1 };
        RegisteredExpressionPayload {
            context: crate::nlaocs::skript_parser_addon::types::ParseContext {
                syntax_context: 0,
                event_classes: Vec::new(),
                section_stack: Vec::new(),
                values: Vec::new(),
            },
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
            time: 0,
            regex_captures: Vec::new(),
            tags: Vec::new(),
            mark: 0,
            children: vec![RegisteredExpressionChild {
                default_expression: None,
                text: "players".to_owned(),
                kind: "custom".to_owned(),
                parser_id: None,
                definition_id: None,
                registration_id: None,
                pattern_index: None,
                element_class: None,
                return_type: Some("org.bukkit.entity.Player".to_owned()),
                possible_return_types: vec!["org.bukkit.entity.Player".to_owned()],
                possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
                multiplicity: Some(multiplicity),
                public_data: Vec::new(),
                metadata: keyed
                    .then(|| metadata(KEY_PROVIDER, "true"))
                    .into_iter()
                    .collect(),
            }],
            parsed_captures: Vec::new(),
            common_child_return_type: None,
            type_options: Vec::new(),
            property_options: Vec::new(),
            selected_property_option_indices: Vec::new(),
            effective_return_type: None,
            effective_possible_return_types: Vec::new(),
            effective_possible_return_types_state: ExpressionPossibleReturnTypesState::Unresolved,
            effective_multiplicity: None,
            public_data: Vec::new(),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn delegates_the_child_type_and_returns_multiple() {
        let result = resolve(&payload(DynamicMultiplicity::Multiple, false));

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
            resolve(&payload(DynamicMultiplicity::Single, false)),
            Some(SemanticResolution::Reject(_))
        ));
    }

    #[test]
    fn preserves_keys_only_for_a_keyed_source() {
        let Some(SemanticResolution::Resolved { metadata, .. }) =
            resolve(&payload(DynamicMultiplicity::Multiple, true))
        else {
            panic!("keyed reversed list must resolve");
        };
        assert_eq!(metadata_value(&metadata, KEY_PROVIDER), Some("true"));

        let Some(SemanticResolution::Resolved { metadata, .. }) =
            resolve(&payload(DynamicMultiplicity::Multiple, false))
        else {
            panic!("unkeyed reversed list must resolve");
        };
        assert_eq!(metadata_value(&metadata, KEY_PROVIDER), None);
    }
}

use super::{
    SemanticResolution, matches, metadata, metadata_value, register_handler,
    resolved_with_possible_types,
};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, MetadataEntry, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprShuffledList";
const HANDLER_ID: &str = "core.expression.expr-shuffled-list";
const KEY_PROVIDER: &str = "expression.capability.key-provider";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        let Some(return_type) = payload
            .children
            .first()
            .and_then(|child| child.return_type.as_deref())
        else {
            return SemanticResolution::Reject(
                "shuffled list Expression requires a typed source Expression".to_owned(),
            );
        };
        let source = payload.children.first().expect("checked above");
        resolved_with_possible_types(
            return_type.to_owned(),
            if source.possible_return_types.is_empty() {
                vec![return_type.to_owned()]
            } else {
                source.possible_return_types.clone()
            },
            source.possible_return_types_state,
            DynamicMultiplicity::Multiple,
            key_preserving_metadata("shuffled-list", &source.metadata),
        )
    })
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
        RegisteredExpressionChild, SourceOrigin, TextRange,
    };

    fn payload(child_type: Option<&str>, keyed: bool) -> RegisteredExpressionPayload {
        let range = TextRange { start: 0, end: 1 };
        RegisteredExpressionPayload {
            context: crate::nlaocs::skript_parser_addon::types::ParseContext {
                syntax_context: 0,
                event_classes: Vec::new(),
                section_stack: Vec::new(),
                values: Vec::new(),
            },
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
            time: 0,
            regex_captures: Vec::new(),
            tags: Vec::new(),
            mark: 0,
            children: child_type.map_or_else(Vec::new, |return_type| {
                vec![RegisteredExpressionChild {
                    default_expression: None,
                    text: "players".to_owned(),
                    kind: "custom".to_owned(),
                    parser_id: None,
                    definition_id: None,
                    registration_id: None,
                    pattern_index: None,
                    element_class: None,
                    return_type: Some(return_type.to_owned()),
                    possible_return_types: vec![return_type.to_owned()],
                    possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
                    multiplicity: Some(DynamicMultiplicity::Single),
                    public_data: Vec::new(),
                    metadata: keyed
                        .then(|| metadata(KEY_PROVIDER, "true"))
                        .into_iter()
                        .collect(),
                }]
            }),
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
    fn delegates_the_child_type_and_always_returns_multiple() {
        let result = resolve(&payload(Some("org.bukkit.entity.Player"), false));

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
            resolve(&payload(None, false)),
            Some(SemanticResolution::Reject(_))
        ));
    }

    #[test]
    fn preserves_keys_only_for_a_keyed_source() {
        let Some(SemanticResolution::Resolved { metadata, .. }) =
            resolve(&payload(Some("org.bukkit.entity.Player"), true))
        else {
            panic!("keyed shuffled list must resolve");
        };
        assert_eq!(metadata_value(&metadata, KEY_PROVIDER), Some("true"));

        let Some(SemanticResolution::Resolved { metadata, .. }) =
            resolve(&payload(Some("org.bukkit.entity.Player"), false))
        else {
            panic!("unkeyed shuffled list must resolve");
        };
        assert_eq!(metadata_value(&metadata, KEY_PROVIDER), None);
    }
}

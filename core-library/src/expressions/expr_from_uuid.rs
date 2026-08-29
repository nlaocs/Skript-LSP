use super::{SemanticResolution, matches, metadata, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprFromUUID";
const HANDLER_ID: &str = "core.expression.expr-from-uuid";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        let Some(multiplicity) = payload
            .children
            .first()
            .and_then(|child| child.multiplicity)
        else {
            return SemanticResolution::Reject(
                "UUID lookup Expression requires a UUID source".to_owned(),
            );
        };
        let (return_type, mode) = match payload.pattern.as_str() {
            pattern
                if pattern.contains("player")
                    && payload.tags.iter().any(|tag| tag.value == "offline") =>
            {
                ("org.bukkit.OfflinePlayer", "offline-player-from-uuid")
            }
            pattern if pattern.contains("player") => {
                ("org.bukkit.entity.Player", "player-from-uuid")
            }
            pattern if pattern.contains("entit") => {
                ("org.bukkit.entity.Entity", "entity-from-uuid")
            }
            pattern if pattern.contains("world") => ("org.bukkit.World", "world-from-uuid"),
            _ => {
                return SemanticResolution::Reject(
                    "UUID lookup Expression has an unknown pattern".to_owned(),
                );
            }
        };
        SemanticResolution::Resolved {
            return_type: return_type.to_owned(),
            multiplicity,
            metadata: vec![metadata("semantic-mode", mode)],
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nlaocs::skript_parser_addon::types::{
        DynamicMultiplicity, ExpressionPossibleReturnTypesState, ExpressionReturnTypeState,
        MappedSpan, OriginKind, RegisteredExpressionChild, RegisteredExpressionPayload,
        RegisteredExpressionTag, SourceOrigin, TextRange,
    };

    fn payload(pattern: &str, offline: bool) -> RegisteredExpressionPayload {
        let range = TextRange { start: 0, end: 1 };
        RegisteredExpressionPayload {
            input: "uuid".to_owned(),
            definition_id: "expression:test".to_owned(),
            registration_id: "expression:test:0".to_owned(),
            element_class: "ch.njol.skript.expressions.ExprFromUUID".to_owned(),
            related_property: None,
            pattern_index: 99,
            pattern: pattern.to_owned(),
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
            tags: if offline {
                vec![RegisteredExpressionTag {
                    value: "offline".to_owned(),
                    implicit: false,
                }]
            } else {
                Vec::new()
            },
            mark: 0,
            children: vec![RegisteredExpressionChild {
                text: "uuid".to_owned(),
                kind: "literal".to_owned(),
                parser_id: None,
                definition_id: None,
                registration_id: None,
                pattern_index: None,
                element_class: None,
                return_type: Some("java.util.UUID".to_owned()),
                multiplicity: Some(DynamicMultiplicity::Single),
                metadata: Vec::new(),
            }],
            parsed_captures: Vec::new(),
            common_child_return_type: None,
            type_options: Vec::new(),
            property_options: Vec::new(),
            selected_property_option_indices: Vec::new(),
            effective_return_type: None,
            effective_multiplicity: None,
            metadata: Vec::new(),
        }
    }

    fn return_type(pattern: &str, offline: bool) -> String {
        let Some(SemanticResolution::Resolved { return_type, .. }) =
            resolve(&payload(pattern, offline))
        else {
            panic!("UUID pattern must resolve");
        };
        return_type
    }

    #[test]
    fn selects_player_entity_and_world_from_pattern_meaning() {
        assert_eq!(
            return_type("[:offline[ ]]player[s] from %uuids%", false),
            "org.bukkit.entity.Player"
        );
        assert_eq!(
            return_type("entit(y|ies) from %uuids%", false),
            "org.bukkit.entity.Entity"
        );
        assert_eq!(
            return_type("world[s] from %uuids%", false),
            "org.bukkit.World"
        );
    }

    #[test]
    fn offline_tag_selects_offline_player_and_preserves_source_multiplicity() {
        let mut input = payload("[:offline[ ]]player[s] from %uuids%", true);
        input.children[0].multiplicity = Some(DynamicMultiplicity::Multiple);
        let result = resolve(&input);

        assert!(matches!(
            result,
            Some(SemanticResolution::Resolved {
                return_type,
                multiplicity: DynamicMultiplicity::Multiple,
                ..
            }) if return_type == "org.bukkit.OfflinePlayer"
        ));
    }

    #[test]
    fn rejects_an_unknown_pattern() {
        assert!(matches!(
            resolve(&payload("uuid", false)),
            Some(SemanticResolution::Reject(_))
        ));
    }

    #[test]
    fn rejects_a_missing_uuid_source() {
        let mut missing = payload("player from %uuid%", false);
        missing.children.clear();

        assert!(matches!(
            resolve(&missing),
            Some(SemanticResolution::Reject(_))
        ));
    }
}

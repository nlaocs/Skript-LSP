use super::{
    SemanticResolution, matches, metadata, metadata_value, register_handler,
    resolved_with_possible_types,
};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionPossibleReturnTypesState, RegisteredExpressionPayload,
    RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprLastSpawnedEntity";
const HANDLER_ID: &str = "core.expression.expr-last-spawned-entity";
const ENTITY: &str = "org.bukkit.entity.Entity";
const ITEM: &str = "org.bukkit.entity.Item";
const LIGHTNING: &str = "org.bukkit.entity.LightningStrike";
const FIREWORK: &str = "org.bukkit.entity.Firework";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_last_spawned(payload))
}

fn resolve_last_spawned(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let return_type = if let Some(return_type) = fixed_entity_type(payload.mark) {
        return_type.to_owned()
    } else {
        match payload.mark {
            0 | 1 => {
                let Some(entity_data) = payload.children.first() else {
                    return SemanticResolution::Reject(
                        "last spawned entity Expression requires an entity type".to_owned(),
                    );
                };
                if metadata_value(&entity_data.metadata, "entity-plural") == Some("true") {
                    return SemanticResolution::Reject(
                        "last spawned entity Expression requires a singular entity type".to_owned(),
                    );
                }
                metadata_value(&entity_data.metadata, "entity-class")
                    .or_else(|| metadata_value(&entity_data.metadata, "literal-represented-class"))
                    .unwrap_or(ENTITY)
                    .to_owned()
            }
            _ => {
                return SemanticResolution::Reject(
                    "last spawned entity Expression has an unknown source mark".to_owned(),
                );
            }
        }
    };
    resolved_with_possible_types(
        return_type.clone(),
        vec![return_type],
        ExpressionPossibleReturnTypesState::Complete,
        DynamicMultiplicity::Single,
        vec![metadata("semantic-mode", last_spawn_source(payload.mark))],
    )
}

fn last_spawn_source(mark: i32) -> &'static str {
    match mark {
        0 => "last-spawned",
        1 => "last-shot",
        2 => "last-dropped",
        3 => "last-struck-lightning",
        4 => "last-launched-firework",
        _ => "unknown",
    }
}

fn fixed_entity_type(mark: i32) -> Option<&'static str> {
    match mark {
        2 => Some(ITEM),
        3 => Some(LIGHTNING),
        4 => Some(FIREWORK),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{FIREWORK, ITEM, LIGHTNING, last_spawn_source};

    #[test]
    fn fixed_marks_have_the_native_entity_types() {
        assert_eq!(super::fixed_entity_type(2), Some(ITEM));
        assert_eq!(super::fixed_entity_type(3), Some(LIGHTNING));
        assert_eq!(super::fixed_entity_type(4), Some(FIREWORK));
    }

    #[test]
    fn labels_follow_the_effect_that_saved_the_last_entity() {
        assert_eq!(last_spawn_source(0), "last-spawned");
        assert_eq!(last_spawn_source(1), "last-shot");
        assert_eq!(last_spawn_source(99), "unknown");
    }
}

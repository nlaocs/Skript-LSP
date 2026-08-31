use super::{
    SemanticResolution, matches, metadata, register_handler, resolved_with_possible_types,
};
use crate::catalog::TypeRelation;
use crate::nlaocs::skript_parser_addon::types::{
    ExpressionPossibleReturnTypesState, RegisteredExpressionChild, RegisteredExpressionPayload,
    RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprXOf";
const HANDLER_ID: &str = "core.expression.expr-x-of";
const ITEM_STACK: &str = "org.bukkit.inventory.ItemStack";
const ITEM_TYPE: &str = "ch.njol.skript.aliases.ItemType";
const ENTITY_TYPE: &str = "ch.njol.skript.entity.EntityType";
const PARTICLE_EFFECT: &str =
    "org.skriptlang.skript.bukkit.particles.particleeffects.ParticleEffect";
const OBJECT: &str = "java.lang.Object";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_x_of(payload))
}

fn resolve_x_of(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let [amount, source] = payload.children.as_slice() else {
        return SemanticResolution::Reject(
            "x of Expression requires an amount and a source Expression".to_owned(),
        );
    };
    let source_types = possible_types(source);
    let (output_types, relation_unresolved) = supported_output_types(&source_types);
    if output_types.is_empty() {
        if source_types.is_empty()
            || relation_unresolved
            || source.possible_return_types_state != ExpressionPossibleReturnTypesState::Complete
        {
            return SemanticResolution::Unresolved {
                reason: "x of Expression source type is unresolved".to_owned(),
                metadata: vec![metadata("semantic-mode", "x-of")],
            };
        }
        return SemanticResolution::Reject(
            "x of Expression source is not an item, entity, or particle type".to_owned(),
        );
    }

    // The same literal collision exists in Skript's parser: `5 of stone` is
    // also an ItemType form, so ExprXOf must decline that candidate.
    if is_literal(amount) && is_literal(source) && source_can_be_item_literal(&source_types) {
        return SemanticResolution::Reject(
            "literal item syntax takes precedence over x of Expression".to_owned(),
        );
    }

    let Some(return_types) = return_types_for_version(
        source.return_type.as_deref(),
        &output_types,
        crate::runtime::skript_at_least(2, 12),
        if relation_unresolved {
            ExpressionPossibleReturnTypesState::Partial
        } else {
            source.possible_return_types_state
        },
    ) else {
        return SemanticResolution::Unresolved {
            reason: "x of Expression return type depends on unavailable Skript version data"
                .to_owned(),
            metadata: vec![metadata("semantic-mode", "x-of")],
        };
    };
    let Some(multiplicity) = source.multiplicity else {
        return SemanticResolution::Unresolved {
            reason: "x of Expression source multiplicity is unresolved".to_owned(),
            metadata: vec![metadata("semantic-mode", "x-of")],
        };
    };
    resolved_with_possible_types(
        return_types.return_type,
        return_types.possible_return_types,
        return_types.possible_return_types_state,
        multiplicity,
        vec![metadata("semantic-mode", "x-of")],
    )
}

struct XOfReturnTypes {
    return_type: String,
    possible_return_types: Vec<String>,
    possible_return_types_state: ExpressionPossibleReturnTypesState,
}

fn return_types_for_version(
    source_return_type: Option<&str>,
    output_types: &[String],
    dynamic_possible_types: Option<bool>,
    source_possible_types_state: ExpressionPossibleReturnTypesState,
) -> Option<XOfReturnTypes> {
    match dynamic_possible_types {
        Some(false) => {
            let source_return_type = source_return_type?;
            Some(XOfReturnTypes {
                // Before 2.12 ExprXOf inherited getReturnType() from its
                // wrapped expression and had no dynamic possibleReturnTypes.
                return_type: source_return_type.to_owned(),
                possible_return_types: vec![source_return_type.to_owned()],
                possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
            })
        }
        Some(true) => modern_x_of_return_types(output_types, source_possible_types_state),
        None => {
            let source_return_type = source_return_type?;
            let modern = modern_x_of_return_types(output_types, source_possible_types_state)?;
            if modern.return_type == source_return_type
                && modern.possible_return_types == [source_return_type]
                && modern.possible_return_types_state
                    == ExpressionPossibleReturnTypesState::Complete
            {
                // Unknown versions are safe only when the old and new
                // observable return metadata are identical.
                Some(XOfReturnTypes {
                    return_type: source_return_type.to_owned(),
                    possible_return_types: vec![source_return_type.to_owned()],
                    possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
                })
            } else {
                None
            }
        }
    }
}

fn modern_x_of_return_types(
    output_types: &[String],
    possible_types_state: ExpressionPossibleReturnTypesState,
) -> Option<XOfReturnTypes> {
    let return_type = match output_types {
        [] => return None,
        [only] => only.clone(),
        _ => {
            // From 2.12 onward, Skript reports Object when several concrete
            // possibleReturnTypes are available, while retaining that list.
            OBJECT.to_owned()
        }
    };
    Some(XOfReturnTypes {
        return_type,
        possible_return_types: output_types.to_vec(),
        possible_return_types_state: possible_types_state,
    })
}

fn possible_types(child: &RegisteredExpressionChild) -> Vec<String> {
    if child.possible_return_types.is_empty() {
        child.return_type.iter().cloned().collect()
    } else {
        child.possible_return_types.clone()
    }
}

fn supported_output_types(source_types: &[String]) -> (Vec<String>, bool) {
    let mut unresolved = false;
    let output = [ITEM_STACK, ITEM_TYPE, ENTITY_TYPE, PARTICLE_EFFECT]
        .into_iter()
        .filter(|supported| {
            source_types.iter().any(|source| {
                if source == OBJECT || source == supported {
                    return true;
                }
                match crate::catalog::is_class_assignable(source, supported) {
                    Ok(TypeRelation::Compatible) => true,
                    Ok(TypeRelation::Incompatible) => false,
                    Ok(TypeRelation::Unknown) | Err(_) => {
                        unresolved = true;
                        false
                    }
                }
            })
        })
        .map(str::to_owned)
        .collect();
    (output, unresolved)
}

fn source_can_be_item_literal(source_types: &[String]) -> bool {
    source_types
        .iter()
        .any(|source| source == ITEM_STACK || source == ITEM_TYPE)
}

fn is_literal(child: &RegisteredExpressionChild) -> bool {
    child.kind == "literal"
        || child
            .parser_id
            .as_deref()
            .is_some_and(|parser_id| parser_id.starts_with("core.literal."))
}

#[cfg(test)]
mod tests {
    use super::{
        ENTITY_TYPE, ITEM_STACK, ITEM_TYPE, OBJECT, PARTICLE_EFFECT, return_types_for_version,
        supported_output_types,
    };
    use crate::nlaocs::skript_parser_addon::types::ExpressionPossibleReturnTypesState;

    #[test]
    fn preserves_each_supported_x_of_result_type() {
        assert_eq!(
            supported_output_types(&[ITEM_STACK.to_owned(), PARTICLE_EFFECT.to_owned()]).0,
            [ITEM_STACK, PARTICLE_EFFECT]
        );
        assert_eq!(supported_output_types(&[OBJECT.to_owned()]).0.len(), 4);
        assert_eq!(
            supported_output_types(&[ENTITY_TYPE.to_owned()]).0,
            [ENTITY_TYPE]
        );
    }

    #[test]
    fn ignores_a_source_that_x_of_cannot_scale() {
        assert!(
            supported_output_types(&["java.lang.String".to_owned()])
                .0
                .is_empty()
        );
        assert_eq!(
            supported_output_types(&[ITEM_TYPE.to_owned()]).0,
            [ITEM_TYPE]
        );
    }

    #[test]
    fn legacy_x_of_delegates_return_type_to_the_source() {
        let result = return_types_for_version(
            Some("my.Source"),
            &[ITEM_STACK.to_owned(), ENTITY_TYPE.to_owned()],
            Some(false),
            ExpressionPossibleReturnTypesState::Unresolved,
        )
        .expect("legacy return type is source-defined");
        assert_eq!(result.return_type, "my.Source");
        assert_eq!(result.possible_return_types, ["my.Source"]);
        assert_eq!(
            result.possible_return_types_state,
            ExpressionPossibleReturnTypesState::Complete
        );
    }

    #[test]
    fn modern_x_of_reports_object_for_multiple_possible_types() {
        let result = return_types_for_version(
            Some(ITEM_STACK),
            &[ITEM_STACK.to_owned(), ENTITY_TYPE.to_owned()],
            Some(true),
            ExpressionPossibleReturnTypesState::Complete,
        )
        .expect("modern possible types are available");
        assert_eq!(result.return_type, "java.lang.Object");
        assert_eq!(result.possible_return_types, [ITEM_STACK, ENTITY_TYPE]);
    }

    #[test]
    fn unknown_x_of_version_resolves_only_for_identical_metadata() {
        let result = return_types_for_version(
            Some(ITEM_STACK),
            &[ITEM_STACK.to_owned()],
            None,
            ExpressionPossibleReturnTypesState::Complete,
        )
        .expect("single source type has identical old and new metadata");
        assert_eq!(result.return_type, ITEM_STACK);
        assert!(
            return_types_for_version(
                Some(ITEM_STACK),
                &[ITEM_STACK.to_owned(), ENTITY_TYPE.to_owned()],
                None,
                ExpressionPossibleReturnTypesState::Complete,
            )
            .is_none()
        );
    }
}

use super::{
    SemanticResolution, matches, metadata, register_handler, resolved_with_possible_types,
};
use crate::nlaocs::skript_parser_addon::types::{
    RegisteredExpressionChild, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprTypeOf";
const HANDLER_ID: &str = "core.expression.expr-type-of";
const OBJECT: &str = "java.lang.Object";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_type_of(payload))
}

fn resolve_type_of(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let Some(source) = payload.children.first() else {
        return SemanticResolution::Reject(
            "type of Expression requires a source Expression".to_owned(),
        );
    };
    let source_types = possible_types(source);
    let mut possible_return_types: Vec<String> = Vec::new();
    for source_type in &source_types {
        if let Some(return_type) = type_of_return_type(source_type)
            && !possible_return_types
                .iter()
                .any(|known| known.as_str() == return_type)
        {
            possible_return_types.push(return_type.to_owned());
        }
    }
    if possible_return_types.is_empty() {
        if source_types_are_unresolved(&source_types) {
            return unresolved("type of Expression source return type is unresolved");
        }
        return SemanticResolution::Reject(
            "type of Expression source is not a supported Skript type".to_owned(),
        );
    }

    let return_type = if possible_return_types.len() == 1 {
        possible_return_types[0].clone()
    } else {
        match crate::catalog::common_assignable_class(&possible_return_types) {
            Ok(Some(return_type)) if return_type != OBJECT => return_type,
            Ok(Some(_)) | Ok(None) => {
                return unresolved(
                    "type of Expression return types have no known concrete common type",
                );
            }
            Err(reason) => {
                return unresolved(&format!(
                    "type of Expression common return type is unresolved: {reason}"
                ));
            }
        }
    };
    let Some(multiplicity) = source.multiplicity.or(payload.declared_multiplicity) else {
        return unresolved("type of Expression source multiplicity is unresolved");
    };
    resolved_with_possible_types(
        return_type,
        possible_return_types,
        source.possible_return_types_state,
        multiplicity,
        vec![metadata("semantic-mode", "type-of")],
    )
}

fn unresolved(reason: &str) -> SemanticResolution {
    SemanticResolution::Unresolved {
        reason: reason.to_owned(),
        metadata: vec![metadata("semantic-mode", "type-of")],
    }
}

fn source_types_are_unresolved(source_types: &[String]) -> bool {
    source_types.is_empty() || source_types.iter().any(|source_type| source_type == OBJECT)
}

fn possible_types(child: &RegisteredExpressionChild) -> Vec<String> {
    if child.possible_return_types.is_empty() {
        child.return_type.iter().cloned().collect()
    } else {
        child.possible_return_types.clone()
    }
}

fn type_of_return_type(source_type: &str) -> Option<&'static str> {
    match source_type {
        "ch.njol.skript.entity.EntityData" => Some("ch.njol.skript.entity.EntityData"),
        "ch.njol.skript.aliases.ItemType" | "org.bukkit.block.data.BlockData" => {
            Some("ch.njol.skript.aliases.ItemType")
        }
        "org.bukkit.inventory.Inventory" => Some("org.bukkit.event.inventory.InventoryType"),
        "org.bukkit.potion.PotionEffect" => Some("org.bukkit.potion.PotionEffectType"),
        "ch.njol.skript.util.EnchantmentType" => Some("org.bukkit.enchantments.Enchantment"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{OBJECT, source_types_are_unresolved, type_of_return_type};

    #[test]
    fn maps_each_native_type_of_source_to_its_result_type() {
        assert_eq!(
            type_of_return_type("org.bukkit.inventory.Inventory"),
            Some("org.bukkit.event.inventory.InventoryType")
        );
        assert_eq!(
            type_of_return_type("org.bukkit.block.data.BlockData"),
            Some("ch.njol.skript.aliases.ItemType")
        );
        assert_eq!(
            type_of_return_type("ch.njol.skript.util.EnchantmentType"),
            Some("org.bukkit.enchantments.Enchantment")
        );
    }

    #[test]
    fn rejects_unrelated_source_types_without_guessing() {
        assert_eq!(type_of_return_type("java.lang.String"), None);
    }

    #[test]
    fn treats_object_source_types_as_unknown_instead_of_unsupported() {
        assert!(source_types_are_unresolved(&[]));
        assert!(source_types_are_unresolved(&[OBJECT.to_owned()]));
        assert!(!source_types_are_unresolved(&[
            "java.lang.String".to_owned()
        ]));
    }
}

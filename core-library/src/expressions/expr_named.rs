use super::{
    SemanticResolution, matches, metadata, metadata_value, register_handler,
    resolved_with_possible_types,
};
use crate::catalog::TypeRelation;
use crate::nlaocs::skript_parser_addon::types::{
    RegisteredExpressionChild, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprNamed";
const HANDLER_ID: &str = "core.expression.expr-named";
const ITEM_TYPE: &str = "ch.njol.skript.aliases.ItemType";
const INVENTORY_TYPE: &str = "org.bukkit.event.inventory.InventoryType";
const INVENTORY: &str = "org.bukkit.inventory.Inventory";
const OBJECT: &str = "java.lang.Object";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_named(payload))
}

fn resolve_named(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let [source, _name] = payload.children.as_slice() else {
        return SemanticResolution::Reject(
            "named Expression requires a source and a name Expression".to_owned(),
        );
    };

    let source_types = source_types(source);
    if source_types.is_empty() {
        return SemanticResolution::Unresolved {
            reason: "named Expression has no source return type information".to_owned(),
            metadata: vec![metadata("semantic-mode", "named-item-or-inventory")],
        };
    }

    let mut return_types = Vec::new();
    let mut relation_unknown = false;
    for source_type in &source_types {
        match named_return_types(source_type) {
            Ok(types) => return_types.extend(types),
            Err(()) => relation_unknown = true,
        }
    }
    if relation_unknown {
        return SemanticResolution::Unresolved {
            reason: "whether the named source is an item type or inventory type is unresolved"
                .to_owned(),
            metadata: vec![metadata("semantic-mode", "named-item-or-inventory")],
        };
    }

    match inventory_literal_creatability(source) {
        InventoryCreatability::NotApplicable | InventoryCreatability::Known(true) => {}
        InventoryCreatability::Known(false) => {
            return SemanticResolution::Reject(
                "cannot create an inventory of the requested type".to_owned(),
            );
        }
        InventoryCreatability::Unknown => {
            return SemanticResolution::Unresolved {
                reason: "whether the requested inventory type is creatable is unresolved"
                    .to_owned(),
                metadata: vec![metadata("semantic-mode", "named-item-or-inventory")],
            };
        }
    }

    return_types.sort_unstable();
    return_types.dedup();
    let return_type = match return_types.as_slice() {
        [only] => (*only).to_owned(),
        _ => OBJECT.to_owned(),
    };
    let Some(multiplicity) = source.multiplicity else {
        return SemanticResolution::Unresolved {
            reason: "named Expression cannot determine source multiplicity".to_owned(),
            metadata: vec![metadata("semantic-mode", "named-item-or-inventory")],
        };
    };
    let possible_return_types = return_types.into_iter().map(str::to_owned).collect();
    resolved_with_possible_types(
        return_type,
        possible_return_types,
        source.possible_return_types_state,
        multiplicity,
        vec![metadata("semantic-mode", "named-item-or-inventory")],
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InventoryCreatability {
    NotApplicable,
    Known(bool),
    Unknown,
}

fn inventory_literal_creatability(source: &RegisteredExpressionChild) -> InventoryCreatability {
    if source.kind != "literal" {
        return InventoryCreatability::NotApplicable;
    }
    if !source_types(source)
        .iter()
        .any(|source_type| source_type == INVENTORY_TYPE)
    {
        return InventoryCreatability::NotApplicable;
    }
    metadata_value(&source.metadata, "inventory-type-creatable")
        .and_then(parse_bool)
        .map_or(InventoryCreatability::Unknown, InventoryCreatability::Known)
}

fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn source_types(source: &RegisteredExpressionChild) -> Vec<String> {
    if source.possible_return_types.is_empty() {
        source.return_type.iter().cloned().collect()
    } else {
        source.possible_return_types.clone()
    }
}

fn named_return_types(source_type: &str) -> Result<Vec<&'static str>, ()> {
    if source_type == ITEM_TYPE {
        return Ok(vec![ITEM_TYPE]);
    }
    if source_type == INVENTORY_TYPE {
        return Ok(vec![INVENTORY]);
    }

    let mut result = Vec::new();
    let mut unknown = false;
    for (accepted_source, output_type) in [(ITEM_TYPE, ITEM_TYPE), (INVENTORY_TYPE, INVENTORY)] {
        match crate::catalog::is_class_assignable(source_type, accepted_source) {
            Ok(TypeRelation::Compatible) => result.push(output_type),
            Ok(TypeRelation::Incompatible) => {}
            Ok(TypeRelation::Unknown) | Err(_) => unknown = true,
        }
    }
    if unknown { Err(()) } else { Ok(result) }
}

#[cfg(test)]
mod tests {
    use super::{INVENTORY, INVENTORY_TYPE, ITEM_TYPE, named_return_types};

    #[test]
    fn item_and_inventory_sources_have_different_native_outputs() {
        assert_eq!(named_return_types(ITEM_TYPE).unwrap(), [ITEM_TYPE]);
        assert_eq!(named_return_types(INVENTORY_TYPE).unwrap(), [INVENTORY]);
    }
}

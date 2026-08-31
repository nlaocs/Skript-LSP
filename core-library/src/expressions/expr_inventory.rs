use super::{
    SemanticResolution, matches, metadata, register_handler, resolved_with_possible_types,
};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionPossibleReturnTypesState, RegisteredExpressionPayload,
    RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprInventory";
const ITEMS_IN_SUFFIX: &str = ".ExprItemsIn";
const HANDLER_ID: &str = "core.expression.expr-inventory";
const INVENTORY: &str = "org.bukkit.inventory.Inventory";
const SLOT: &str = "ch.njol.skript.util.slot.Slot";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| resolve_inventory(payload))
}

fn resolve_inventory(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let Some(holders) = payload.children.first() else {
        return SemanticResolution::Reject("inventory Expression requires holders".to_owned());
    };
    if holders
        .element_class
        .as_deref()
        .is_some_and(|class| class.ends_with(ITEMS_IN_SUFFIX))
    {
        return SemanticResolution::Reject(
            "inventory of items-in is rejected to avoid the native expression conflict".to_owned(),
        );
    }

    let in_exact_loop = payload
        .context
        .values
        .iter()
        .rfind(|entry| entry.key == crate::loop_context::CONTEXT_KEY)
        .map(|entry| crate::loop_context::decode(Some(&entry.value)))
        .and_then(|frames| frames.last().cloned())
        .zip(expression_source(payload))
        .is_some_and(|(frame, source)| frame.source == source.trim());
    let Some(multiplicity) = inventory_multiplicity(in_exact_loop, holders.multiplicity) else {
        return SemanticResolution::Unresolved {
            reason: "inventory holder multiplicity is unresolved".to_owned(),
            metadata: vec![metadata("semantic-mode", "holder-inventory")],
        };
    };
    let (return_type, mode) = if in_exact_loop {
        (SLOT, "inventory-loop-slots")
    } else {
        (INVENTORY, "holder-inventory")
    };
    resolved_with_possible_types(
        return_type.to_owned(),
        vec![return_type.to_owned()],
        ExpressionPossibleReturnTypesState::Complete,
        multiplicity,
        vec![
            metadata("semantic-mode", mode),
            metadata("inventory-loop-expansion", &in_exact_loop.to_string()),
        ],
    )
}

fn expression_source(payload: &RegisteredExpressionPayload) -> Option<&str> {
    let start = usize::try_from(payload.span.virtual_range.start).ok()?;
    let end = usize::try_from(payload.span.virtual_range.end).ok()?;
    payload.input.get(start..end)
}

fn inventory_multiplicity(
    in_exact_loop: bool,
    holders: Option<DynamicMultiplicity>,
) -> Option<DynamicMultiplicity> {
    if in_exact_loop {
        // ExprInventory expands to ExprItemsIn in a loop. ExprItemsIn is
        // explicitly multiple-valued even when the holder expression is not.
        Some(DynamicMultiplicity::Multiple)
    } else {
        // Outside a loop, ExprInventory is a transparent property of the
        // holder expression and delegates isSingle() to it.
        holders
    }
}

#[cfg(test)]
mod tests {
    use super::inventory_multiplicity;
    use crate::loop_context::{self, LoopFrame};
    use crate::nlaocs::skript_parser_addon::types::DynamicMultiplicity;

    #[test]
    fn inventory_loop_expansion_is_multiple_without_holder_metadata() {
        assert_eq!(
            inventory_multiplicity(true, None),
            Some(DynamicMultiplicity::Multiple)
        );
    }

    #[test]
    fn holder_inventory_requires_delegated_multiplicity() {
        assert_eq!(inventory_multiplicity(false, None), None);
        assert_eq!(
            inventory_multiplicity(false, Some(DynamicMultiplicity::Both)),
            Some(DynamicMultiplicity::Both)
        );
    }

    #[test]
    fn loop_frame_preserves_the_exact_source_expression() {
        let encoded = loop_context::push(
            None,
            LoopFrame {
                source: "inventory of player".to_owned(),
                return_type: "org.bukkit.inventory.Inventory".to_owned(),
                possible_return_types: vec!["org.bukkit.inventory.Inventory".to_owned()],
                keyed: Some(false),
                supports_peeking: Some(true),
            },
        );
        assert_eq!(
            loop_context::decode(Some(&encoded))[0].source,
            "inventory of player"
        );
    }
}

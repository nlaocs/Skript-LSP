use super::{SemanticResolution, matches};
use crate::catalog::TypeRelation;
use crate::nlaocs::skript_parser_addon::types::{
    ParseContext, RegisteredExpressionChild, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const HANDLER_ID: &str = "core.expression.context-validation";
const TARGETS: &[&str] = &[
    ".ExprBreedingFamily",
    ".ExprClicked",
    ".ExprCommandInfo",
    ".ExprFishingApproachAngle",
    ".ExprFishingBiteTime",
    ".ExprFishingHookEntity",
    ".ExprFishingWaitTime",
    ".ExprFurnaceEventItems",
    ".ExprFurnaceSlot",
    ".ExprFurnaceTime",
    ".ExprHanging",
    ".ExprIP",
    ".ExprLoot",
    ".ExprMe",
    ".ExprReadiedArrow",
    ".ExprSpectatorTarget",
];

const ENTITY_BREED: &[&str] = &["org.bukkit.event.entity.EntityBreedEvent"];
const PLAYER_FISH: &[&str] = &["org.bukkit.event.player.PlayerFishEvent"];
const LOOT_GENERATE: &[&str] = &["org.bukkit.event.world.LootGenerateEvent"];
const READY_ARROW: &[&str] = &["com.destroystokyo.paper.event.player.PlayerReadyArrowEvent"];
const EFFECT_COMMAND: &[&str] = &["ch.njol.skript.events.EffectCommandEvent"];
const HANGING_BREAK: &[&str] = &["org.bukkit.event.hanging.HangingBreakEvent"];
const HANGING_EVENTS: &[&str] = &[
    "org.bukkit.event.hanging.HangingBreakEvent",
    "org.bukkit.event.hanging.HangingPlaceEvent",
];
const IP_CONTEXT: &[&str] = &[
    "org.bukkit.event.player.PlayerLoginEvent",
    "org.bukkit.event.server.ServerListPingEvent",
    "com.destroystokyo.paper.event.server.PaperServerListPingEvent",
];
const COMMAND_CONTEXT: &[&str] = &[
    "ch.njol.skript.command.ScriptCommandEvent",
    "org.bukkit.event.player.PlayerCommandPreprocessEvent",
    "org.bukkit.event.server.ServerCommandEvent",
];
const SPECTATOR_CONTEXT: &[&str] = &[
    "com.destroystokyo.paper.event.player.PlayerStartSpectatingEntityEvent",
    "com.destroystokyo.paper.event.player.PlayerStopSpectatingEntityEvent",
];
const INVENTORY_CLICK: &[&str] = &["org.bukkit.event.inventory.InventoryClickEvent"];
const ENCHANT_ITEM: &[&str] = &["org.bukkit.event.enchantment.EnchantItemEvent"];
const PLAYER_INTERACT_BLOCK: &[&str] = &["org.bukkit.event.player.PlayerInteractEvent"];
const PLAYER_INTERACT_ENTITY: &[&str] = &[
    "org.bukkit.event.player.PlayerInteractEntityEvent",
    "org.bukkit.event.player.PlayerInteractAtEntityEvent",
];
const FURNACE_EVENTS: &[&str] = &[
    "org.bukkit.event.inventory.FurnaceBurnEvent",
    "org.bukkit.event.inventory.FurnaceStartSmeltEvent",
    "org.bukkit.event.inventory.FurnaceExtractEvent",
    "org.bukkit.event.inventory.FurnaceSmeltEvent",
];
const FURNACE_SMELT: &[&str] = &["org.bukkit.event.inventory.FurnaceSmeltEvent"];
const FURNACE_EXTRACT: &[&str] = &["org.bukkit.event.inventory.FurnaceExtractEvent"];
const FURNACE_START: &[&str] = &["org.bukkit.event.inventory.FurnaceStartSmeltEvent"];
const FURNACE_BURN: &[&str] = &["org.bukkit.event.inventory.FurnaceBurnEvent"];

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler_targets(handlers, HANDLER_ID, TARGETS);
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    if !matches(payload, HANDLER_ID) {
        return None;
    }
    let required = required_events(payload)?;
    match event_relation(&payload.context, required) {
        Ok(TypeRelation::Compatible) => None,
        Ok(TypeRelation::Incompatible) => Some(SemanticResolution::Reject(format!(
            "this Expression is not available in the current event; expected {}",
            required.join(" or ")
        ))),
        Ok(TypeRelation::Unknown) | Err(_) => Some(SemanticResolution::Unresolved {
            reason: format!(
                "the current event relationship to {} is unavailable",
                required.join(" or ")
            ),
            metadata: vec![super::metadata("semantic-mode", "event-context")],
        }),
    }
}

fn required_events(payload: &RegisteredExpressionPayload) -> Option<&'static [&'static str]> {
    let class = payload.element_class.as_str();
    if class.ends_with(".ExprBreedingFamily") {
        Some(ENTITY_BREED)
    } else if class.ends_with(".ExprFishingApproachAngle")
        || class.ends_with(".ExprFishingBiteTime")
        || class.ends_with(".ExprFishingHookEntity")
        || class.ends_with(".ExprFishingWaitTime")
    {
        Some(PLAYER_FISH)
    } else if class.ends_with(".ExprLoot") {
        Some(LOOT_GENERATE)
    } else if class.ends_with(".ExprReadiedArrow") {
        Some(READY_ARROW)
    } else if class.ends_with(".ExprMe") {
        Some(EFFECT_COMMAND)
    } else if class.ends_with(".ExprHanging") {
        Some(hanging_required_events(has_tag(payload, "remover")))
    } else if class.ends_with(".ExprIP") {
        (payload.pattern_index == 2).then_some(IP_CONTEXT)
    } else if class.ends_with(".ExprCommandInfo") {
        payload.children.is_empty().then_some(COMMAND_CONTEXT)
    } else if class.ends_with(".ExprSpectatorTarget") {
        payload.children.is_empty().then_some(SPECTATOR_CONTEXT)
    } else if class.ends_with(".ExprClicked") {
        clicked_events(payload.mark, payload.children.first())
    } else if class.ends_with(".ExprFurnaceEventItems") {
        match payload.pattern_index {
            0 => Some(FURNACE_SMELT),
            1 => Some(FURNACE_EXTRACT),
            2 => Some(FURNACE_START),
            3 => Some(FURNACE_BURN),
            _ => None,
        }
    } else if class.ends_with(".ExprFurnaceSlot") || class.ends_with(".ExprFurnaceTime") {
        payload.children.is_empty().then_some(FURNACE_EVENTS)
    } else {
        None
    }
}

fn hanging_required_events(remover: bool) -> &'static [&'static str] {
    if remover {
        HANGING_BREAK
    } else {
        HANGING_EVENTS
    }
}

fn clicked_events(
    mark: i32,
    source: Option<&RegisteredExpressionChild>,
) -> Option<&'static [&'static str]> {
    match mark {
        1 => Some(ENCHANT_ITEM),
        2 if source.is_some_and(|child| {
            child.return_type.as_deref() == Some("ch.njol.skript.entity.EntityData")
        }) =>
        {
            Some(PLAYER_INTERACT_ENTITY)
        }
        2 => Some(PLAYER_INTERACT_BLOCK),
        3..=6 => Some(INVENTORY_CLICK),
        _ => None,
    }
}

fn has_tag(payload: &RegisteredExpressionPayload, tag: &str) -> bool {
    payload.tags.iter().any(|entry| entry.value == tag)
}

fn event_relation(context: &ParseContext, required: &[&str]) -> Result<TypeRelation, String> {
    if context.event_classes.is_empty() {
        return Ok(TypeRelation::Incompatible);
    }
    let mut unknown = false;
    for current in &context.event_classes {
        for required in required {
            if current == required {
                return Ok(TypeRelation::Compatible);
            }
            match crate::catalog::is_class_assignable(current, required)? {
                TypeRelation::Compatible => return Ok(TypeRelation::Compatible),
                TypeRelation::Incompatible => {}
                TypeRelation::Unknown => unknown = true,
            }
        }
    }
    Ok(if unknown {
        TypeRelation::Unknown
    } else {
        TypeRelation::Incompatible
    })
}

#[cfg(test)]
mod tests {
    use super::{FURNACE_BURN, FURNACE_SMELT, clicked_events, hanging_required_events};

    #[test]
    fn clicked_mark_selects_the_native_event_family() {
        assert_eq!(
            clicked_events(1, None).unwrap()[0],
            "org.bukkit.event.enchantment.EnchantItemEvent"
        );
        assert_eq!(
            clicked_events(2, None).unwrap()[0],
            "org.bukkit.event.player.PlayerInteractEvent"
        );
        assert_eq!(
            clicked_events(6, None).unwrap()[0],
            "org.bukkit.event.inventory.InventoryClickEvent"
        );
    }

    #[test]
    fn furnace_event_patterns_keep_the_native_order() {
        assert_eq!(
            FURNACE_SMELT[0],
            "org.bukkit.event.inventory.FurnaceSmeltEvent"
        );
        assert_eq!(
            FURNACE_BURN[0],
            "org.bukkit.event.inventory.FurnaceBurnEvent"
        );
    }

    #[test]
    fn hanging_expression_uses_break_only_for_remover() {
        assert_eq!(
            hanging_required_events(true),
            &["org.bukkit.event.hanging.HangingBreakEvent"]
        );
        assert_eq!(
            hanging_required_events(false),
            &[
                "org.bukkit.event.hanging.HangingBreakEvent",
                "org.bukkit.event.hanging.HangingPlaceEvent",
            ]
        );
    }
}

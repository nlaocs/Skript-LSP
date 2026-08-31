use crate::catalog::TypeRelation;
use crate::nlaocs::skript_parser_addon::types::{
    ConditionPayload, HookOutput, RegisteredSyntaxHandler,
};

const HANDLER_ID: &str = "core.condition.event-context";
const TARGETS: &[&str] = &[
    ".CondElytraBoostConsume",
    ".CondFishingLure",
    ".CondIncendiary",
    ".CondLeashWillDrop",
    ".CondRespawnLocation",
    ".CondResourcePack",
    ".CondWillHatch",
];

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler_targets(handlers, HANDLER_ID, TARGETS);
}

pub(super) fn resolve(mut payload: ConditionPayload) -> Option<HookOutput> {
    if !super::matches(&payload, HANDLER_ID) {
        return None;
    }
    let class = payload.candidate.element_class.as_deref()?;
    let Some((event_class, description)) = rule(class, payload.candidate.pattern_index) else {
        return Some(super::accept(payload));
    };
    super::annotate(&mut payload, "semantic-mode", "event-context");
    let span = payload.candidate.span.clone();
    Some(match super::event_relation(&payload.context, event_class) {
        Ok(TypeRelation::Compatible) => super::accept(payload),
        Ok(TypeRelation::Incompatible) => super::reject_with(
            format!("this Condition can only be used in {description}"),
            "core.condition.invalid-event-context",
            span,
        ),
        Ok(TypeRelation::Unknown) | Err(_) => {
            super::mark_unresolved(&mut payload, "core.condition.unresolved-event-context");
            let mut output = super::accept(payload);
            output.effects.diagnostics.push(super::warning(
                "core.condition.unresolved-event-context",
                format!("the current event is unavailable, so the {description} restriction could not be validated"),
                span,
            ));
            output
        }
    })
}

fn rule(class: &str, pattern_index: u64) -> Option<(&'static str, &'static str)> {
    if class.ends_with(".CondElytraBoostConsume") {
        Some((
            "com.destroystokyo.paper.event.player.PlayerElytraBoostEvent",
            "an elytra boost event",
        ))
    } else if class.ends_with(".CondFishingLure") {
        Some(("org.bukkit.event.player.PlayerFishEvent", "a fishing event"))
    } else if class.ends_with(".CondIncendiary") {
        (pattern_index == 2).then_some((
            "org.bukkit.event.entity.ExplosionPrimeEvent",
            "an explosion prime event",
        ))
    } else if class.ends_with(".CondLeashWillDrop") {
        Some((
            "org.bukkit.event.entity.EntityUnleashEvent",
            "an entity unleash event",
        ))
    } else if class.ends_with(".CondRespawnLocation") {
        Some((
            "org.bukkit.event.player.PlayerRespawnEvent",
            "a player respawn event",
        ))
    } else if class.ends_with(".CondResourcePack") {
        Some((
            "org.bukkit.event.player.PlayerResourcePackStatusEvent",
            "a resource pack status event",
        ))
    } else if class.ends_with(".CondWillHatch") {
        Some((
            "org.bukkit.event.player.PlayerEggThrowEvent",
            "a player egg throw event",
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::rule;

    #[test]
    fn only_the_event_incendiary_pattern_requires_an_event() {
        assert!(rule("ch.njol.skript.conditions.CondIncendiary", 0).is_none());
        assert!(rule("ch.njol.skript.conditions.CondIncendiary", 2).is_some());
    }
}

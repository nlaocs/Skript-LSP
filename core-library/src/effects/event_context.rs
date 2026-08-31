use super::{ContractVerdict, event_relation};
use crate::catalog::TypeRelation;
use crate::nlaocs::skript_parser_addon::types::{
    EffectCandidate, EffectPayload, HookOutput, RegisteredSyntaxHandler,
};

const HANDLER_ID: &str = "core.effect.event-context";
const TARGETS: &[&str] = &[
    ".EffCancelCooldown",
    ".EffCancelDrops",
    ".EffDropLeash",
    ".EffElytraBoostConsume",
    ".EffHidePlayerFromServerList",
    ".EffIncendiary",
    ".EffKeepInventory",
    ".EffMakeEggHatch",
    ".EffFishingLure",
    ".EffPlayerInfoVisibility",
    ".EffPullHookedEntity",
    ".EffTeleport",
];

struct Rule {
    event_class: Option<&'static str>,
    forbidden: bool,
    must_be_immediate: bool,
    description: &'static str,
}

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler_targets(handlers, HANDLER_ID, TARGETS);
}

pub(super) fn resolve(mut payload: EffectPayload) -> Option<HookOutput> {
    if !super::matches(&payload, HANDLER_ID) {
        return None;
    }
    let candidate = payload.candidate.as_ref()?;
    let class = candidate.element_class.as_deref()?;
    let teleport_delay_state = teleport_delay_state(class, candidate);
    let syntax_context = payload.context.syntax_context;
    let Some(rule) = rule(class, candidate.pattern_index) else {
        return Some(super::accept(payload));
    };
    super::annotate(&mut payload, "semantic-mode", "event-context");

    match event_verdict(&payload, &rule) {
        Ok(ContractVerdict::Rejected) => {
            let span = payload.candidate.as_ref()?.span.clone();
            return Some(super::reject_with(
                format!(
                    "this Effect cannot be used in the current context; expected {}",
                    rule.description
                ),
                "core.effect.invalid-event-context",
                span,
            ));
        }
        Ok(ContractVerdict::Unresolved) | Err(_) => {
            let span = payload.candidate.as_ref()?.span.clone();
            super::mark_unresolved(&mut payload, "core.effect.unresolved-event-context");
            let mut output = super::continue_with_diagnostics(
                payload,
                vec![super::warning(
                    "core.effect.unresolved-event-context",
                    format!(
                        "the current event is unavailable, so the {} restriction could not be validated",
                        rule.description
                    ),
                    span,
                )],
            );
            apply_teleport_delay_state(&mut output, teleport_delay_state, syntax_context);
            return Some(output);
        }
        Ok(ContractVerdict::Accepted) => {}
    }

    if rule.must_be_immediate {
        match super::context_bool(&payload.context, super::DELAY_STATE_KEY) {
            Some(true) => {
                let span = payload.candidate.as_ref()?.span.clone();
                return Some(super::reject_with(
                    "this event can no longer be changed after the trigger has been delayed",
                    "core.effect.delayed-event-change",
                    span,
                ));
            }
            None => {
                let span = payload.candidate.as_ref()?.span.clone();
                super::mark_unresolved(&mut payload, "core.effect.unresolved-delay-state");
                return Some(super::continue_with_diagnostics(
                    payload,
                    vec![super::warning(
                        "core.effect.unresolved-delay-state",
                        "the delay state is unavailable, so the event timing restriction could not be validated",
                        span,
                    )],
                ));
            }
            Some(false) => {}
        }
    }
    let mut output = super::accept(payload);
    apply_teleport_delay_state(&mut output, teleport_delay_state, syntax_context);
    Some(output)
}

fn apply_teleport_delay_state(
    output: &mut HookOutput,
    delay_state: Option<&'static str>,
    syntax_context: u64,
) {
    if let Some(delay_state) = delay_state {
        super::add_context_update(
            output,
            syntax_context,
            super::DELAY_STATE_KEY,
            Some(delay_state.as_bytes()),
        );
    }
}

fn teleport_delay_state(class: &str, candidate: &EffectCandidate) -> Option<&'static str> {
    if !class.ends_with(".EffTeleport") || is_force(candidate) {
        return None;
    }

    // EffTeleport became unconditionally async in 2.16. In older releases
    // PaperLib only selected the async path on Paper-like servers.
    teleport_delay_state_for(
        false,
        crate::runtime::skript_at_least(2, 16),
        legacy_can_run_async(),
    )
}

fn teleport_delay_state_for(
    force: bool,
    modern_generation: Option<bool>,
    legacy_can_run_async: Option<bool>,
) -> Option<&'static str> {
    if force {
        return None;
    }
    match modern_generation {
        Some(true) | None => Some("unknown"),
        Some(false) => match legacy_can_run_async {
            Some(false) => None,
            Some(true) | None => Some("unknown"),
        },
    }
}

fn legacy_can_run_async() -> Option<bool> {
    // PaperLib 1.0.8 selects PaperEnvironment from either of these runtime classes.
    // A missing ClassHierarchy node does not prove that a runtime class is absent,
    // so the negative case deliberately remains unresolved.
    [
        "com.destroystokyo.paper.PaperConfig",
        "io.papermc.paper.configuration.Configuration",
    ]
    .into_iter()
    .any(|class_name| crate::catalog::class_known(class_name) == Ok(true))
    .then_some(true)
}

fn is_force(candidate: &EffectCandidate) -> bool {
    is_force_marker(
        candidate.tags.iter().any(|tag| tag.value == "force"),
        candidate.mark,
        &candidate.pattern,
    )
}

fn is_force_marker(has_force_tag: bool, mark: i32, pattern: &str) -> bool {
    has_force_tag || (mark == 1 && pattern.contains("1\u{00a6}force"))
}

fn event_verdict(payload: &EffectPayload, rule: &Rule) -> Result<ContractVerdict, String> {
    let Some(event_class) = rule.event_class else {
        return Ok(ContractVerdict::Accepted);
    };
    Ok(
        match (
            event_relation(&payload.context, event_class)?,
            rule.forbidden,
        ) {
            (TypeRelation::Compatible, false) | (TypeRelation::Incompatible, true) => {
                ContractVerdict::Accepted
            }
            (TypeRelation::Incompatible, false) | (TypeRelation::Compatible, true) => {
                ContractVerdict::Rejected
            }
            (TypeRelation::Unknown, _) => ContractVerdict::Unresolved,
        },
    )
}

fn rule(class: &str, pattern_index: u64) -> Option<Rule> {
    let required = |event_class, must_be_immediate, description| Rule {
        event_class: Some(event_class),
        forbidden: false,
        must_be_immediate,
        description,
    };
    Some(if class.ends_with(".EffCancelCooldown") {
        required(
            "ch.njol.skript.command.ScriptCommandEvent",
            false,
            "script command",
        )
    } else if class.ends_with(".EffCancelDrops") {
        // EventRestrictedSyntax supplies the accepted event set; this hook adds
        // the init-time delayed-event guard that CommonSyntaxData cannot express.
        return Some(Rule {
            event_class: None,
            forbidden: false,
            must_be_immediate: true,
            description: "drop event",
        });
    } else if class.ends_with(".EffDropLeash") {
        required(
            "org.bukkit.event.entity.EntityUnleashEvent",
            false,
            "entity unleash event",
        )
    } else if class.ends_with(".EffElytraBoostConsume") {
        required(
            "com.destroystokyo.paper.event.player.PlayerElytraBoostEvent",
            false,
            "elytra boost event",
        )
    } else if class.ends_with(".EffHidePlayerFromServerList") {
        required(
            "org.bukkit.event.server.ServerListPingEvent",
            true,
            "server list ping event",
        )
    } else if class.ends_with(".EffIncendiary") {
        if pattern_index != 2 {
            return None;
        }
        required(
            "org.bukkit.event.entity.ExplosionPrimeEvent",
            false,
            "explosion prime event",
        )
    } else if class.ends_with(".EffKeepInventory") {
        required(
            "org.bukkit.event.entity.EntityDeathEvent",
            true,
            "entity death event",
        )
    } else if class.ends_with(".EffMakeEggHatch") {
        required(
            "org.bukkit.event.player.PlayerEggThrowEvent",
            false,
            "player egg throw event",
        )
    } else if class.ends_with(".EffFishingLure") || class.ends_with(".EffPullHookedEntity") {
        required(
            "org.bukkit.event.player.PlayerFishEvent",
            false,
            "fishing event",
        )
    } else if class.ends_with(".EffPlayerInfoVisibility") {
        required(
            "com.destroystokyo.paper.event.server.PaperServerListPingEvent",
            true,
            "Paper server list ping event",
        )
    } else if class.ends_with(".EffTeleport") {
        Rule {
            event_class: Some("ch.njol.skript.sections.EffSecSpawn$SpawnEvent"),
            forbidden: true,
            must_be_immediate: false,
            description: "non-spawn event",
        }
    } else {
        return None;
    })
}

#[cfg(test)]
mod tests {
    use super::{is_force_marker, rule, teleport_delay_state_for};

    #[test]
    fn only_the_event_form_of_incendiary_is_restricted() {
        assert!(rule("ch.njol.skript.effects.EffIncendiary", 0).is_none());
        assert_eq!(
            rule("ch.njol.skript.effects.EffIncendiary", 2).and_then(|rule| rule.event_class),
            Some("org.bukkit.event.entity.ExplosionPrimeEvent")
        );
    }

    #[test]
    fn teleport_uses_a_forbidden_event_rule() {
        assert!(rule("ch.njol.skript.effects.EffTeleport", 0).is_some_and(|rule| rule.forbidden));
    }

    #[test]
    fn teleport_delay_follows_force_and_runtime_generation() {
        assert_eq!(teleport_delay_state_for(true, Some(true), Some(true)), None);
        assert_eq!(
            teleport_delay_state_for(false, Some(true), None),
            Some("unknown")
        );
        assert_eq!(
            teleport_delay_state_for(false, Some(false), Some(true)),
            Some("unknown")
        );
        assert_eq!(
            teleport_delay_state_for(false, Some(false), Some(false)),
            None
        );
        assert_eq!(teleport_delay_state_for(false, None, None), Some("unknown"));
    }

    #[test]
    fn legacy_force_marker_uses_the_pattern_separator() {
        assert!(is_force_marker(false, 1, "[(1\u{00a6}force)] teleport..."));
        assert!(!is_force_marker(false, 0, "[(1\u{00a6}force)] teleport..."));
        assert!(is_force_marker(true, 0, "teleport..."));
    }
}

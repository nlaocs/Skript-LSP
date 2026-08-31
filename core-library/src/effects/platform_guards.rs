use crate::nlaocs::skript_parser_addon::types::{
    EffectPayload, HookOutput, RegisteredSyntaxHandler,
};

const HANDLER_ID: &str = "core.effect.platform-guards";
const TARGETS: &[&str] = &[
    ".EffConnect",
    ".EffExplodeCreeper",
    ".EffLoadServerIcon",
    ".EffPlayerInfoVisibility",
    ".EffSwingHand",
];
const PAPER_PING_EVENT: &str = "com.destroystokyo.paper.event.server.PaperServerListPingEvent";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Availability {
    Available,
    Unavailable,
    Unresolved,
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
    let syntax_context = payload.context.syntax_context;
    let loads_server_icon = loads_server_icon_delay(class);
    let Some((availability, requirement)) = availability(class, candidate.pattern_index) else {
        let mut output = super::accept(payload);
        apply_load_server_icon_delay(&mut output, loads_server_icon, syntax_context);
        return Some(output);
    };
    super::annotate(&mut payload, "semantic-mode", "platform-guard");
    let span = payload.candidate.as_ref()?.span.clone();
    Some(match availability {
        Availability::Available => {
            let mut output = super::accept(payload);
            apply_load_server_icon_delay(&mut output, loads_server_icon, syntax_context);
            output
        }
        Availability::Unavailable => super::reject_with(
            format!("this Effect requires {requirement}"),
            "core.effect.unsupported-platform",
            span,
        ),
        Availability::Unresolved => {
            super::mark_unresolved(&mut payload, "core.effect.unresolved-platform");
            let mut output = super::continue_with_diagnostics(
                payload,
                vec![super::warning(
                    "core.effect.unresolved-platform",
                    format!("the runtime profile cannot confirm the required {requirement}"),
                    span,
                )],
            );
            apply_load_server_icon_delay(&mut output, loads_server_icon, syntax_context);
            output
        }
    })
}

fn availability(class: &str, pattern_index: u64) -> Option<(Availability, &'static str)> {
    if class.ends_with(".EffConnect") && pattern_index == 2 {
        Some((
            declared_method(
                "org.bukkit.entity.Player",
                "transfer",
                &["java.lang.String", "int"],
            ),
            "Player.transfer(String, int)",
        ))
    } else if class.ends_with(".EffExplodeCreeper") && pattern_index == 4 {
        Some((
            declared_method("org.bukkit.entity.Creeper", "setIgnited", &["boolean"]),
            "Creeper.setIgnited(boolean)",
        ))
    } else if class.ends_with(".EffPlayerInfoVisibility") {
        Some((
            match crate::catalog::class_known(PAPER_PING_EVENT) {
                Ok(true) => Availability::Available,
                Ok(false) | Err(_) => Availability::Unresolved,
            },
            "Paper's server list ping API",
        ))
    } else if class.ends_with(".EffSwingHand") {
        Some((
            declared_method("org.bukkit.entity.LivingEntity", "swingMainHand", &[]),
            "LivingEntity.swingMainHand()",
        ))
    } else {
        None
    }
}

fn declared_method(class: &str, method: &str, parameters: &[&str]) -> Availability {
    match crate::catalog::declared_method_exists(class, method, parameters, None) {
        Ok(value) => from_probe(value),
        Err(_) => Availability::Unresolved,
    }
}

fn from_probe(value: Option<bool>) -> Availability {
    match value {
        Some(true) => Availability::Available,
        Some(false) => Availability::Unavailable,
        None => Availability::Unresolved,
    }
}

fn loads_server_icon_delay(class: &str) -> bool {
    class.ends_with(".EffLoadServerIcon")
}

fn apply_load_server_icon_delay(
    output: &mut HookOutput,
    loads_server_icon: bool,
    syntax_context: u64,
) {
    if loads_server_icon {
        super::add_context_update(
            output,
            syntax_context,
            super::DELAY_STATE_KEY,
            Some(b"true"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{Availability, availability, from_probe, loads_server_icon_delay};

    #[test]
    fn platform_checks_preserve_unknown_runtime_profiles() {
        assert_eq!(from_probe(None), Availability::Unresolved);
        assert_eq!(from_probe(Some(true)), Availability::Available);
        assert_eq!(from_probe(Some(false)), Availability::Unavailable);
    }

    #[test]
    fn load_server_icon_delays_without_requiring_paper() {
        assert!(loads_server_icon_delay(
            "ch.njol.skript.effects.EffLoadServerIcon"
        ));
        assert!(!loads_server_icon_delay(
            "ch.njol.skript.effects.EffPlayerInfoVisibility"
        ));
        let load = availability("ch.njol.skript.effects.EffLoadServerIcon", 0);
        let visibility = availability("ch.njol.skript.effects.EffPlayerInfoVisibility", 0);
        assert_eq!(load, None);
        assert!(
            visibility
                .is_some_and(|(_, requirement)| requirement == "Paper's server list ping API")
        );
    }
}

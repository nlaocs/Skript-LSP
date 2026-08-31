use crate::catalog::TypeRelation;
use crate::nlaocs::skript_parser_addon::types::{
    Diagnostic, EffectPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".EffRespawn";
const HANDLER_ID: &str = "core.effect.eff-respawn";
const PLAYER_RESPAWN_EVENT: &str = "org.bukkit.event.player.PlayerRespawnEvent";
const ENTITY_DEATH_EVENT: &str = "org.bukkit.event.entity.EntityDeathEvent";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX);
}

pub(super) fn resolve(
    mut payload: EffectPayload,
) -> Option<crate::nlaocs::skript_parser_addon::types::HookOutput> {
    if !super::matches(&payload, HANDLER_ID) {
        return None;
    }
    let span = payload.candidate.as_ref()?.span.clone();
    super::annotate(&mut payload, "semantic-mode", "force-respawn");
    let mut diagnostics = Vec::<Diagnostic>::new();

    match super::event_relation(&payload.context, PLAYER_RESPAWN_EVENT) {
        Ok(TypeRelation::Compatible) => {
            return Some(super::reject_with(
                "respawning a player from a respawn Event is not possible",
                "core.eff-respawn.respawn-event",
                span,
            ));
        }
        Ok(TypeRelation::Unknown) | Err(_) => unresolved(
            &mut payload,
            &mut diagnostics,
            "core.eff-respawn.unresolved-event-type",
            "the current Event classes are insufficient to exclude a respawn Event",
            span.clone(),
        ),
        Ok(TypeRelation::Incompatible) => {}
    }

    match super::event_relation(&payload.context, ENTITY_DEATH_EVENT) {
        Ok(TypeRelation::Compatible) => {
            match super::context_bool(&payload.context, super::DELAY_STATE_KEY) {
                Some(false) => super::annotate(&mut payload, "respawn-force-delay", "true"),
                Some(true) => super::annotate(&mut payload, "respawn-force-delay", "false"),
                None => unresolved(
                    &mut payload,
                    &mut diagnostics,
                    "core.eff-respawn.unresolved-delay-state",
                    "the parser did not expose whether the death Event has already been delayed",
                    span,
                ),
            }
        }
        Ok(TypeRelation::Unknown) | Err(_) => unresolved(
            &mut payload,
            &mut diagnostics,
            "core.eff-respawn.unresolved-death-event",
            "the current Event classes are insufficient to determine whether a one-tick delay is required",
            span,
        ),
        Ok(TypeRelation::Incompatible) => {
            super::annotate(&mut payload, "respawn-force-delay", "false")
        }
    }
    Some(super::continue_with_diagnostics(payload, diagnostics))
}

fn unresolved(
    payload: &mut EffectPayload,
    diagnostics: &mut Vec<Diagnostic>,
    code: &str,
    message: &str,
    span: crate::nlaocs::skript_parser_addon::types::MappedSpan,
) {
    super::mark_unresolved(payload, code);
    diagnostics.push(super::warning(code, message, span));
}

#[cfg(test)]
mod tests {
    #[test]
    fn death_events_only_force_a_delay_when_not_already_delayed() {
        let force_delay =
            |death_event: bool, delayed: Option<bool>| death_event && delayed == Some(false);
        assert!(force_delay(true, Some(false)));
        assert!(!force_delay(true, Some(true)));
        assert!(!force_delay(false, Some(false)));
    }
}

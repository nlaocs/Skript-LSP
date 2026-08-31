use crate::catalog::TypeRelation;
use crate::nlaocs::skript_parser_addon::types::{
    ConditionPayload, HookOutput, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".CondIsPressingKey";
const HANDLER_ID: &str = "core.condition.cond-is-pressing-key";
const PLAYER_INPUT_EVENT: &str = "org.bukkit.event.player.PlayerInputEvent";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX);
}

pub(super) fn resolve(mut payload: ConditionPayload) -> Option<HookOutput> {
    if !super::matches(&payload, HANDLER_ID) {
        return None;
    }
    let past = payload.candidate.pattern_index > 1;
    let negated = matches!(payload.candidate.pattern_index, 1 | 3);
    super::annotate(
        &mut payload,
        "input-state",
        if past { "past" } else { "current" },
    );
    super::annotate(
        &mut payload,
        "negated",
        if negated { "true" } else { "false" },
    );
    let span = payload.candidate.span.clone();
    let mut output = super::accept(payload.clone());
    if !past {
        return Some(output);
    }
    match super::event_relation(&payload.context, PLAYER_INPUT_EVENT) {
        Ok(TypeRelation::Incompatible) => output.effects.diagnostics.push(super::warning(
            "core.condition.past-input-outside-event",
            "checking a player's past input outside a player input event has no effect",
            span,
        )),
        Ok(TypeRelation::Compatible) => {
            if delay_state(&payload) != Some(false) {
                output.effects.diagnostics.push(super::warning(
                    "core.condition.past-input-after-delay",
                    "checking a player's past input after the event has passed has no effect",
                    span,
                ));
            }
        }
        Ok(TypeRelation::Unknown) | Err(_) => {
            super::mark_unresolved(&mut payload, "core.condition.unresolved-input-event");
            output = super::accept(payload);
            output.effects.diagnostics.push(super::warning(
                "core.condition.unresolved-input-event",
                "the current event is unavailable, so past input semantics could not be validated",
                span,
            ));
        }
    }
    Some(output)
}

fn delay_state(payload: &ConditionPayload) -> Option<bool> {
    payload
        .context
        .values
        .iter()
        .rfind(|entry| entry.key == "parser.delay-state")
        .and_then(|entry| delay_value(&entry.value))
}

fn delay_value(value: &str) -> Option<bool> {
    match value {
        "false" => Some(false),
        "true" | "unknown" => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::delay_value;

    #[test]
    fn unknown_delay_is_conservatively_treated_as_delayed() {
        assert_eq!(delay_value("unknown"), Some(true));
        assert_eq!(delay_value("false"), Some(false));
    }
}

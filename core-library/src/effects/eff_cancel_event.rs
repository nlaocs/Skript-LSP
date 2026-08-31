use crate::catalog::TypeRelation;
use crate::nlaocs::skript_parser_addon::types::{
    Diagnostic, EffectPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".EffCancelEvent";
const HANDLER_ID: &str = "core.effect.eff-cancel-event";
const CANCELLABLE: &str = "org.bukkit.event.Cancellable";
const BLOCK_CAN_BUILD_EVENT: &str = "org.bukkit.event.block.BlockCanBuildEvent";
const TOGGLE_SWIM_EVENT: &str = "org.bukkit.event.entity.EntityToggleSwimEvent";
const PLAYER_LOGIN_EVENT: &str = "org.bukkit.event.player.PlayerLoginEvent";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX);
}

pub(super) fn resolve(
    mut payload: EffectPayload,
) -> Option<crate::nlaocs::skript_parser_addon::types::HookOutput> {
    if !super::matches(&payload, HANDLER_ID) {
        return None;
    }
    let candidate = payload.candidate.as_ref()?;
    let cancel = match candidate.pattern_index {
        0 => true,
        1 => false,
        _ => {
            return Some(super::reject_with(
                "cancel Event Effect has an unknown pattern index",
                "core.eff-cancel-event.unknown-pattern",
                candidate.span.clone(),
            ));
        }
    };
    let span = candidate.span.clone();
    super::annotate(
        &mut payload,
        "semantic-mode",
        if cancel {
            "cancel-event"
        } else {
            "uncancel-event"
        },
    );

    if payload.context.event_classes.is_empty() {
        return Some(super::reject_with(
            "the cancel Event Effect can only be used inside an Event",
            "core.eff-cancel-event.outside-event",
            span,
        ));
    }

    let mut diagnostics = Vec::<Diagnostic>::new();
    match super::context_bool(&payload.context, super::DELAY_STATE_KEY) {
        Some(true) => {
            return Some(super::reject_with(
                "an Event cannot be cancelled after it has already passed",
                "core.eff-cancel-event.after-delay",
                span,
            ));
        }
        Some(false) => {}
        None => unresolved(
            &mut payload,
            &mut diagnostics,
            "core.eff-cancel-event.unresolved-delay-state",
            "the parser did not expose whether this Effect runs after a delay",
            span.clone(),
        ),
    }

    if cancel
        && matches!(
            super::event_relation(&payload.context, TOGGLE_SWIM_EVENT),
            Ok(TypeRelation::Compatible)
        )
    {
        match crate::runtime::skript_at_least(2, 9) {
            Some(true) => {
                return Some(super::reject_with(
                    "cancelling a toggle swim Event has no effect",
                    "core.eff-cancel-event.toggle-swim",
                    span,
                ));
            }
            Some(false) => {}
            None => unresolved(
                &mut payload,
                &mut diagnostics,
                "core.eff-cancel-event.unresolved-toggle-swim-generation",
                "the Skript version is unavailable, so the toggle swim cancellation rule could not be selected",
                span.clone(),
            ),
        }
    }

    let cancellable = super::event_relation(&payload.context, CANCELLABLE);
    let block_can_build = super::event_relation(&payload.context, BLOCK_CAN_BUILD_EVENT);
    if matches!(cancellable, Ok(TypeRelation::Compatible))
        || matches!(block_can_build, Ok(TypeRelation::Compatible))
    {
        return Some(super::continue_with_diagnostics(payload, diagnostics));
    }
    if relation_unresolved(&cancellable) || relation_unresolved(&block_can_build) {
        unresolved(
            &mut payload,
            &mut diagnostics,
            "core.eff-cancel-event.unresolved-event-type",
            "the current Event classes are insufficient to determine whether the Event is cancellable",
            span,
        );
        return Some(super::continue_with_diagnostics(payload, diagnostics));
    }

    let login = super::event_relation(&payload.context, PLAYER_LOGIN_EVENT);
    let message = if matches!(login, Ok(TypeRelation::Compatible)) {
        "a connect Event cannot be cancelled; kick the player instead"
    } else {
        "the current Event cannot be cancelled"
    };
    Some(super::reject_with(
        message,
        "core.eff-cancel-event.unsupported-event",
        span,
    ))
}

fn relation_unresolved(relation: &Result<TypeRelation, String>) -> bool {
    matches!(relation, Ok(TypeRelation::Unknown) | Err(_))
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
    fn cancel_and_uncancel_patterns_keep_their_java_modes() {
        let mode = |pattern_index| match pattern_index {
            0 => Some("cancel"),
            1 => Some("uncancel"),
            _ => None,
        };
        assert_eq!(mode(0), Some("cancel"));
        assert_eq!(mode(1), Some("uncancel"));
        assert_eq!(mode(2), None);
    }
}

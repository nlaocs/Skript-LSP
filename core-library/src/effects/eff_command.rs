use crate::nlaocs::skript_parser_addon::types::{
    EffectCapture, EffectPayload, HookOutput, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".EffCommand";
const HANDLER_ID: &str = "core.effect.eff-command";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX);
}

pub(super) fn resolve(mut payload: EffectPayload) -> Option<HookOutput> {
    if !super::matches(&payload, HANDLER_ID) {
        return None;
    }
    super::annotate(&mut payload, "semantic-mode", "execute-command");
    let candidate = payload.candidate.as_ref()?;
    let bungee = candidate
        .tags
        .iter()
        .any(|tag| tag.value.eq_ignore_ascii_case("bungee"));
    let expression_count = candidate
        .captures
        .iter()
        .filter(|capture| matches!(capture, EffectCapture::Expression(_)))
        .count();
    if missing_sender(candidate.pattern_index, bungee, expression_count) {
        return Some(super::reject_with(
            "the commandsenders Expression cannot be omitted when using the bungeecord option",
            "core.eff-command.missing-command-sender",
            candidate.span.clone(),
        ));
    }
    Some(super::accept(payload))
}

fn missing_sender(pattern_index: u64, bungee: bool, expression_count: usize) -> bool {
    bungee && pattern_index == 0 && expression_count == 1
}

#[cfg(test)]
mod tests {
    use super::missing_sender;

    #[test]
    fn omitted_sender_guard_only_applies_to_the_optional_sender_pattern() {
        assert!(missing_sender(0, true, 1));
        assert!(!missing_sender(0, false, 1));
        assert!(!missing_sender(1, true, 2));
    }
}

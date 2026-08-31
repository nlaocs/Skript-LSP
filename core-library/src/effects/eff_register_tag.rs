use crate::nlaocs::skript_parser_addon::types::{
    EffectPayload, HookOutput, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".EffRegisterTag";
const HANDLER_ID: &str = "core.effect.eff-register-tag";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX);
}

pub(super) fn resolve(mut payload: EffectPayload) -> Option<HookOutput> {
    if !super::matches(&payload, HANDLER_ID) {
        return None;
    }
    super::annotate(&mut payload, "semantic-mode", "register-tag");
    let name = super::parsed_capture(&payload, 0)?;
    if name.parser_id != "core.literal.string" || name.text.contains('%') {
        return Some(super::accept(payload));
    }
    let name = unquote(&name.text);
    let key = name.strip_prefix("skript:").unwrap_or(name);
    if !key.is_empty() && key.chars().all(is_valid_key_character) {
        return Some(super::accept(payload));
    }
    Some(super::reject_with(
        "tag names may contain only letters, numbers, '/', '.', '_', and '-'",
        "core.eff-register-tag.invalid-name",
        name_span(&payload),
    ))
}

fn is_valid_key_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '/' | '.' | '_' | '-')
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
}

fn name_span(payload: &EffectPayload) -> crate::nlaocs::skript_parser_addon::types::MappedSpan {
    super::parsed_capture(payload, 0)
        .map(|capture| capture.span.clone())
        .or_else(|| {
            payload
                .candidate
                .as_ref()
                .map(|candidate| candidate.span.clone())
        })
        .unwrap_or_else(|| payload.span.clone())
}

#[cfg(test)]
mod tests {
    use super::is_valid_key_character;

    #[test]
    fn tag_key_characters_match_skript() {
        assert!("fish/river.v2-test_1".chars().all(is_valid_key_character));
        assert!(!"fish tag".chars().all(is_valid_key_character));
        assert!(!"fish:tag".chars().all(is_valid_key_character));
    }
}

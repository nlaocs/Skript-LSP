use crate::nlaocs::skript_parser_addon::types::{
    ConditionCapture, ConditionPayload, HookOutput, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".CondScriptLoaded";
const HANDLER_ID: &str = "core.condition.cond-script-loaded";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX);
}

pub(super) fn resolve(mut payload: ConditionPayload) -> Option<HookOutput> {
    if !super::matches(&payload, HANDLER_ID) {
        return None;
    }
    super::annotate(&mut payload, "semantic-mode", "script-loaded");
    let has_script_names = payload
        .candidate
        .captures
        .iter()
        .any(|capture| matches!(capture, ConditionCapture::Expression(_)));
    if has_script_names {
        return Some(super::accept(payload));
    }
    let active = payload
        .context
        .values
        .iter()
        .rfind(|entry| entry.key == "parser.active")
        .and_then(|entry| match entry.value.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        });
    let span = payload.candidate.span.clone();
    Some(match active {
        Some(true) => super::accept(payload),
        Some(false) => super::reject_with(
            "the 'script loaded' Condition requires a script name outside an active script",
            "core.cond-script-loaded.missing-script",
            span,
        ),
        None => {
            super::mark_unresolved(&mut payload, "core.cond-script-loaded.unresolved-script");
            let mut output = super::accept(payload);
            output.effects.diagnostics.push(super::warning(
                "core.cond-script-loaded.unresolved-script",
                "the parser did not expose whether a script is active",
                span,
            ));
            output
        }
    })
}

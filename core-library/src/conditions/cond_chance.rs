use super::{accept, annotate, matches, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    ConditionPayload, HookOutput, RegisteredSyntaxHandler,
};

const HANDLER_ID: &str = "core.condition.cond-chance";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, ".CondChance");
}

pub(super) fn resolve(mut payload: ConditionPayload) -> Option<HookOutput> {
    matches(&payload, HANDLER_ID).then(|| {
        // Skript documents 0..1 and 0%..100%, but CondChance.init() accepts dynamic and
        // out-of-range values alike. Keep that runtime behavior instead of inventing an error.
        let unit = if payload.candidate.mark == 1 {
            "percent"
        } else {
            "fraction"
        };
        annotate(&mut payload, "semantic-mode", "chance");
        annotate(&mut payload, "chance-unit", unit);
        accept(payload)
    })
}

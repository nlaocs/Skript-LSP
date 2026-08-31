use super::{accept, annotate, matches, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    ConditionPayload, HookOutput, RegisteredSyntaxHandler,
};

const HANDLER_ID: &str = "core.condition.cond-matches";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, ".CondMatches");
}

pub(super) fn resolve(mut payload: ConditionPayload) -> Option<HookOutput> {
    matches(&payload, HANDLER_ID).then(|| {
        let mode = if payload.candidate.pattern_index == 1 {
            "partial-regex"
        } else {
            "regex"
        };
        annotate(&mut payload, "semantic-mode", mode);
        accept(payload)
    })
}

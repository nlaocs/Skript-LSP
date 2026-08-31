use super::{accept, annotate, matches, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    ConditionPayload, HookOutput, RegisteredSyntaxHandler,
};

const HANDLER_ID: &str = "core.condition.cond-is-set";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, ".CondIsSet");
}

pub(super) fn resolve(mut payload: ConditionPayload) -> Option<HookOutput> {
    matches(&payload, HANDLER_ID).then(|| {
        annotate(&mut payload, "semantic-mode", "existence");
        accept(payload)
    })
}

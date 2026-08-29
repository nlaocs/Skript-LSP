use super::{register_handler, resolve_condition_section};
use crate::nlaocs::skript_parser_addon::types::{
    HookOutput, InvocationContext, RegisteredSyntaxHandler, SectionPayload,
};

const CLASS_SUFFIX: &str = ".SecConditional";
const HANDLER_ID: &str = "core.section.sec-conditional";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX);
}

pub(super) fn matches(payload: &SectionPayload) -> bool {
    crate::runtime::handler_matches(HANDLER_ID, &payload.candidate.registration_id)
}

pub(super) fn resolve(context: InvocationContext, payload: SectionPayload) -> HookOutput {
    resolve_condition_section(context, payload)
}

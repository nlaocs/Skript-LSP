use super::{register_handler, resolve_condition_section};
use crate::nlaocs::skript_parser_addon::types::{
    HookOutput, InvocationContext, RegisteredSyntaxHandler, SectionPayload,
};

const CLASS_SUFFIX: &str = ".SecWhile";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, CLASS_SUFFIX);
}

pub(super) fn matches(payload: &SectionPayload) -> bool {
    payload
        .candidate
        .element_class
        .as_deref()
        .is_some_and(|class| class.ends_with(CLASS_SUFFIX))
}

pub(super) fn resolve(context: InvocationContext, payload: SectionPayload) -> HookOutput {
    resolve_condition_section(context, payload)
}

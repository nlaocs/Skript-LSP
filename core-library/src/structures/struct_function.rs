use super::register_handler;
use crate::nlaocs::skript_parser_addon::types::{
    HookOutput, InvocationContext, RegisteredSyntaxHandler, StructureBodyMode, StructurePayload,
};

const CLASS_SUFFIX: &str = ".StructFunction";
const HANDLER_ID: &str = "core.structure.struct-function";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn matches(payload: &StructurePayload) -> bool {
    crate::runtime::handler_matches(HANDLER_ID, &payload.candidate.registration_id)
}

pub(super) fn resolve(context: InvocationContext, payload: StructurePayload) -> HookOutput {
    super::continue_with_mode(
        &context,
        payload,
        StructureBodyMode::Trigger,
        "function-structure",
        "core.structure.function",
    )
}

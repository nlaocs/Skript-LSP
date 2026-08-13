mod eff_do_if;

use crate::nlaocs::skript_parser_addon::types::{
    AddonError, AddonErrorKind, HookInvocation, HookOutput, HookPayload, HookPhase,
    RegisteredSyntaxHandler,
};
use crate::{addon_error, continue_without_replacement};

pub(crate) fn handlers() -> Vec<RegisteredSyntaxHandler> {
    let mut handlers = Vec::new();
    eff_do_if::register(&mut handlers);
    handlers
}

pub(crate) fn parse(input: HookInvocation) -> Result<HookOutput, AddonError> {
    if !matches!(input.phase, HookPhase::Effect) {
        return Err(addon_error(
            AddonErrorKind::InvalidPayload,
            "CoreLibrary Effect semantics require the Effect phase",
        ));
    }
    let HookPayload::Effect(payload) = input.payload else {
        return Err(addon_error(
            AddonErrorKind::InvalidPayload,
            "CoreLibrary Effect semantics require an Effect payload",
        ));
    };
    Ok(eff_do_if::resolve(payload).unwrap_or_else(continue_without_replacement))
}

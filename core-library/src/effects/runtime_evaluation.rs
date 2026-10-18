//! Exact early operand observations, not whole-effect preservation.
use crate::nlaocs::skript_parser_addon::types::{
    EffectPayload, HookOutput, RegisteredSyntaxHandler,
};

const HANDLER: &str = "core.effect.runtime-evaluation";
const TELEPORT: &str = "ch.njol.skript.effects.EffTeleport";
const MESSAGE: &str = "org.skriptlang.skript.bukkit.text.elements.effects.EffMessage";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler_targets(handlers, HANDLER, &[".EffTeleport", ".EffMessage"]);
}
pub(super) fn resolve(mut payload: EffectPayload) -> Option<HookOutput> {
    if !super::matches(&payload, HANDLER) {
        return None;
    }
    let candidate = payload.candidate.as_mut()?;
    let (class, prefix): (&str, &[u64]) = match candidate.element_class.as_deref()? {
        // Direction.combine.get/getAll reads directions then locations before
        // getRelative. That virtual call ends this prefix; entities and flags
        // are deliberately unlisted, including flags read after async removal.
        TELEPORT => (TELEPORT, &[1, 2]),
        // Recipients are read before the message component conversion wrapper.
        MESSAGE => (MESSAGE, &[1]),
        _ => return None,
    };
    if !crate::catalog::standard_effect_source(candidate, class) {
        return None;
    }
    crate::public_data::publish_metadata(
        &mut candidate.metadata,
        crate::public_data::operand_evaluation(prefix),
    );
    Some(super::accept(payload))
}

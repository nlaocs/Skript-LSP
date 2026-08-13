use super::{SemanticResolution, matches, metadata, register_handler};
use crate::nlaocs::skript_parser_addon::types::{
    RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprInventorySlot";
const SLOT: &str = "ch.njol.skript.util.slot.Slot";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, CLASS_SUFFIX).then(|| {
        let number_index = match payload.pattern_index {
            0 => 0,
            1 => 1,
            _ => {
                return SemanticResolution::Reject(
                    "inventory slot Expression has an unknown pattern".to_owned(),
                );
            }
        };
        let Some(multiplicity) = payload
            .children
            .get(number_index)
            .and_then(|child| child.multiplicity)
        else {
            return SemanticResolution::Reject(
                "inventory slot Expression requires a resolved slot number".to_owned(),
            );
        };
        SemanticResolution::Resolved {
            return_type: SLOT.to_owned(),
            multiplicity,
            metadata: vec![metadata("semantic-mode", "inventory-slot")],
        }
    })
}

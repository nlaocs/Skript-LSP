use super::{SemanticResolution, matches, metadata, register_handler, resolved_with_metadata};
use crate::nlaocs::skript_parser_addon::types::{
    RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprInventorySlot";
const HANDLER_ID: &str = "core.expression.expr-inventory-slot";
const SLOT: &str = "ch.njol.skript.util.slot.Slot";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        // ExprInventorySlot.init() selects exprs[0] for matchedPattern 0 and exprs[1] for pattern 1.
        let Some(number_index) = number_child_index(payload.pattern_index) else {
            return SemanticResolution::Reject(
                "inventory slot Expression has an unknown pattern index".to_owned(),
            );
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
        resolved_with_metadata(
            SLOT.to_owned(),
            multiplicity,
            vec![metadata("semantic-mode", "inventory-slot")],
        )
    })
}

fn number_child_index(pattern_index: u64) -> Option<usize> {
    match pattern_index {
        0 => Some(0),
        1 => Some(1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::number_child_index;

    #[test]
    fn derives_the_number_capture_from_skript_pattern_index() {
        assert_eq!(number_child_index(0), Some(0));
        assert_eq!(number_child_index(1), Some(1));
        assert_eq!(number_child_index(2), None);
    }
}

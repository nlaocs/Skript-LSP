use super::{SemanticResolution, matches, metadata, register_handler};
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
        let Some(number_index) = number_child_index(&payload.pattern) else {
            return SemanticResolution::Reject(
                "inventory slot Expression has an unknown pattern".to_owned(),
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
        SemanticResolution::Resolved {
            return_type: SLOT.to_owned(),
            multiplicity,
            metadata: vec![metadata("semantic-mode", "inventory-slot")],
        }
    })
}

fn number_child_index(pattern: &str) -> Option<usize> {
    let numbers = pattern.find("%numbers%")?;
    let inventory = pattern.find("%inventory%")?;
    Some(usize::from(inventory < numbers))
}

#[cfg(test)]
mod tests {
    use super::number_child_index;

    #[test]
    fn derives_the_number_capture_from_pattern_order() {
        assert_eq!(
            number_child_index("[the] slot[s] %numbers% of %inventory%"),
            Some(0)
        );
        assert_eq!(
            number_child_index("%inventory%'[s] slot[s] %numbers%"),
            Some(1)
        );
        assert_eq!(number_child_index("slot"), None);
    }
}

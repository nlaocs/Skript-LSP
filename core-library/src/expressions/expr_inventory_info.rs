use super::{SemanticResolution, matches, metadata, register_handler, resolved_with_metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprInventoryInfo";
const HANDLER_ID: &str = "core.expression.expr-inventory-info";
const INVENTORY_HOLDER: &str = "org.bukkit.inventory.InventoryHolder";
const PLAYER: &str = "org.bukkit.entity.Player";
const NUMBER: &str = "java.lang.Number";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        // ExprInventoryInfo changed its parse-mark table in Skript 2.16. RuntimeProfile is the
        // source of truth; guessing from translated pattern text would silently select the wrong
        // return type when a profile is malformed.
        let Some(modern) = crate::runtime::skript_at_least(2, 16) else {
            return SemanticResolution::Reject(
                "inventory info Expression requires a valid Skript runtime version".to_owned(),
            );
        };
        let Some(source_multiplicity) = payload
            .children
            .first()
            .and_then(|child| child.multiplicity)
        else {
            return SemanticResolution::Reject(
                "inventory info Expression requires an inventory source".to_owned(),
            );
        };
        let Some((return_type, multiplicity, mode)) =
            inventory_info(modern, payload.mark, source_multiplicity)
        else {
            return SemanticResolution::Reject(
                "inventory info Expression has an unknown parse mark".to_owned(),
            );
        };
        resolved_with_metadata(
            return_type.to_owned(),
            multiplicity,
            vec![
                metadata("semantic-mode", "inventory-info"),
                metadata("inventory-info", mode),
            ],
        )
    })
}

fn inventory_info(
    modern: bool,
    mark: i32,
    source_multiplicity: DynamicMultiplicity,
) -> Option<(&'static str, DynamicMultiplicity, &'static str)> {
    match (modern, mark) {
        (_, 1) => Some((INVENTORY_HOLDER, source_multiplicity, "holder")),
        (false, 2) => Some((PLAYER, DynamicMultiplicity::Multiple, "viewers")),
        (false, 3 | 4) | (true, 2 | 3) => Some((NUMBER, source_multiplicity, "size")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_two_tracks_the_skript_216_breaking_change() {
        assert_eq!(
            inventory_info(false, 2, DynamicMultiplicity::Single),
            Some((PLAYER, DynamicMultiplicity::Multiple, "viewers"))
        );
        assert_eq!(
            inventory_info(true, 2, DynamicMultiplicity::Single),
            Some((NUMBER, DynamicMultiplicity::Single, "size"))
        );
    }

    #[test]
    fn holder_and_size_keep_the_inventory_multiplicity() {
        assert_eq!(
            inventory_info(false, 1, DynamicMultiplicity::Multiple),
            Some((INVENTORY_HOLDER, DynamicMultiplicity::Multiple, "holder"))
        );
        assert_eq!(
            inventory_info(false, 4, DynamicMultiplicity::Multiple),
            Some((NUMBER, DynamicMultiplicity::Multiple, "size"))
        );
        assert_eq!(
            inventory_info(true, 3, DynamicMultiplicity::Multiple),
            Some((NUMBER, DynamicMultiplicity::Multiple, "size"))
        );
    }
}

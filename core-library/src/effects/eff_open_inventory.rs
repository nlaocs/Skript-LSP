use crate::nlaocs::skript_parser_addon::types::{EffectPayload, RegisteredSyntaxHandler};

const CLASS_SUFFIX: &str = ".EffOpenInventory";
const HANDLER_ID: &str = "core.effect.eff-open-inventory";
const INVENTORY_TYPE: &str = "org.bukkit.event.inventory.InventoryType";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX);
}

pub(super) fn resolve(
    mut payload: EffectPayload,
) -> Option<crate::nlaocs::skript_parser_addon::types::HookOutput> {
    if !super::matches(&payload, HANDLER_ID) {
        return None;
    }
    let pattern_index = payload.candidate.as_ref()?.pattern_index;
    let modern_guard = crate::runtime::skript_at_least(2, 10);
    super::annotate(&mut payload, "semantic-mode", "open-close-inventory");
    if modern_guard == Some(false) {
        return Some(super::accept(payload));
    }
    if modern_guard.is_none() {
        let span = payload.candidate.as_ref()?.span.clone();
        return Some(unresolved(
            payload,
            "core.eff-open-inventory.unresolved-generation",
            "the Skript version is unavailable, so InventoryType creatability checks were skipped",
            span,
        ));
    }
    if pattern_index != 2 {
        return Some(super::accept(payload));
    }

    let Some(inventory) = super::parsed_capture(&payload, 0) else {
        let span = payload.candidate.as_ref()?.span.clone();
        return Some(unresolved(
            payload,
            "core.eff-open-inventory.missing-inventory",
            "the inventory Expression was not parsed",
            span,
        ));
    };
    let span = inventory.span.clone();
    let Some(summary) = inventory.summary.as_ref() else {
        return Some(unresolved(
            payload,
            "core.eff-open-inventory.unresolved-inventory",
            "the inventory Expression has no semantic summary",
            span,
        ));
    };
    if summary.kind != "literal" || summary.return_type.as_deref() != Some(INVENTORY_TYPE) {
        return Some(super::accept(payload));
    }
    match creatable(super::metadata_value(
        &summary.metadata,
        "inventory-type-creatable",
    )) {
        Some(true) => Some(super::accept(payload)),
        Some(false) => Some(super::reject_with(
            "this InventoryType cannot be used to create an inventory",
            "core.eff-open-inventory.non-creatable-type",
            span,
        )),
        None => Some(unresolved(
            payload,
            "core.eff-open-inventory.unresolved-creatable-state",
            "the literal InventoryType does not expose InventoryType.isCreatable()",
            span,
        )),
    }
}

fn unresolved(
    mut payload: EffectPayload,
    code: &str,
    message: &str,
    span: crate::nlaocs::skript_parser_addon::types::MappedSpan,
) -> crate::nlaocs::skript_parser_addon::types::HookOutput {
    super::mark_unresolved(&mut payload, code);
    super::continue_with_diagnostics(payload, vec![super::warning(code, message, span)])
}

fn creatable(value: Option<&str>) -> Option<bool> {
    match value? {
        value if value.eq_ignore_ascii_case("true") => Some(true),
        value if value.eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::creatable;

    #[test]
    fn only_uses_explicit_inventory_creatability() {
        assert_eq!(creatable(Some("true")), Some(true));
        assert_eq!(creatable(Some("false")), Some(false));
        assert_eq!(creatable(Some("unknown")), None);
        assert_eq!(creatable(None), None);
    }
}

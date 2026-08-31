use crate::nlaocs::skript_parser_addon::types::{
    EffectPayload, HookOutput, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".EffEnchant";
const HANDLER_ID: &str = "core.effect.eff-enchant";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX);
}

pub(super) fn resolve(mut payload: EffectPayload) -> Option<HookOutput> {
    if !super::matches(&payload, HANDLER_ID) {
        return None;
    }
    super::annotate(&mut payload, "semantic-mode", "enchant-items");
    let Some(items) = super::parsed_capture(&payload, 0) else {
        let span = payload.candidate.as_ref()?.span.clone();
        super::mark_unresolved(&mut payload, "core.eff-enchant.unresolved-items");
        return Some(super::continue_with_diagnostics(
            payload,
            vec![super::warning(
                "core.eff-enchant.unresolved-items",
                "the ItemType Expression is unavailable, so its SET changer could not be validated",
                span,
            )],
        ));
    };
    let span = items.span.clone();
    match super::set_contract_verdict(items, &["org.bukkit.inventory.ItemStack"]) {
        Ok(super::ContractVerdict::Accepted) => Some(super::accept(payload)),
        Ok(super::ContractVerdict::Rejected) => Some(super::reject_with(
            "the ItemType Expression cannot be changed to ItemStack and therefore cannot be enchanted",
            "core.eff-enchant.unsupported-change",
            span,
        )),
        Ok(super::ContractVerdict::Unresolved) | Err(_) => {
            super::mark_unresolved(&mut payload, "core.eff-enchant.unresolved-change-contract");
            Some(super::continue_with_diagnostics(
                payload,
                vec![super::warning(
                    "core.eff-enchant.unresolved-change-contract",
                    "the ItemType Expression's SET changer is unresolved",
                    span,
                )],
            ))
        }
    }
}

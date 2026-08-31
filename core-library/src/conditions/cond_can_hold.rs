use crate::nlaocs::skript_parser_addon::types::{
    ConditionPayload, HookOutput, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".CondCanHold";
const HANDLER_ID: &str = "core.condition.cond-can-hold";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX);
}

pub(super) fn resolve(mut payload: ConditionPayload) -> Option<HookOutput> {
    if !super::matches(&payload, HANDLER_ID) {
        return None;
    }
    super::annotate(&mut payload, "semantic-mode", "inventory-can-hold");
    let Some(items) = super::child(&payload, 1) else {
        return Some(unresolved(
            payload,
            "the ItemType Expression is unavailable",
        ));
    };
    if !is_static_item_literal(&items.kind) {
        return Some(super::accept(payload));
    }
    let alias_all = metadata_bool(&items.metadata, "literal-alias-all");
    let type_count = metadata_u64(&items.metadata, "literal-alias-type-count");
    let span = super::child_span(&payload, 1);
    Some(match (alias_all, type_count) {
        (Some(true), _) | (_, Some(1)) => super::accept(payload),
        (Some(false), Some(2..)) => super::reject_with(
            "the 'can hold' Condition only accepts a single item type or an all-material alias",
            "core.cond-can-hold.ambiguous-item-type",
            span,
        ),
        _ => unresolved(
            payload,
            "the literal ItemType does not expose enough alias data for the can-hold restriction",
        ),
    })
}

fn is_static_item_literal(kind: &str) -> bool {
    kind == "literal"
}

fn metadata_bool(
    metadata: &[crate::nlaocs::skript_parser_addon::types::MetadataEntry],
    key: &str,
) -> Option<bool> {
    match metadata
        .iter()
        .rfind(|entry| entry.key.ends_with(key))?
        .value
        .as_str()
    {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn metadata_u64(
    metadata: &[crate::nlaocs::skript_parser_addon::types::MetadataEntry],
    key: &str,
) -> Option<u64> {
    metadata
        .iter()
        .rfind(|entry| entry.key.ends_with(key))?
        .value
        .parse()
        .ok()
}

fn unresolved(mut payload: ConditionPayload, message: &str) -> HookOutput {
    let span = payload.candidate.span.clone();
    super::mark_unresolved(&mut payload, "core.cond-can-hold.unresolved-item-type");
    let mut output = super::accept(payload);
    output.effects.diagnostics.push(super::warning(
        "core.cond-can-hold.unresolved-item-type",
        message,
        span,
    ));
    output
}

#[cfg(test)]
mod tests {
    use super::{is_static_item_literal, metadata_bool, metadata_u64};
    use crate::nlaocs::skript_parser_addon::types::MetadataEntry;

    #[test]
    fn reads_alias_cardinality_from_literal_metadata() {
        let metadata = vec![
            MetadataEntry {
                key: "literal-alias-all".to_owned(),
                value: "false".to_owned(),
                owner_component_id: None,
            },
            MetadataEntry {
                key: "literal-alias-type-count".to_owned(),
                value: "2".to_owned(),
                owner_component_id: None,
            },
        ];
        assert_eq!(metadata_bool(&metadata, "literal-alias-all"), Some(false));
        assert_eq!(metadata_u64(&metadata, "literal-alias-type-count"), Some(2));
    }

    #[test]
    fn only_literal_items_use_static_item_type_checks() {
        assert!(is_static_item_literal("literal"));
        assert!(!is_static_item_literal("expression-list"));
        assert!(!is_static_item_literal("registered-expression"));
    }
}

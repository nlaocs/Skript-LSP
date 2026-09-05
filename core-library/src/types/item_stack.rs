//! Skript's exact ItemStack parser, backed by the ItemType alias parser.

use crate::expression_candidates::metadata;
use crate::nlaocs::skript_parser_addon::types::{ExpressionLeafCandidate, ExpressionPayload};

const ITEM_STACK: &str = "org.bukkit.inventory.ItemStack";

pub(super) const PARSER: super::TypeParser = super::TypeParser {
    id: "core.type.item-stack",
    classes: &[ITEM_STACK],
    parse,
    unresolved: None,
    all_type_options: false,
};

pub(super) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    let mut parsed = super::item_type::parse(payload, text, end)?;
    if metadata_value(&parsed, "literal-source") == Some("script-alias") {
        return None;
    }
    if metadata_value(&parsed, "literal-alias-type-count").is_some_and(|count| count != "1") {
        return None;
    }
    parsed.parser_id = "core.literal.item-stack".to_owned();
    parsed.return_type = Some(ITEM_STACK.to_owned());
    parsed
        .metadata
        .push(metadata("item-stack-source", "item-type-parser"));
    Some(parsed)
}

fn metadata_value<'a>(candidate: &'a ExpressionLeafCandidate, key: &str) -> Option<&'a str> {
    candidate
        .metadata
        .iter()
        .find(|entry| entry.key == key)
        .map(|entry| entry.value.as_str())
}

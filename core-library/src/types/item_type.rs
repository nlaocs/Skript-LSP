use super::registered_literal::candidate_from_option;
use crate::expression_candidates::metadata;
use crate::nlaocs::skript_parser_addon::types::{
    ExpressionLeafCandidate, ExpressionLiteralSource, ExpressionPayload,
};

const ITEM_TYPE: &str = "ch.njol.skript.aliases.ItemType";

pub(super) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_literals {
        return None;
    }
    let prefix = parse_prefix(text)?;
    let literal_start = payload.remaining.start.checked_add(prefix.bytes)?;
    let option = payload
        .literal_options
        .iter()
        .filter(|option| {
            option.range.start == literal_start
                && option.range.end == end
                && option.class_name == ITEM_TYPE
                && matches!(option.source, ExpressionLiteralSource::Alias)
        })
        .min_by_key(|option| option.type_parse_order)?;
    let mut candidate = candidate_from_option(
        option,
        "core.literal.item-type",
        payload.remaining.start,
        end,
    );
    if let Some(amount) = prefix.amount {
        candidate.metadata.push(metadata("literal-amount", amount));
    }
    candidate.metadata.push(metadata(
        "literal-all",
        if prefix.all { "true" } else { "false" },
    ));
    Some(candidate)
}

struct ItemPrefix<'a> {
    bytes: u64,
    amount: Option<&'a str>,
    all: bool,
}

fn parse_prefix(text: &str) -> Option<ItemPrefix<'_>> {
    let bytes = text.as_bytes();
    let digits = bytes
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    let mut offset = 0;
    let mut amount = None;
    if digits > 0 {
        amount = text.get(..digits);
        offset = skip_spaces(text, digits)?;
        if let Some(next) = skip_word(text, offset, "of") {
            offset = next;
        }
    }

    let mut all = false;
    for word in ["all", "every"] {
        if let Some(next) = skip_word(text, offset, word) {
            offset = next;
            all = true;
            break;
        }
    }
    if amount.is_none() && !all {
        for word in ["a", "an"] {
            if let Some(next) = skip_word(text, offset, word) {
                offset = next;
                amount = Some("1");
                break;
            }
        }
    }
    (offset > 0 && offset < text.len()).then_some(ItemPrefix {
        bytes: u64::try_from(offset).ok()?,
        amount,
        all,
    })
}

fn skip_word(text: &str, offset: usize, word: &str) -> Option<usize> {
    let end = offset.checked_add(word.len())?;
    if !text.get(offset..end)?.eq_ignore_ascii_case(word) {
        return None;
    }
    skip_spaces(text, end)
}

fn skip_spaces(text: &str, offset: usize) -> Option<usize> {
    let spaces = text
        .as_bytes()
        .get(offset..)?
        .iter()
        .take_while(|byte| byte.is_ascii_whitespace())
        .count();
    (spaces > 0).then_some(offset + spaces)
}

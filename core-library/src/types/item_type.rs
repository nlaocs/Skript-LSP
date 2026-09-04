use super::registered_literal::candidate_from_option;
use crate::expression_candidates::metadata;
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionLiteralSource,
    ExpressionPayload,
};
#[cfg(target_arch = "wasm32")]
use crate::nlaocs::skript_parser_addon::{
    state_store,
    types::{StateNamespaceVisibility, StateScope},
};
use fancy_regex::Regex;
use std::cell::RefCell;
use std::collections::HashMap;

const ITEM_TYPE: &str = "ch.njol.skript.aliases.ItemType";

thread_local! {
    static PREFIX_PATTERNS: RefCell<HashMap<String, Option<Regex>>> =
        RefCell::new(HashMap::new());
}

pub(super) const PARSER: super::TypeParser = super::TypeParser {
    id: "core.type.item-type",
    classes: &["ch.njol.skript.aliases.ItemType"],
    parse,
    unresolved: None,
    all_type_options: false,
};

pub(super) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_literals {
        return None;
    }
    if let Some(candidate) = script_alias(payload, text, end) {
        return Some(candidate);
    }
    if !payload
        .literal_options
        .iter()
        .any(|option| option.class_name == ITEM_TYPE && option.range.end <= end)
    {
        return None;
    }
    // Aliases.parseItemType accepts both a bare alias (`stone`) and the same alias
    // behind an amount/article prefix (`2 stone`, `a stone`). The host publishes
    // literal options for both ranges, so absence of a prefix means a zero-byte prefix.
    let prefix = parse_prefix(text).unwrap_or(ItemPrefix {
        bytes: 0,
        amount: None,
        all: false,
    });
    let literal_start = payload.remaining.start.checked_add(prefix.bytes)?;
    let direct = item_alias_option(payload, literal_start, end);
    let (option, enchantments) = direct.map_or_else(
        || item_alias_with_enchantments(payload, text, prefix.bytes, literal_start),
        |option| Some((option, Vec::new())),
    )?;
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
    for (index, enchantment) in enchantments.into_iter().enumerate() {
        candidate.metadata.push(metadata(
            &format!("literal-enchantment.{index}.name"),
            enchantment.name,
        ));
        if let Some(level) = enchantment.level {
            candidate.metadata.push(metadata(
                &format!("literal-enchantment.{index}.level"),
                level,
            ));
        }
    }
    Some(candidate)
}

fn script_alias(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    let prefix = parse_prefix(text).unwrap_or(ItemPrefix {
        bytes: 0,
        amount: None,
        all: false,
    });
    let alias_start = usize::try_from(prefix.bytes).ok()?;
    let name = text.get(alias_start..)?.trim().to_ascii_lowercase();
    let (plural, value) = script_alias_value(&name)?;
    let mut candidate = crate::expression_candidates::candidate(
        "core.literal.script-alias",
        ExpressionLeafKind::Literal,
        payload.remaining.start,
        end,
        ITEM_TYPE,
        DynamicMultiplicity::Single,
    );
    candidate.metadata.extend([
        metadata("type-code-name", "itemtype"),
        metadata("literal-canonical", &name),
        metadata("literal-source", "script-alias"),
        metadata("literal-plural", if plural { "true" } else { "false" }),
        metadata("script-alias-values", &value),
        metadata("literal-all", if prefix.all { "true" } else { "false" }),
    ]);
    if let Some(amount) = prefix.amount {
        candidate.metadata.push(metadata("literal-amount", amount));
    }
    Some(candidate)
}

#[cfg(target_arch = "wasm32")]
fn script_alias_value(name: &str) -> Option<(bool, String)> {
    let value = state_store::get(
        StateScope::Parse,
        StateNamespaceVisibility::Private,
        "aliases",
        name,
    )
    .ok()??;
    let (&plural, value) = value.bytes.split_first()?;
    Some((plural != 0, String::from_utf8(value.to_vec()).ok()?))
}

#[cfg(not(target_arch = "wasm32"))]
fn script_alias_value(_name: &str) -> Option<(bool, String)> {
    None
}

fn item_alias_option(
    payload: &ExpressionPayload,
    start: u64,
    end: u64,
) -> Option<&crate::nlaocs::skript_parser_addon::types::ExpressionLiteralOption> {
    payload
        .literal_options
        .iter()
        .filter(|option| {
            option.range.start == start
                && option.range.end == end
                && option.class_name == ITEM_TYPE
                && matches!(option.source, ExpressionLiteralSource::Alias)
        })
        .min_by_key(|option| option.type_parse_order)
}

struct ParsedEnchantment<'a> {
    name: &'a str,
    level: Option<&'a str>,
}

fn item_alias_with_enchantments<'a>(
    payload: &'a ExpressionPayload,
    text: &'a str,
    prefix_bytes: u64,
    literal_start: u64,
) -> Option<(
    &'a crate::nlaocs::skript_parser_addon::types::ExpressionLiteralOption,
    Vec<ParsedEnchantment<'a>>,
)> {
    let prefix = usize::try_from(prefix_bytes).ok()?;
    let remaining = text.get(prefix..)?;
    let of = crate::language::value("of", "of");
    let separator = format!(" {of} ");
    let lowercase = remaining.to_ascii_lowercase();
    let lowercase_separator = separator.to_ascii_lowercase();
    let mut search_from = 0usize;
    while let Some(relative) = lowercase.get(search_from..)?.find(&lowercase_separator) {
        let separator_start = search_from + relative;
        let base = remaining.get(..separator_start)?.trim_end();
        let base_end = literal_start.checked_add(u64::try_from(base.len()).ok()?)?;
        let Some(option) = item_alias_option(payload, literal_start, base_end) else {
            search_from = separator_start + 1;
            continue;
        };
        let suffix = remaining.get(separator_start + separator.len()..)?;
        if let Some(enchantments) = parse_enchantment_list(suffix, enchantment_exists) {
            return Some((option, enchantments));
        }
        search_from = separator_start + 1;
    }
    None
}

fn parse_enchantment_list<'a>(
    source: &'a str,
    mut exists: impl FnMut(&str) -> bool,
) -> Option<Vec<ParsedEnchantment<'a>>> {
    let mut parsed = Vec::new();
    let mut cursor = 0usize;
    while cursor <= source.len() {
        let rest = source.get(cursor..)?;
        let comma = rest.find(',');
        let and_separator = crate::language::value("and", "and");
        let and = rest
            .to_ascii_lowercase()
            .find(&and_separator.to_ascii_lowercase());
        let (end, separator_len) = match (comma, and) {
            (Some(comma), Some(and)) if comma <= and => (cursor + comma, 1),
            (Some(_), Some(and)) => (cursor + and, and_separator.len()),
            (Some(comma), None) => (cursor + comma, 1),
            (None, Some(and)) => (cursor + and, and_separator.len()),
            (None, None) => (source.len(), 0),
        };
        let piece = source.get(cursor..end)?.trim();
        if piece.is_empty() {
            return None;
        }
        let (name, level) = split_enchantment_level(piece);
        if !exists(name) {
            return None;
        }
        parsed.push(ParsedEnchantment { name, level });
        if separator_len == 0 {
            break;
        }
        cursor = end.checked_add(separator_len)?;
    }
    (!parsed.is_empty()).then_some(parsed)
}

fn split_enchantment_level(source: &str) -> (&str, Option<&str>) {
    let Some((name, level)) = source.rsplit_once(' ') else {
        return (source, None);
    };
    if !name.is_empty() && !level.is_empty() && level.bytes().all(|byte| byte.is_ascii_digit()) {
        (name.trim_end(), Some(level))
    } else {
        (source, None)
    }
}

#[cfg(target_arch = "wasm32")]
fn enchantment_exists(name: &str) -> bool {
    crate::nlaocs::skript_parser_addon::catalog_data::type_literal_matches(name).is_ok_and(
        |matches| {
            matches
                .iter()
                .any(|option| option.code_name == "enchantment")
        },
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn enchantment_exists(_name: &str) -> bool {
    false
}

struct ItemPrefix<'a> {
    bytes: u64,
    amount: Option<&'a str>,
    all: bool,
}

fn parse_prefix(text: &str) -> Option<ItemPrefix<'_>> {
    let every = crate::language::value("aliases.every", "(?:every|all)");
    let of_every = crate::language::value("aliases.of every", "of (?:every|all)");
    let of = crate::language::value("aliases.of", "of(?: any)?");
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    let amount = text.get(..digits).filter(|value| !value.is_empty());

    if amount.is_some() {
        let pattern = format!(r"(?i)^\d+ (?:{of_every}) (?P<item>.+)$");
        if let Some(bytes) = item_start(&pattern, text) {
            return Some(ItemPrefix {
                bytes: u64::try_from(bytes).ok()?,
                amount,
                all: true,
            });
        }
        let pattern = format!(r"(?i)^\d+ (?:(?:{of}) )?(?P<item>.+)$");
        if let Some(bytes) = item_start(&pattern, text) {
            return Some(ItemPrefix {
                bytes: u64::try_from(bytes).ok()?,
                amount,
                all: false,
            });
        }
    }

    let pattern = format!(r"(?i)^(?:{every}) (?P<item>.+)$");
    if let Some(bytes) = item_start(&pattern, text) {
        return Some(ItemPrefix {
            bytes: u64::try_from(bytes).ok()?,
            amount: None,
            all: true,
        });
    }

    let item = crate::language::strip_indefinite_article(text);
    (item.len() < text.len()).then_some(ItemPrefix {
        bytes: u64::try_from(text.len() - item.len()).ok()?,
        amount: Some("1"),
        all: false,
    })
}

fn item_start(pattern: &str, text: &str) -> Option<usize> {
    PREFIX_PATTERNS.with(|patterns| {
        let mut patterns = patterns.borrow_mut();
        let pattern = patterns
            .entry(pattern.to_owned())
            .or_insert_with(|| Regex::new(pattern).ok())
            .as_ref()?;
        pattern
            .captures(text)
            .ok()??
            .name("item")
            .map(|item| item.start())
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_enchantment_list, parse_prefix};

    #[test]
    fn matches_skript_amount_and_all_prefixes() {
        let prefix = parse_prefix("2 stone").expect("amount prefix");
        assert_eq!(prefix.bytes, 2);
        assert_eq!(prefix.amount, Some("2"));
        assert!(!prefix.all);

        let prefix = parse_prefix("2 of every stone").expect("amount and all prefix");
        assert_eq!(prefix.bytes, 11);
        assert_eq!(prefix.amount, Some("2"));
        assert!(prefix.all);

        let prefix = parse_prefix("an apple").expect("indefinite article");
        assert_eq!(prefix.bytes, 3);
        assert_eq!(prefix.amount, Some("1"));
    }

    #[test]
    fn does_not_treat_every_as_an_amount_modifier_without_of() {
        let prefix = parse_prefix("2 every stone").expect("numeric prefix remains valid");
        assert_eq!(prefix.bytes, 2);
        assert_eq!(prefix.amount, Some("2"));
        assert!(!prefix.all);
    }

    #[test]
    fn matches_skript_any_amount_form() {
        let prefix = parse_prefix("2 of any stone").expect("amount and any prefix");
        assert_eq!(prefix.bytes, 9);
        assert_eq!(prefix.amount, Some("2"));
        assert!(!prefix.all);
    }

    #[test]
    fn parses_enchantment_names_levels_and_separators() {
        let parsed = parse_enchantment_list("sharpness 5, unbreaking and fortune", |name| {
            matches!(name, "sharpness" | "unbreaking" | "fortune")
        })
        .unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].name, "sharpness");
        assert_eq!(parsed[0].level, Some("5"));
        assert_eq!(parsed[1].name, "unbreaking");
        assert_eq!(parsed[1].level, None);
    }

    #[test]
    fn rejects_unknown_or_empty_enchantments() {
        assert!(parse_enchantment_list("unknown", |_| false).is_none());
        assert!(parse_enchantment_list("sharpness,", |_| true).is_none());
    }
}

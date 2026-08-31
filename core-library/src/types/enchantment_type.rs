use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
};

const ENCHANTMENT_TYPE: &str = "ch.njol.skript.util.EnchantmentType";

pub(super) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_literals {
        return None;
    }
    parse_with(payload, text, end, enchantment_literal)
}

fn parse_with(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
    mut lookup: impl FnMut(&str) -> Option<String>,
) -> Option<ExpressionLeafCandidate> {
    let text = text.trim();
    let (name, level) = split_level(text);
    let canonical = lookup(name)?;
    let mut parsed = candidate(
        "core.literal.enchantment-type",
        ExpressionLeafKind::Literal,
        payload.remaining.start,
        end,
        ENCHANTMENT_TYPE,
        DynamicMultiplicity::Single,
    );
    parsed.metadata = vec![metadata("enchantment", &canonical)];
    if let Some(level) = level {
        parsed.metadata.push(metadata("enchantment-level", level));
    }
    Some(parsed)
}

fn split_level(text: &str) -> (&str, Option<&str>) {
    let Some((name, level)) = text.rsplit_once(' ') else {
        return (text, None);
    };
    if !name.is_empty() && !level.is_empty() && level.bytes().all(|byte| byte.is_ascii_digit()) {
        (name, Some(level))
    } else {
        (text, None)
    }
}

#[cfg(target_arch = "wasm32")]
fn enchantment_literal(name: &str) -> Option<String> {
    crate::nlaocs::skript_parser_addon::catalog_data::type_literal_matches(name)
        .ok()?
        .into_iter()
        .find(|option| option.code_name == "enchantment")
        .map(|option| option.canonical_value)
}

#[cfg(not(target_arch = "wasm32"))]
fn enchantment_literal(_name: &str) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::split_level;

    #[test]
    fn level_is_only_a_trailing_decimal_integer() {
        assert_eq!(split_level("sharpness 5"), ("sharpness", Some("5")));
        assert_eq!(split_level("sharpness"), ("sharpness", None));
        assert_eq!(split_level("sharpness V"), ("sharpness V", None));
        assert_eq!(split_level("sharpness -1"), ("sharpness -1", None));
    }
}

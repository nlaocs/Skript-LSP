//! Skript's experience point literal parser.

use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
};
#[cfg(not(target_arch = "wasm32"))]
use fancy_regex::Regex;

const EXPERIENCE: &str = "ch.njol.skript.util.Experience";

pub(super) const PARSER: super::TypeParser = super::TypeParser {
    id: "core.type.experience",
    classes: &[EXPERIENCE],
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
    let (amount, unit) = split_amount(text)?;
    if !matches_experience_pattern(unit) {
        return None;
    }
    let mut parsed = candidate(
        "core.literal.experience",
        ExpressionLeafKind::Literal,
        payload.remaining.start,
        end,
        EXPERIENCE,
        DynamicMultiplicity::Single,
    );
    parsed
        .metadata
        .push(metadata("experience-points", &amount.to_string()));
    Some(parsed)
}

#[cfg(target_arch = "wasm32")]
fn matches_experience_pattern(input: &str) -> bool {
    let pattern =
        crate::language::value("types.experience.pattern", "(e?xp|experience( points?)?)");
    // Skript's RegexMessage uses Pattern.CASE_INSENSITIVE here. Compile and cache the
    // equivalent regex in the host because rebuilding it for every candidate exhausts guest fuel.
    let pattern = format!("(?i:{pattern})");
    crate::nlaocs::skript_parser_addon::catalog_data::language_pattern_matches(
        "nlaocs.core-library.types.experience.pattern",
        &pattern,
        input,
    )
    .unwrap_or(false)
}

#[cfg(not(target_arch = "wasm32"))]
fn matches_experience_pattern(input: &str) -> bool {
    let pattern =
        crate::language::value("types.experience.pattern", "(e?xp|experience( points?)?)");
    Regex::new(&format!("(?i)^(?:{pattern})$"))
        .ok()
        .and_then(|pattern| pattern.is_match(input).ok())
        .unwrap_or(false)
}

fn split_amount(source: &str) -> Option<(i32, &str)> {
    if let Some((amount, unit)) = source.split_once(' ')
        && !unit.is_empty()
        && !amount.is_empty()
        && amount.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Some((amount.parse().unwrap_or(i32::MAX), unit));
    }
    (!source.is_empty()).then_some((-1, source))
}

#[cfg(test)]
mod tests {
    use super::{matches_experience_pattern, split_amount};

    #[test]
    fn amount_matches_skript_unsigned_integer_prefix() {
        assert_eq!(split_amount("10 xp"), Some((10, "xp")));
        assert_eq!(split_amount("xp"), Some((-1, "xp")));
        assert_eq!(
            split_amount("999999999999999999 xp"),
            Some((i32::MAX, "xp"))
        );
        assert_eq!(split_amount("-1 xp"), Some((-1, "-1 xp")));
    }

    #[test]
    fn matches_experience_pattern_accepts_case_insensitive_units_and_rejects_invalid_units() {
        for unit in ["xp", "EXP", "experience point", "EXPERIENCE POINTS"] {
            assert!(matches_experience_pattern(unit), "accepted unit: {unit}");
        }
        for unit in ["level", "experience pointss"] {
            assert!(!matches_experience_pattern(unit), "rejected unit: {unit}");
        }
    }
}

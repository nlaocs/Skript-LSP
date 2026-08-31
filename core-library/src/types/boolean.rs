use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
};
#[cfg(not(target_arch = "wasm32"))]
use fancy_regex::Regex;
#[cfg(not(target_arch = "wasm32"))]
use std::cell::RefCell;
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static PATTERNS: RefCell<HashMap<String, Option<Regex>>> = RefCell::new(HashMap::new());
}

pub(super) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_literals {
        return None;
    }
    let value = parse_boolean_value(text)?;
    let mut candidate = candidate(
        "core.literal.boolean",
        ExpressionLeafKind::Literal,
        payload.remaining.start,
        end,
        "java.lang.Boolean",
        DynamicMultiplicity::Single,
    );
    candidate.metadata.push(metadata("boolean-value", value));
    Some(candidate)
}

fn parse_boolean_value(text: &str) -> Option<&'static str> {
    if matches_language_pattern("boolean.true.pattern", "(true|yes|on)", text) {
        Some("true")
    } else if matches_language_pattern("boolean.false.pattern", "(false|no|off)", text) {
        Some("false")
    } else {
        None
    }
}

#[cfg(target_arch = "wasm32")]
fn matches_language_pattern(key: &str, fallback: &str, text: &str) -> bool {
    crate::nlaocs::skript_parser_addon::catalog_data::language_pattern_matches(key, fallback, text)
        .unwrap_or(false)
}

#[cfg(not(target_arch = "wasm32"))]
fn matches_language_pattern(_key: &str, fallback: &str, text: &str) -> bool {
    matches_pattern(fallback, text)
}

#[cfg(not(target_arch = "wasm32"))]
fn matches_pattern(pattern: &str, text: &str) -> bool {
    PATTERNS.with(|patterns| {
        let mut patterns = patterns.borrow_mut();
        patterns
            .entry(pattern.to_owned())
            .or_insert_with(|| Regex::new(&format!("^(?:{pattern})$")).ok())
            .as_ref()
            .is_some_and(|pattern| pattern.is_match(text).unwrap_or(false))
    })
}

#[cfg(test)]
mod tests {
    use super::parse_boolean_value;

    #[test]
    fn accepts_the_runtime_boolean_language_patterns() {
        for value in ["true", "yes", "on"] {
            assert_eq!(parse_boolean_value(value), Some("true"));
        }
        for value in ["false", "no", "off"] {
            assert_eq!(parse_boolean_value(value), Some("false"));
        }
    }

    #[test]
    fn rejects_values_outside_the_boolean_language() {
        for value in ["", "1", "0", "maybe", "true ", " yes", "TRUE"] {
            assert_eq!(parse_boolean_value(value), None, "{value:?}");
        }
    }
}

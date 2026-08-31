use crate::expression_candidates::candidate;
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
};

pub(super) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_literals || !is_quoted_correctly(text) || !has_balanced_percent_signs(text) {
        return None;
    }
    Some(candidate(
        "core.literal.string",
        ExpressionLeafKind::Literal,
        payload.remaining.start,
        end,
        "java.lang.String",
        DynamicMultiplicity::Single,
    ))
}

/// Mirrors `VariableString.isQuotedCorrectly(string, true)`. Quotes inside an
/// interpolation belong to the nested expression and are therefore ignored;
/// literal quotes outside one must be doubled.
fn is_quoted_correctly(text: &str) -> bool {
    if text.len() < 2 || !text.starts_with('"') || !text.ends_with('"') {
        return false;
    }
    let mut quote = false;
    let mut interpolation = false;
    for character in text[1..text.len() - 1].chars() {
        if interpolation {
            if character == '%' {
                interpolation = false;
            }
            continue;
        }
        if quote && character != '"' {
            return false;
        }
        if character == '"' {
            quote = !quote;
        } else if character == '%' {
            interpolation = true;
        }
    }
    !quote
}

/// `VariableString.newInstance` rejects an odd number of raw percent signs.
/// Escaped `%%` is naturally balanced by this check and is kept as text.
fn has_balanced_percent_signs(text: &str) -> bool {
    text.as_bytes().iter().filter(|byte| **byte == b'%').count() % 2 == 0
}

#[cfg(test)]
mod tests {
    use super::{has_balanced_percent_signs, is_quoted_correctly};

    #[test]
    fn follows_skript_quote_rules_around_interpolations() {
        assert!(is_quoted_correctly("\"hello\""));
        assert!(is_quoted_correctly("\"say \"\"hello\"\"\""));
        assert!(is_quoted_correctly("\"value: %\"quoted\"%\""));
        assert!(!is_quoted_correctly("\"say \"hello\"\""));
        assert!(!is_quoted_correctly("hello"));
    }

    #[test]
    fn requires_balanced_percent_signs() {
        assert!(has_balanced_percent_signs("\"100%%\""));
        assert!(has_balanced_percent_signs("\"%player%\""));
        assert!(!has_balanced_percent_signs("\"%player\""));
    }
}

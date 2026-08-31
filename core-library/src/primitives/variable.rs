use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
};

const SEPARATOR: &str = "::";

pub(super) fn parse(
    payload: &ExpressionPayload,
    text: &str,
    end: u64,
) -> Option<ExpressionLeafCandidate> {
    if !payload.allow_expressions
        || text.len() < 3
        || !text.starts_with('{')
        || !text.ends_with('}')
        || !is_valid_variable_name_body(&text[1..text.len() - 1], true)
    {
        return None;
    }
    let body = &text[1..text.len() - 1];
    let mut candidate = candidate(
        "core.variable",
        ExpressionLeafKind::Variable,
        payload.remaining.start,
        end,
        payload
            .expected_types
            .first()
            .map_or("java.lang.Object", |expected| expected.class_name.as_str()),
        if is_list_variable_body(body) {
            DynamicMultiplicity::Multiple
        } else {
            DynamicMultiplicity::Single
        },
    );
    if is_list_variable_body(body) {
        candidate
            .metadata
            .push(metadata("expression.capability.key-provider", "true"));
        candidate
            .metadata
            .push(metadata("expression.capability.nested-structures", "true"));
    }
    Some(candidate)
}

/// Mirrors Skript's `Variable.isValidVariableName` checks which affect parsing.
///
/// Skript deliberately permits arbitrary text in a variable name. The checks
/// here therefore cover only its structural rules: list separators, list
/// asterisks, and embedded `%...%` expressions. A lone `:` remains valid; the
/// upstream implementation reports that case as a warning only.
pub(super) fn is_valid_variable_name_body(body: &str, allow_list_variable: bool) -> bool {
    let name = normalized_name(body);
    if name.is_empty() {
        return false;
    }
    let Some(protected_ranges) = percent_expression_ranges(name) else {
        return false;
    };

    let has_separator = contains_outside(name, SEPARATOR, &protected_ranges);
    if !allow_list_variable && has_separator {
        return false;
    }
    if name.starts_with(SEPARATOR) || name.ends_with(SEPARATOR) {
        return false;
    }

    let asterisks = name
        .char_indices()
        .filter_map(|(index, character)| {
            (character == '*' && !is_protected(index, &protected_ranges)).then_some(index)
        })
        .collect::<Vec<_>>();
    if !asterisks.is_empty() {
        // The final `::*` is the only asterisk accepted by Skript. The
        // asterisks inside `%...%` are expression text and do not count.
        let valid_list = allow_list_variable
            && asterisks.len() == 1
            && asterisks[0] + 1 == name.len()
            && name.ends_with("::*");
        if !valid_list {
            return false;
        }
    } else if contains_outside(name, "::::", &protected_ranges) {
        return false;
    }

    true
}

fn is_list_variable_body(body: &str) -> bool {
    normalized_name(body).ends_with("::*")
}

fn normalized_name(body: &str) -> &str {
    let name = body.trim();
    name.strip_prefix('_').map_or(name, |name| name.trim())
}

fn contains_outside(name: &str, needle: &str, protected_ranges: &[std::ops::Range<usize>]) -> bool {
    let mut search_from = 0;
    while let Some(relative) = name.get(search_from..).and_then(|rest| rest.find(needle)) {
        let index = search_from + relative;
        if !is_protected(index, protected_ranges) {
            return true;
        }
        search_from = index.saturating_add(1);
    }
    false
}

fn is_protected(index: usize, protected_ranges: &[std::ops::Range<usize>]) -> bool {
    protected_ranges.iter().any(|range| range.contains(&index))
}

/// Returns the byte ranges of embedded expressions, skipping escaped `%%`.
/// Braces keep `%` characters inside a dynamic variable name from terminating
/// the outer expression too early.
fn percent_expression_ranges(input: &str) -> Option<Vec<std::ops::Range<usize>>> {
    let bytes = input.as_bytes();
    let mut ranges = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] != b'%' {
            cursor += 1;
            continue;
        }
        if bytes.get(cursor + 1) == Some(&b'%') {
            cursor += 2;
            continue;
        }
        let start = cursor + 1;
        cursor = start;
        let mut braces = 0usize;
        while cursor < bytes.len() {
            match bytes[cursor] {
                b'{' => braces = braces.saturating_add(1),
                b'}' if braces > 0 => braces -= 1,
                b'%' if braces == 0 => break,
                _ => {}
            }
            cursor += 1;
        }
        if cursor == bytes.len() {
            return None;
        }
        ranges.push(start..cursor);
        cursor += 1;
    }
    Some(ranges)
}

#[cfg(test)]
mod tests {
    use super::is_valid_variable_name_body;

    #[test]
    fn accepts_simple_and_interpolated_variable_names() {
        for name in ["value", "_local value", "values::*", "data::%event-player%"] {
            assert!(is_valid_variable_name_body(name, true), "{name}");
        }
    }

    #[test]
    fn rejects_invalid_separator_and_asterisk_shapes() {
        for name in [
            "::value",
            "value::",
            "value::::other",
            "value*other",
            "value::*other",
        ] {
            assert!(!is_valid_variable_name_body(name, true), "{name}");
        }
        assert!(!is_valid_variable_name_body("values::*", false));
        assert!(is_valid_variable_name_body("value", false));
    }

    #[test]
    fn ignores_expression_text_when_validating_variable_structure() {
        assert!(is_valid_variable_name_body("data::%value*%", true));
        assert!(!is_valid_variable_name_body("data::%value", true));
        assert!(is_valid_variable_name_body("literal%%percent", true));
    }
}

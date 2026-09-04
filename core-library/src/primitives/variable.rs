use crate::expression_candidates::{candidate, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload,
    ExpressionPublicData,
};
use crate::public_data::{
    VARIABLE_SCHEMA_ID, VARIABLE_SCHEMA_VERSION, VariableData, VariableNamePart, VariableScope,
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
    // Interpolated names must own their parsed child Expressions. The
    // interpolation provider creates those before publishing the same schema.
    if !percent_expression_ranges(normalized_name(body))?.is_empty() {
        return None;
    }
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
    candidate.public_data.push(public_name_data(body)?);
    Some(candidate)
}

pub(super) fn public_name_data(body: &str) -> Option<ExpressionPublicData> {
    let scope = if body.trim().starts_with('_') {
        VariableScope::Local
    } else {
        VariableScope::Global
    };
    let name = normalized_name(body);
    let mut parts = Vec::new();
    let mut previous = 0;
    for (index, range) in percent_expression_ranges(name)?.into_iter().enumerate() {
        let text_end = range.start - 1;
        if previous < text_end {
            parts.push(VariableNamePart::Text {
                text: name[previous..text_end].to_owned(),
            });
        }
        parts.push(VariableNamePart::Expression {
            child_index: u32::try_from(index).ok()?,
        });
        previous = range.end + 1;
    }
    if previous < name.len() {
        parts.push(VariableNamePart::Text {
            text: name[previous..].to_owned(),
        });
    }
    Some(ExpressionPublicData {
        schema_id: VARIABLE_SCHEMA_ID.to_owned(),
        schema_version: VARIABLE_SCHEMA_VERSION,
        json: serde_json::to_string(&VariableData { scope, name: parts }).ok()?,
    })
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
    fn publishes_local_global_list_and_interpolated_name_templates() {
        use crate::public_data::{VariableData, VariableNamePart, VariableScope};
        let parse = |body| {
            serde_json::from_str::<VariableData>(&super::public_name_data(body).unwrap().json)
                .unwrap()
        };
        assert_eq!(parse("_money").scope, VariableScope::Local);
        assert_eq!(parse("money").scope, VariableScope::Global);
        assert_eq!(
            parse("_\u{6240}\u{6301}\u{91d1}").name,
            vec![VariableNamePart::Text {
                text: "\u{6240}\u{6301}\u{91d1}".into()
            }]
        );
        assert_eq!(
            parse("_values::*").name,
            vec![VariableNamePart::Text {
                text: "values::*".into()
            }]
        );
        assert_eq!(
            parse("_data::%player%::%{_key}%").name,
            vec![
                VariableNamePart::Text {
                    text: "data::".into()
                },
                VariableNamePart::Expression { child_index: 0 },
                VariableNamePart::Text { text: "::".into() },
                VariableNamePart::Expression { child_index: 1 },
            ]
        );
        assert_eq!(
            parse("literal%%percent").name,
            vec![VariableNamePart::Text {
                text: "literal%%percent".into()
            }]
        );
    }

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

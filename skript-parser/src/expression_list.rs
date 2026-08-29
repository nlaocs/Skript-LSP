//! Shared lexical structure and conjunction rules for Skript Expression lists.

use crate::TextRange;
use crate::pattern_match::{
    find_parenthesis_end, find_quote_end, find_variable_end, java_trim_range,
};

/// Runtime selection semantics of a Skript Expression list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpressionListConjunction {
    /// Every child contributes values. Comma-only and `nor` lists use this mode.
    And,
    /// One child is selected at runtime.
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExpressionListDelimiter {
    Comma,
    And,
    Or,
    Nor,
}

#[derive(Debug)]
pub(crate) struct RawExpressionList {
    pub pieces: Vec<TextRange>,
    pub delimiters: Vec<ExpressionListDelimiter>,
}

impl RawExpressionList {
    pub fn conjunction_for_children(&self, child_starts: &[usize]) -> ExpressionListConjunction {
        let mut mode = None;
        for first_piece in child_starts.iter().copied().skip(1) {
            let incoming = match self.delimiters[first_piece - 1] {
                ExpressionListDelimiter::Comma => continue,
                ExpressionListDelimiter::And | ExpressionListDelimiter::Nor => {
                    ExpressionListConjunction::And
                }
                ExpressionListDelimiter::Or => ExpressionListConjunction::Or,
            };
            mode = Some(match mode {
                None => incoming,
                Some(current) if current == incoming => current,
                Some(_) => ExpressionListConjunction::And,
            });
        }
        mode.unwrap_or(ExpressionListConjunction::And)
    }
}

pub(crate) fn split_expression_list(input: &str, range: TextRange) -> Option<RawExpressionList> {
    let mut pieces = Vec::new();
    let mut delimiters = Vec::new();
    let mut part_start = range.start;
    let mut cursor = range.start;
    while cursor < range.end {
        let character = input.get(cursor..range.end)?.chars().next()?;
        let next = cursor + character.len_utf8();
        match character {
            '"' => {
                cursor = find_quote_end(input, next, range.end)
                    .map(|close| close + '"'.len_utf8())
                    .unwrap_or(range.end);
            }
            '{' => {
                cursor = find_variable_end(input, next, range.end)
                    .map(|close| close + '}'.len_utf8())
                    .unwrap_or(range.end);
            }
            '(' => {
                cursor = find_parenthesis_end(input, next, range.end)
                    .map(|close| close + ')'.len_utf8())
                    .unwrap_or(range.end);
            }
            ',' => {
                let (delimiter_end, delimiter) = comma_delimiter(input, cursor, range.end);
                push_piece(input, part_start, cursor, &mut pieces)?;
                delimiters.push(delimiter);
                part_start = delimiter_end;
                cursor = delimiter_end;
            }
            value if is_java_regex_whitespace(value) => {
                if let Some((delimiter_end, delimiter)) = word_delimiter(input, cursor, range.end) {
                    push_piece(input, part_start, cursor, &mut pieces)?;
                    delimiters.push(delimiter);
                    part_start = delimiter_end;
                    cursor = delimiter_end;
                } else {
                    cursor = next;
                }
            }
            _ => cursor = next,
        }
    }
    if delimiters.is_empty() {
        return None;
    }
    push_piece(input, part_start, range.end, &mut pieces)?;
    Some(RawExpressionList { pieces, delimiters })
}

fn push_piece(input: &str, start: usize, end: usize, pieces: &mut Vec<TextRange>) -> Option<()> {
    let source = input.get(start..end)?;
    let trimmed = java_trim_range(source);
    if trimmed.is_empty() {
        return None;
    }
    pieces.push(TextRange::new(start + trimmed.start, start + trimmed.end));
    Some(())
}

fn comma_delimiter(input: &str, comma: usize, end: usize) -> (usize, ExpressionListDelimiter) {
    let after_comma = comma + ','.len_utf8();
    let word_start = skip_whitespace(input, after_comma, end);
    let had_whitespace = word_start > after_comma;
    if had_whitespace
        && let Some((word_end, delimiter)) = list_operator_at(input, word_start, end)
        && word_end < end
        && input
            .get(word_end..end)
            .and_then(|rest| rest.chars().next())
            .is_some_and(is_java_regex_whitespace)
    {
        return (skip_whitespace(input, word_end, end), delimiter);
    }
    (word_start, ExpressionListDelimiter::Comma)
}

fn word_delimiter(
    input: &str,
    whitespace: usize,
    end: usize,
) -> Option<(usize, ExpressionListDelimiter)> {
    let word_start = skip_whitespace(input, whitespace, end);
    let (word_end, delimiter) = list_operator_at(input, word_start, end)?;
    if word_end >= end
        || !input
            .get(word_end..end)?
            .chars()
            .next()
            .is_some_and(is_java_regex_whitespace)
    {
        return None;
    }
    Some((skip_whitespace(input, word_end, end), delimiter))
}

fn list_operator_at(
    input: &str,
    start: usize,
    end: usize,
) -> Option<(usize, ExpressionListDelimiter)> {
    let remaining = input.get(start..end)?;
    for (word, delimiter) in [
        ("and", ExpressionListDelimiter::And),
        ("nor", ExpressionListDelimiter::Nor),
        ("or", ExpressionListDelimiter::Or),
    ] {
        if remaining
            .get(..word.len())
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(word))
        {
            return Some((start + word.len(), delimiter));
        }
    }
    None
}

fn skip_whitespace(input: &str, mut cursor: usize, end: usize) -> usize {
    while cursor < end {
        let Some(character) = input.get(cursor..end).and_then(|rest| rest.chars().next()) else {
            break;
        };
        if !is_java_regex_whitespace(character) {
            break;
        }
        cursor += character.len_utf8();
    }
    cursor
}

fn is_java_regex_whitespace(character: char) -> bool {
    matches!(
        character,
        ' ' | '\t' | '\n' | '\u{000B}' | '\u{000C}' | '\r'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(input: &str) -> RawExpressionList {
        split_expression_list(input, TextRange::new(0, input.len())).expect("list expected")
    }

    fn pieces<'a>(input: &'a str, list: &RawExpressionList) -> Vec<&'a str> {
        list.pieces
            .iter()
            .map(|range| range.slice(input).unwrap())
            .collect()
    }

    #[test]
    fn scans_only_top_level_delimiters() {
        let input = "\"a,b\" and {value::%1,2%} and (3 or 4)";
        let list = split(input);
        assert_eq!(
            pieces(input, &list),
            ["\"a,b\"", "{value::%1,2%}", "(3 or 4)"]
        );
        assert_eq!(
            list.conjunction_for_children(&[0, 1, 2]),
            ExpressionListConjunction::And
        );
    }

    #[test]
    fn preserves_comma_bearing_child_ranges_for_parser_growth() {
        let input = "spherical vector radius 1, yaw 45, pitch 90 and 2";
        let list = split(input);
        assert_eq!(
            pieces(input, &list),
            ["spherical vector radius 1", "yaw 45", "pitch 90", "2"]
        );
        assert_eq!(
            list.conjunction_for_children(&[0, 3]),
            ExpressionListConjunction::And
        );
    }

    #[test]
    fn comma_is_neutral_and_mixed_explicit_modes_become_and() {
        let list = split("1, 2 or 3");
        assert_eq!(
            list.conjunction_for_children(&[0, 1, 2]),
            ExpressionListConjunction::Or
        );
        let list = split("1 nor 2 or 3");
        assert_eq!(
            list.conjunction_for_children(&[0, 1, 2]),
            ExpressionListConjunction::And
        );
    }

    #[test]
    fn comma_requires_whitespace_before_absorbing_a_word_operator() {
        let input = "1,and 2";
        let list = split(input);
        assert_eq!(pieces(input, &list), ["1", "and 2"]);
        assert_eq!(list.delimiters, [ExpressionListDelimiter::Comma]);

        let input = "1, and 2";
        let list = split(input);
        assert_eq!(pieces(input, &list), ["1", "2"]);
        assert_eq!(list.delimiters, [ExpressionListDelimiter::And]);
    }

    #[test]
    fn non_breaking_space_is_not_regex_whitespace_for_list_delimiters() {
        assert!(
            split_expression_list(
                "1\u{00A0}and\u{00A0}2",
                TextRange::new(0, "1\u{00A0}and\u{00A0}2".len()),
            )
            .is_none()
        );
    }

    #[test]
    fn ascii_tab_is_regex_whitespace_for_list_delimiters() {
        let input = "1\tand\t2";
        let list = split(input);
        assert_eq!(pieces(input, &list), ["1", "2"]);
        assert_eq!(list.delimiters, [ExpressionListDelimiter::And]);
    }
}

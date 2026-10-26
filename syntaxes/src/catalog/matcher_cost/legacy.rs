//! Frozen pre-optimization sequence, for differential tests only.
use super::*;
pub(super) fn legacy_match_static_pattern(
    elements: &[SpannedPatternElement],
    input: &str,
) -> Option<StaticPatternMatch> {
    legacy_match_static_sequence(elements, input, vec![StaticPatternMatch::default()])
        .into_iter()
        .find(|state| state.cursor == input.len())
}

pub(super) fn legacy_match_static_sequence(
    elements: &[SpannedPatternElement],
    input: &str,
    mut states: Vec<StaticPatternMatch>,
) -> Vec<StaticPatternMatch> {
    for element in elements {
        let mut next = Vec::new();
        for state in states {
            match &element.value {
                PatternElement::Literal(literal) => {
                    if let Some(cursor) = match_static_literal(input, state.cursor, literal) {
                        next.push(StaticPatternMatch { cursor, ..state });
                    }
                }
                PatternElement::Choice(branches) => {
                    for branch in branches {
                        next.extend(legacy_match_static_sequence(branch, input, vec![state.clone()]));
                    }
                }
                PatternElement::Group(children) => {
                    next.extend(legacy_match_static_sequence(children, input, vec![state]));
                }
                PatternElement::Option(children) => {
                    next.push(state.clone());
                    next.extend(legacy_match_static_sequence(children, input, vec![state]));
                }
                PatternElement::ParseTag(tag) => {
                    let mut state = state;
                    if !tag.is_empty() {
                        state.tags.push(tag.clone());
                        if let Ok(mark) = tag.parse::<i32>() {
                            state.mark ^= mark;
                        }
                    }
                    next.push(state);
                }
                PatternElement::ParseMark(mark) => {
                    let mut state = state;
                    state.mark ^= mark;
                    next.push(state);
                }
                PatternElement::Empty => next.push(state),
                PatternElement::Regex(_) | PatternElement::TypeExpr(_) => {}
            }
            if next.len() >= MAX_STATIC_PATTERN_STATES {
                next.truncate(MAX_STATIC_PATTERN_STATES);
                break;
            }
        }
        states = next;
        if states.is_empty() {
            break;
        }
    }
    states
}


//! Data-driven parsing of Skript arithmetic expressions.
//!
//! Operator precedence and valid operand combinations come from the current
//! SSG snapshot. The resulting tree follows Skript's `ArithmeticChain`: the
//! last operator in the lowest-priority group becomes the root, which keeps
//! operators in the same group left-associative.

use crate::TextRange;
use crate::expression::{
    ExpressionCandidate, ExpressionExpectedType, ExpressionNode, ExpressionNodeKind,
    ExpressionParseEnvironment, ExpressionParseError, ExpressionSession,
};
use crate::pattern_match::{
    find_parenthesis_end, find_quote_end, find_variable_end, java_trim_range,
};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use syntaxes::{Multiplicity, Operation, Operator, Priority};

#[derive(Debug, Clone, Copy)]
struct OperatorOccurrence {
    operator_index: usize,
    start: usize,
    end: usize,
}

pub(crate) fn parse_arithmetic<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    range: TextRange,
    candidate_ends: &[usize],
    expected_types: &[ExpressionExpectedType],
    depth: usize,
) -> Result<Vec<ExpressionCandidate>, ExpressionParseError> {
    let operators = session.catalog().operators().to_vec();
    if operators.is_empty() {
        return Ok(Vec::new());
    }

    for end in candidate_ends.iter().copied() {
        let candidate_range = TextRange::new(range.start, end);
        let roots = root_candidates(
            session.source().virtual_source(),
            candidate_range,
            &operators,
        );
        for root in roots {
            let operator = &operators[root.operator_index];
            let mut operations = session
                .catalog()
                .operations()
                .get(&operator.sign)
                .cloned()
                .unwrap_or_default();
            operations.sort_by_key(|operation| operation.registration_order);

            for operation in operations {
                if !session.return_type_matches(Some(&operation.return_type), expected_types) {
                    continue;
                }
                session
                    .begin_semantic_candidate()
                    .map_err(environment_error)?;
                session
                    .begin_expression_candidate()
                    .map_err(environment_error)?;
                let parsed =
                    parse_operation(session, candidate_range, root, operator, &operation, depth);
                match parsed {
                    Ok(Some(mut candidate)) => {
                        candidate.node.effects = session
                            .defer_expression_candidate(true)
                            .map_err(environment_error)?;
                        session
                            .finish_semantic_candidate(true)
                            .map_err(environment_error)?;
                        return Ok(vec![candidate]);
                    }
                    Ok(None) => {
                        session
                            .defer_expression_candidate(false)
                            .map_err(environment_error)?;
                        session
                            .finish_semantic_candidate(false)
                            .map_err(environment_error)?;
                    }
                    Err(error) => {
                        session
                            .defer_expression_candidate(false)
                            .map_err(environment_error)?;
                        session
                            .finish_semantic_candidate(false)
                            .map_err(environment_error)?;
                        return Err(error);
                    }
                }
            }
        }
    }
    Ok(Vec::new())
}

fn parse_operation<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    range: TextRange,
    root: OperatorOccurrence,
    operator: &Operator,
    operation: &Operation,
    depth: usize,
) -> Result<Option<ExpressionCandidate>, ExpressionParseError> {
    let Some(left_range) = trimmed_range(
        session.source().virtual_source(),
        TextRange::new(range.start, root.start),
    ) else {
        return Ok(None);
    };
    let Some(right_range) = trimmed_range(
        session.source().virtual_source(),
        TextRange::new(root.end, range.end),
    ) else {
        return Ok(None);
    };

    let Some(left) = parse_operand(session, left_range, &operation.left, depth + 1)? else {
        return Ok(None);
    };
    session.select_expression(&left)?;
    let Some(right) = parse_operand(session, right_range, &operation.right, depth + 1)? else {
        return Ok(None);
    };
    session.select_expression(&right)?;

    let metadata = BTreeMap::from([
        ("arithmetic.operator".to_owned(), operator.sign.clone()),
        (
            "arithmetic.operation-registration-id".to_owned(),
            operation.registration_id.as_str().to_owned(),
        ),
        (
            "arithmetic.left-type".to_owned(),
            operation.left.as_str().to_owned(),
        ),
        (
            "arithmetic.right-type".to_owned(),
            operation.right.as_str().to_owned(),
        ),
        ("arithmetic.addon".to_owned(), operation.addon.name.clone()),
        (
            "arithmetic.addon-version".to_owned(),
            operation.addon.version.clone(),
        ),
    ]);
    Ok(Some(ExpressionCandidate {
        node: ExpressionNode {
            effects: None,
            kind: ExpressionNodeKind::Arithmetic {
                operator: operator.sign.clone(),
                operation_registration_id: operation.registration_id.as_str().to_owned(),
            },
            function: None,
            span: session.map_range(range)?,
            return_type: Some(operation.return_type.clone()),
            possible_return_types: vec![operation.return_type.clone()],
            possible_return_types_state: syntaxes::PossibleReturnTypesState::Complete,
            multiplicity: Some(Multiplicity::Single),
            captures: Vec::new(),
            tags: Vec::new(),
            mark: 0,
            children: vec![left, right],
            routed_captures: Vec::new(),
            public_data: Vec::new(),
            metadata,
        },
        expected_alternative: None,
    }))
}

fn parse_operand<E: ExpressionParseEnvironment>(
    session: &mut ExpressionSession<'_, E>,
    range: TextRange,
    expected_class: &syntaxes::ClassName,
    depth: usize,
) -> Result<Option<ExpressionNode>, ExpressionParseError> {
    let expected = [ExpressionExpectedType {
        class_name: expected_class.clone(),
        plural: false,
    }];
    let mut candidates =
        session.parse_prefixes(range, &[range.end], &expected, true, true, 0, depth)?;
    Ok((!candidates.is_empty()).then(|| candidates.remove(0).node))
}

fn root_candidates(
    input: &str,
    range: TextRange,
    operators: &[Operator],
) -> Vec<OperatorOccurrence> {
    let occurrences = operator_occurrences(input, range, operators);
    if occurrences.is_empty() {
        return Vec::new();
    }

    let mut ordered = (0..operators.len()).collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        compare_priority(&operators[*left].priority, &operators[*right].priority)
            .then_with(|| operators[*left].sign.cmp(&operators[*right].sign))
    });
    ordered.reverse();

    let mut groups: Vec<Vec<usize>> = Vec::new();
    for index in ordered {
        if let Some(group) = groups.last_mut()
            && compare_priority(&operators[group[0]].priority, &operators[index].priority)
                == Ordering::Equal
        {
            group.push(index);
        } else {
            groups.push(vec![index]);
        }
    }

    let mut roots = Vec::new();
    for group in groups {
        let mut matching = occurrences
            .iter()
            .copied()
            .filter(|occurrence| group.contains(&occurrence.operator_index))
            .collect::<Vec<_>>();
        matching.sort_by_key(|occurrence| std::cmp::Reverse(occurrence.start));
        roots.extend(matching);
    }
    roots
}

fn operator_occurrences(
    input: &str,
    range: TextRange,
    operators: &[Operator],
) -> Vec<OperatorOccurrence> {
    let mut occurrences = Vec::new();
    let mut cursor = range.start;
    while cursor < range.end {
        let Some(ch) = input
            .get(cursor..range.end)
            .and_then(|text| text.chars().next())
        else {
            break;
        };
        match ch {
            '"' => {
                let Some(end) = find_quote_end(input, cursor + ch.len_utf8(), range.end) else {
                    return Vec::new();
                };
                cursor = end + ch.len_utf8();
                continue;
            }
            '{' => {
                let Some(end) = find_variable_end(input, cursor + ch.len_utf8(), range.end) else {
                    return Vec::new();
                };
                cursor = end + '}'.len_utf8();
                continue;
            }
            '(' => {
                let Some(end) = find_parenthesis_end(input, cursor + ch.len_utf8(), range.end)
                else {
                    return Vec::new();
                };
                cursor = end + ')'.len_utf8();
                continue;
            }
            _ => {}
        }

        let tail = &input[cursor..range.end];
        let longest = operators
            .iter()
            .filter(|operator| !operator.sign.is_empty() && tail.starts_with(&operator.sign))
            .map(|operator| operator.sign.len())
            .max();
        if let Some(length) = longest {
            occurrences.extend(
                operators
                    .iter()
                    .enumerate()
                    .filter(|(_, operator)| {
                        operator.sign.len() == length && tail.starts_with(&operator.sign)
                    })
                    .map(|(operator_index, _)| OperatorOccurrence {
                        operator_index,
                        start: cursor,
                        end: cursor + length,
                    }),
            );
            cursor += length;
        } else {
            cursor += ch.len_utf8();
        }
    }
    occurrences
}

fn compare_priority(left: &Priority, right: &Priority) -> Ordering {
    if left == right {
        return Ordering::Equal;
    }
    if left.before.contains(right)
        || right.after.contains(left)
        || left
            .before
            .iter()
            .any(|left_before| right.after.contains(left_before))
    {
        return Ordering::Less;
    }
    if left.after.contains(right)
        || right.before.contains(left)
        || left
            .after
            .iter()
            .any(|left_after| right.before.contains(left_after))
    {
        return Ordering::Greater;
    }
    Ordering::Equal
}

fn trimmed_range(input: &str, range: TextRange) -> Option<TextRange> {
    let local = java_trim_range(range.slice(input)?);
    (!local.is_empty()).then(|| TextRange::new(range.start + local.start, range.start + local.end))
}

fn environment_error(message: String) -> ExpressionParseError {
    ExpressionParseError::Environment { message }
}

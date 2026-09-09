use super::{
    accept, annotate, child, child_span, mark_unresolved, matches, register_handler, reject_with,
};
use crate::catalog::{self, ComparatorContract, TypeRelation};
use crate::nlaocs::skript_parser_addon::types::{
    ConditionPayload, ExpressionExpectedType, ExpressionPossibleReturnTypesState, HookDecision,
    HookEffects, HookOutput, HookPayload, MetadataEntry, ParseRequest, ParseResult,
    ParseResultStatus, RegisteredExpressionChild, RegisteredSyntaxHandler,
};

const HANDLER_ID: &str = "core.condition.cond-compare";
const OBJECT: &str = "java.lang.Object";
const EXPRESSION_PARSER: &str = "host.expression";
const REPARSE_SECOND: u64 = 0x434f_4d50_0000_0001;
const REPARSE_FIRST: u64 = 0x434f_4d50_0000_0002;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ComparisonVerdict {
    Accepted,
    Rejected,
    Unresolved,
}

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(handlers, HANDLER_ID, ".CondCompare");
}

pub(super) fn resolve(
    mut payload: ConditionPayload,
    parse_results: &[ParseResult],
) -> Option<HookOutput> {
    if !matches(&payload, HANDLER_ID) {
        return None;
    }
    let mut reparse_unresolved = false;
    reparse_unresolved |= apply_reparse_result(&mut payload, 1, REPARSE_SECOND, parse_results);
    reparse_unresolved |= apply_reparse_result(&mut payload, 0, REPARSE_FIRST, parse_results);
    let Some(first) = child(&payload, 0) else {
        mark_unresolved(&mut payload, "comparison-return-type");
        return Some(accept(payload));
    };
    let Some(second) = child(&payload, 1) else {
        mark_unresolved(&mut payload, "comparison-return-type");
        return Some(accept(payload));
    };
    let (first_types, first_types_unresolved) = possible_types(first);
    let (right_types, right_types_unresolved, common_unresolved) = if let Some(third) =
        child(&payload, 2)
    {
        let (second_types, second_types_unresolved) = possible_types(second);
        let (third_types, third_types_unresolved) = possible_types(third);
        let (common_types, common_unresolved) = common_type_candidates(&second_types, &third_types);
        (
            common_types,
            second_types_unresolved || third_types_unresolved || common_unresolved,
            common_unresolved,
        )
    } else {
        let (second_types, second_types_unresolved) = possible_types(second);
        (second_types, second_types_unresolved, false)
    };
    if first_types.is_empty() || right_types.is_empty() {
        mark_unresolved(
            &mut payload,
            if common_unresolved {
                "comparison-common-type"
            } else {
                "comparison-return-type"
            },
        );
        return Some(accept(payload));
    }

    let relation = payload.candidate.pattern_index % 8;
    let requires_ordering = !matches!(relation, 4 | 5);
    let (contracts, catalog_unresolved) = comparator_candidates(&first_types, &right_types);
    let verdict = comparison_verdict(
        &contracts,
        requires_ordering,
        first_types_unresolved || right_types_unresolved || catalog_unresolved,
    );
    match verdict {
        ComparisonVerdict::Rejected => {
            if let Some(request) =
                next_reparse_request(&payload, &first_types, &right_types, parse_results)
            {
                return Some(request_reparse(payload, request));
            }
            if reparse_unresolved {
                mark_unresolved(&mut payload, "comparison-literal-reparse");
                return Some(accept(payload));
            }
            let first_display = display_types(&first_types);
            let right_display = display_types(&right_types);
            if requires_ordering && all_ordering_unsupported(&contracts) {
                return Some(reject_with(
                    if matches!(relation, 6 | 7) {
                        format!(
                            "cannot test whether {first_display} is between values of type {right_display}"
                        )
                    } else {
                        format!(
                            "the comparator between {first_display} and {right_display} does not support ordering"
                        )
                    },
                    "core.cond-compare.ordering-unsupported",
                    payload.candidate.span.clone(),
                ));
            }
            return Some(reject_with(
                format!("cannot compare {first_display} with {right_display}"),
                "core.cond-compare.incompatible-types",
                child_span(&payload, 1),
            ));
        }
        ComparisonVerdict::Unresolved => {
            let code = if first_types.iter().any(|type_name| type_name == OBJECT)
                || right_types.iter().any(|type_name| type_name == OBJECT)
            {
                "comparison-object-type"
            } else if common_unresolved {
                "comparison-common-type"
            } else if catalog_unresolved {
                "comparison-catalog"
            } else if first_types_unresolved || right_types_unresolved {
                "comparison-return-type"
            } else if requires_ordering
                && contracts.iter().any(|contract| {
                    matches!(contract.relation, TypeRelation::Compatible)
                        && contract.supports_ordering.is_none()
                })
            {
                "comparison-ordering"
            } else {
                "comparison-contract"
            };
            mark_unresolved(&mut payload, code);
        }
        ComparisonVerdict::Accepted => {}
    }
    annotate(&mut payload, "semantic-mode", "comparison");
    let mark_semantics = normalize_marks(payload.candidate.mark);
    annotate(
        &mut payload,
        "comparison-negated",
        if mark_semantics.negated {
            "true"
        } else {
            "false"
        },
    );
    annotate(
        &mut payload,
        "comparison-invert-right-lists",
        if mark_semantics.invert_right_lists {
            "true"
        } else {
            "false"
        },
    );
    annotate(
        &mut payload,
        "comparison-relation",
        match relation {
            0 => "greater",
            1 => "greater-or-equal",
            2 => "smaller",
            3 => "smaller-or-equal",
            4 | 5 => "equal",
            6 | 7 => "between",
            _ => unreachable!(),
        },
    );
    if let Some(registration_id) = unique_registration_id(&contracts) {
        annotate(&mut payload, "comparator-registration-id", registration_id);
    }
    Some(accept(payload))
}

fn next_reparse_request(
    payload: &ConditionPayload,
    first_types: &[String],
    right_types: &[String],
    results: &[ParseResult],
) -> Option<ParseRequest> {
    for (child_index, request_id, expected_types) in [
        (1, REPARSE_SECOND, first_types),
        (0, REPARSE_FIRST, right_types),
    ] {
        let expression = child(payload, child_index)?;
        if expression.kind != "literal"
            || results.iter().any(|result| {
                result.request_id == request_id && result.parser_id == EXPRESSION_PARSER
            })
        {
            continue;
        }
        let (current_types, _) = possible_types(expression);
        if current_types
            .iter()
            .any(|current| expected_types.iter().any(|expected| expected == current))
        {
            continue;
        }
        let expected_types = expected_types
            .iter()
            .filter(|class_name| class_name.as_str() != OBJECT)
            .map(|class_name| ExpressionExpectedType {
                class_name: class_name.clone(),
                plural: false,
            })
            .collect::<Vec<_>>();
        if expected_types.is_empty() {
            continue;
        }
        return Some(ParseRequest {
            request_id,
            parser_id: EXPRESSION_PARSER.to_owned(),
            input: expression.text.clone(),
            expected_types,
            span: child_span(payload, child_index),
            options: vec![MetadataEntry {
                key: "parse.mode".to_owned(),
                value: "literals-only".to_owned(),
                owner_component_id: None,
            }],
        });
    }
    None
}

fn request_reparse(payload: ConditionPayload, request: ParseRequest) -> HookOutput {
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Condition(payload)),
        effects: HookEffects {
            diagnostics: Vec::new(),
            context_updates: Vec::new(),
            parse_requests: vec![request],
            parse_results: Vec::new(),
        },
    }
}

fn apply_reparse_result(
    payload: &mut ConditionPayload,
    child_index: usize,
    request_id: u64,
    results: &[ParseResult],
) -> bool {
    let Some(result) = results
        .iter()
        .find(|result| result.request_id == request_id && result.parser_id == EXPRESSION_PARSER)
    else {
        return false;
    };
    if result.status == ParseResultStatus::Failed {
        return false;
    }
    let Some(summary) = result
        .roots
        .first()
        .and_then(|root| result.nodes.iter().find(|node| node.node_id == *root))
        .and_then(|node| node.summary.as_ref())
    else {
        return true;
    };
    if result.status != ParseResultStatus::Success || summary.return_type.is_none() {
        return true;
    }
    let Some(expression) = payload.candidate.children.get_mut(child_index) else {
        return true;
    };
    expression.return_type = summary.return_type.clone();
    expression.possible_return_types = summary.possible_return_types.clone();
    expression.possible_return_types_state = summary.possible_return_types_state;
    expression.multiplicity = summary.multiplicity;
    expression.metadata.extend(summary.metadata.clone());
    false
}

fn possible_types(child: &RegisteredExpressionChild) -> (Vec<String>, bool) {
    let mut types = child.possible_return_types.clone();
    let unresolved =
        child.possible_return_types_state != ExpressionPossibleReturnTypesState::Complete;
    if (types.is_empty() || unresolved)
        && let Some(return_type) = child.return_type.as_ref()
        && !types.contains(return_type)
    {
        types.push(return_type.clone());
    }
    types.sort();
    types.dedup();
    (types, unresolved)
}

fn common_type_candidates(left: &[String], right: &[String]) -> (Vec<String>, bool) {
    let mut candidates = Vec::new();
    let mut unresolved = false;
    for left_type in left {
        for right_type in right {
            if left_type == right_type {
                candidates.push(left_type.clone());
                continue;
            }
            match catalog::common_assignable_class(&[left_type.clone(), right_type.clone()]) {
                Ok(Some(common)) => {
                    if common == OBJECT {
                        unresolved = true;
                    }
                    candidates.push(common);
                }
                Ok(None) | Err(_) => unresolved = true,
            }
        }
    }
    candidates.sort();
    candidates.dedup();
    (candidates, unresolved)
}

fn comparator_candidates(
    first_types: &[String],
    right_types: &[String],
) -> (Vec<ComparatorContract>, bool) {
    let mut contracts = Vec::new();
    let mut unresolved = false;
    for first_type in first_types {
        for right_type in right_types {
            if first_type == OBJECT || right_type == OBJECT {
                unresolved = true;
                continue;
            }
            match catalog::comparator_for_types(first_type, right_type) {
                Ok(contract) => contracts.push(contract),
                Err(_) => unresolved = true,
            }
        }
    }
    (contracts, unresolved)
}

fn comparison_verdict(
    contracts: &[ComparatorContract],
    requires_ordering: bool,
    information_unresolved: bool,
) -> ComparisonVerdict {
    let mut accepted = false;
    let mut rejected = false;
    let mut unresolved = information_unresolved;
    for contract in contracts {
        match contract.relation {
            TypeRelation::Compatible if !requires_ordering => accepted = true,
            TypeRelation::Compatible => match contract.supports_ordering {
                Some(true) => accepted = true,
                Some(false) => rejected = true,
                None => unresolved = true,
            },
            TypeRelation::Incompatible => rejected = true,
            TypeRelation::Unknown => unresolved = true,
        }
    }
    if accepted && (rejected || unresolved) {
        ComparisonVerdict::Unresolved
    } else if accepted {
        ComparisonVerdict::Accepted
    } else if unresolved {
        ComparisonVerdict::Unresolved
    } else {
        ComparisonVerdict::Rejected
    }
}

fn all_ordering_unsupported(contracts: &[ComparatorContract]) -> bool {
    !contracts.is_empty()
        && contracts.iter().all(|contract| {
            matches!(contract.relation, TypeRelation::Compatible)
                && contract.supports_ordering == Some(false)
        })
}

fn display_types(types: &[String]) -> String {
    if types.is_empty() {
        "unknown".to_owned()
    } else {
        types.join(" | ")
    }
}

fn unique_registration_id(contracts: &[ComparatorContract]) -> Option<&str> {
    let mut registration_id = None;
    for contract in contracts {
        let id = contract.registration_id.as_deref()?;
        match registration_id {
            Some(existing) if existing != id => return None,
            None => registration_id = Some(id),
            Some(_) => {}
        }
    }
    registration_id
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MarkSemantics {
    negated: bool,
    invert_right_lists: bool,
}

fn normalize_marks(mark: i32) -> MarkSemantics {
    MarkSemantics {
        negated: (mark & 0x2 != 0) ^ (mark & 0x1 != 0),
        invert_right_lists: mark & 0x4 != 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ComparisonVerdict, MarkSemantics, comparison_verdict, normalize_marks, possible_types,
    };
    use crate::catalog::{ComparatorContract, TypeRelation};
    use crate::nlaocs::skript_parser_addon::types::{
        ExpressionPossibleReturnTypesState, RegisteredExpressionChild,
    };

    fn comparator(relation: TypeRelation, supports_ordering: Option<bool>) -> ComparatorContract {
        ComparatorContract {
            relation,
            supports_ordering,
            supports_inversion: None,
            registration_id: None,
            reversed: false,
        }
    }

    #[test]
    fn neither_and_not_marks_follow_skripts_toggle_order() {
        assert_eq!(
            normalize_marks(0),
            MarkSemantics {
                negated: false,
                invert_right_lists: false,
            }
        );
        assert!(normalize_marks(0x1).negated);
        assert!(normalize_marks(0x2).negated);
        assert!(!normalize_marks(0x1 | 0x2).negated);
        assert!(normalize_marks(0x4).invert_right_lists);
    }

    #[test]
    fn complete_possible_types_do_not_inherit_a_broad_return_type() {
        let child = RegisteredExpressionChild {
            default_expression: None,
            text: "value".to_owned(),
            kind: "registered-expression".to_owned(),
            parser_id: None,
            definition_id: None,
            registration_id: None,
            pattern_index: None,
            element_class: None,
            return_type: Some("java.lang.Object".to_owned()),
            possible_return_types: vec!["java.lang.String".to_owned()],
            possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
            multiplicity: None,
            public_data: Vec::new(),
            metadata: Vec::new(),
        };
        assert_eq!(
            possible_types(&child),
            (vec!["java.lang.String".to_owned()], false)
        );
    }

    #[test]
    fn only_definitive_comparator_failures_are_rejected() {
        assert_eq!(
            comparison_verdict(
                &[comparator(TypeRelation::Incompatible, None)],
                false,
                false,
            ),
            ComparisonVerdict::Rejected
        );
        assert_eq!(
            comparison_verdict(&[comparator(TypeRelation::Unknown, None)], false, false),
            ComparisonVerdict::Unresolved
        );
        assert_eq!(
            comparison_verdict(
                &[
                    comparator(TypeRelation::Compatible, Some(true)),
                    comparator(TypeRelation::Incompatible, None),
                ],
                true,
                false,
            ),
            ComparisonVerdict::Unresolved
        );
        assert_eq!(
            comparison_verdict(
                &[comparator(TypeRelation::Compatible, Some(false))],
                true,
                false,
            ),
            ComparisonVerdict::Rejected
        );
    }
}

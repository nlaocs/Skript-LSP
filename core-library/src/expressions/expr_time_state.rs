use super::{SemanticResolution, matches, metadata};
use crate::nlaocs::skript_parser_addon::types::{
    CaptureParserBinding, DynamicMultiplicity, ExpressionPossibleReturnTypesState, MetadataEntry,
    ParseResultStatus, RegisteredExpressionPayload, RegisteredSyntaxHandler,
    RegisteredSyntaxHandlerTarget, SyntaxKind,
};

const CLASS_SUFFIX: &str = ".ExprTimeState";
const PAST_HANDLER_ID: &str = "core.expression.expr-time-state.past";
const FUTURE_HANDLER_ID: &str = "core.expression.expr-time-state.future";
const EXPRESSION_PARSER: &str = "host.expression";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    let accepts_plural = crate::runtime::skript_at_least_patch(2, 9, 5).unwrap_or(false);
    handlers.push(handler(PAST_HANDLER_ID, &[0, 1], -1, accepts_plural));
    handlers.push(handler(FUTURE_HANDLER_ID, &[2, 3], 1, accepts_plural));
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    let time = if matches(payload, PAST_HANDLER_ID) {
        -1
    } else if matches(payload, FUTURE_HANDLER_ID) {
        1
    } else {
        return None;
    };
    Some(resolve_time_state(payload, time))
}

fn resolve_time_state(payload: &RegisteredExpressionPayload, time: i32) -> SemanticResolution {
    if payload
        .context
        .values
        .iter()
        .rfind(|entry| entry.key == "parser.delay-state")
        .is_some_and(|entry| entry.value.eq_ignore_ascii_case("true"))
    {
        return SemanticResolution::Reject(
            "time states cannot be used after the event has already passed".to_owned(),
        );
    }

    let Some(capture) = payload.parsed_captures.iter().find(|capture| {
        capture.capture_index == 0
            && capture.parser_id == EXPRESSION_PARSER
            && capture.status == ParseResultStatus::Success
    }) else {
        return SemanticResolution::Reject(format!(
            "the {} state expression could not be parsed",
            time_name(time)
        ));
    };
    let Some(summary) = capture.summary.as_ref() else {
        return SemanticResolution::Unresolved {
            reason: format!(
                "the {} state expression has no semantic summary",
                time_name(time)
            ),
            metadata: vec![metadata("semantic-mode", "time-state")],
        };
    };
    let Some(return_type) = summary.return_type.clone() else {
        return SemanticResolution::Unresolved {
            reason: format!(
                "the {} state expression has no resolved return type",
                time_name(time)
            ),
            metadata: vec![metadata("semantic-mode", "time-state")],
        };
    };

    let Some(multiplicity) = delegated_multiplicity(summary.multiplicity) else {
        return SemanticResolution::Unresolved {
            reason: format!(
                "the {} state expression multiplicity is unresolved",
                time_name(time)
            ),
            metadata: vec![metadata("semantic-mode", "time-state")],
        };
    };
    let possible_return_types = delegated_possible_return_types(
        &return_type,
        &summary.possible_return_types,
        summary.possible_return_types_state,
    );
    let mut output_metadata = summary.metadata.clone();
    output_metadata.push(metadata("semantic-mode", "time-state"));
    output_metadata.push(metadata("time-state", time_name(time)));
    SemanticResolution::Resolved {
        return_type,
        possible_return_types,
        possible_return_types_state: summary.possible_return_types_state,
        multiplicity,
        metadata: output_metadata,
    }
}

fn delegated_possible_return_types(
    return_type: &str,
    possible_return_types: &[String],
    state: ExpressionPossibleReturnTypesState,
) -> Vec<String> {
    if !possible_return_types.is_empty() {
        possible_return_types.to_vec()
    } else if state == ExpressionPossibleReturnTypesState::Complete {
        // ExprTimeState is a transparent WrapperExpression. As with
        // Expression's default possibleReturnTypes(), an empty complete list
        // means the single declared return type.
        vec![return_type.to_owned()]
    } else {
        Vec::new()
    }
}

fn delegated_multiplicity(
    multiplicity: Option<DynamicMultiplicity>,
) -> Option<DynamicMultiplicity> {
    // WrapperExpression delegates isSingle() to its child. Both is a valid
    // delegated result; only the absence of the child's result is unresolved.
    multiplicity
}

fn handler(
    handler_id: &str,
    pattern_indices: &[u64],
    time: i32,
    accepts_plural: bool,
) -> RegisteredSyntaxHandler {
    RegisteredSyntaxHandler {
        handler_id: handler_id.to_owned(),
        kind: SyntaxKind::Expression,
        phase: crate::nlaocs::skript_parser_addon::types::HookPhase::Expression,
        targets: vec![RegisteredSyntaxHandlerTarget::ClassSuffix(
            CLASS_SUFFIX.to_owned(),
        )],
        pattern_indices: pattern_indices.to_vec(),
        pattern_sources: Vec::new(),
        required_tags: Vec::new(),
        forbidden_tags: Vec::new(),
        marks: Vec::new(),
        capture_parsers: vec![CaptureParserBinding {
            capture_index: 0,
            parser_id: EXPRESSION_PARSER.to_owned(),
            required: true,
            options: vec![
                entry(
                    "expression.expected-types",
                    if accepts_plural {
                        "java.lang.Object[]"
                    } else {
                        "java.lang.Object"
                    },
                ),
                entry("expression.time-state", &time.to_string()),
            ],
        }],
        context_requirements: Vec::new(),
    }
}

fn time_name(time: i32) -> &'static str {
    if time < 0 { "past" } else { "future" }
}

fn entry(key: &str, value: &str) -> MetadataEntry {
    MetadataEntry {
        key: key.to_owned(),
        value: value.to_owned(),
        owner_component_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FUTURE_HANDLER_ID, PAST_HANDLER_ID, delegated_multiplicity,
        delegated_possible_return_types, handler,
    };
    use crate::nlaocs::skript_parser_addon::types::{
        DynamicMultiplicity, ExpressionPossibleReturnTypesState,
    };

    #[test]
    fn registrations_reparse_the_child_with_the_requested_time_state() {
        let past = handler(PAST_HANDLER_ID, &[0, 1], -1, false);
        let future = handler(FUTURE_HANDLER_ID, &[2, 3], 1, true);
        assert_eq!(past.pattern_indices, vec![0, 1]);
        assert_eq!(future.pattern_indices, vec![2, 3]);
        assert!(
            past.capture_parsers[0]
                .options
                .iter()
                .any(|entry| { entry.key == "expression.time-state" && entry.value == "-1" })
        );
        assert!(past.capture_parsers[0].options.iter().any(|entry| {
            entry.key == "expression.expected-types" && entry.value == "java.lang.Object"
        }));
        assert!(future.capture_parsers[0].options.iter().any(|entry| {
            entry.key == "expression.expected-types" && entry.value == "java.lang.Object[]"
        }));
        assert!(
            future.capture_parsers[0]
                .options
                .iter()
                .any(|entry| { entry.key == "expression.time-state" && entry.value == "1" })
        );
    }

    #[test]
    fn wrapper_uses_the_default_possible_type_only_for_complete_metadata() {
        assert_eq!(
            delegated_possible_return_types(
                "java.lang.String",
                &[],
                ExpressionPossibleReturnTypesState::Complete,
            ),
            ["java.lang.String"]
        );
        assert!(
            delegated_possible_return_types(
                "java.lang.String",
                &[],
                ExpressionPossibleReturnTypesState::Partial,
            )
            .is_empty()
        );
    }

    #[test]
    fn wrapper_preserves_explicit_both_and_does_not_invent_missing_data() {
        assert_eq!(
            delegated_multiplicity(Some(DynamicMultiplicity::Both)),
            Some(DynamicMultiplicity::Both)
        );
        assert_eq!(delegated_multiplicity(None), None);
    }
}

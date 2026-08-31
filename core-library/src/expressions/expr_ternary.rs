use super::{
    SemanticResolution, capture_parser, matches, metadata, register_handler,
    resolved_with_possible_types,
};
use crate::catalog;
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionPossibleReturnTypesState, ParseResultStatus,
    RegisteredExpressionPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".ExprTernary";
const HANDLER_ID: &str = "core.expression.expr-ternary";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(
        handlers,
        HANDLER_ID,
        CLASS_SUFFIX,
        vec![capture_parser(1, "host.condition")],
    );
}

pub(super) fn resolve(payload: &RegisteredExpressionPayload) -> Option<SemanticResolution> {
    matches(payload, HANDLER_ID).then(|| {
        if !payload.parsed_captures.iter().any(|capture| {
            capture.capture_index == 1
                && capture.parser_id == "host.condition"
                && capture.status == ParseResultStatus::Success
        }) {
            return SemanticResolution::Reject(
                "ternary Expression requires one parsed Condition".to_owned(),
            );
        }
        if payload.children.len() != 2 {
            return SemanticResolution::Reject(
                "ternary Expression requires two result Expressions".to_owned(),
            );
        }
        // ExprTernary.init() rejects `ifFalse instanceof ExprTernary || ifTrue instanceof
        // ExprTernary` before it computes the common return type. Registration identity is the
        // addon-neutral equivalent of Java's instanceof check; element_class is retained for
        // older/custom parsers that have not attached a registration ID.
        if payload.children.iter().any(is_ternary_child) {
            return SemanticResolution::Reject("ternary operators may not be nested".to_owned());
        }
        let mut possible_return_types = payload
            .children
            .iter()
            .flat_map(|child| child.possible_return_types.iter().cloned())
            .collect::<Vec<_>>();
        possible_return_types.sort();
        possible_return_types.dedup();
        let return_type = catalog::common_assignable_class(&possible_return_types)
            .ok()
            .flatten()
            .or_else(|| payload.common_child_return_type.clone());
        let Some(return_type) = return_type else {
            return SemanticResolution::Reject(
                "ternary result Expressions have no common return type".to_owned(),
            );
        };
        let multiplicity = if payload
            .children
            .iter()
            .all(|child| matches!(child.multiplicity, Some(DynamicMultiplicity::Single)))
        {
            DynamicMultiplicity::Single
        } else if payload
            .children
            .iter()
            .any(|child| matches!(child.multiplicity, Some(DynamicMultiplicity::Multiple)))
        {
            DynamicMultiplicity::Multiple
        } else {
            DynamicMultiplicity::Both
        };
        let possible_return_types_state = if payload.children.iter().any(|child| {
            child.possible_return_types_state == ExpressionPossibleReturnTypesState::Unresolved
        }) {
            ExpressionPossibleReturnTypesState::Unresolved
        } else if payload.children.iter().any(|child| {
            child.possible_return_types_state == ExpressionPossibleReturnTypesState::Partial
        }) {
            ExpressionPossibleReturnTypesState::Partial
        } else {
            ExpressionPossibleReturnTypesState::Complete
        };
        resolved_with_possible_types(
            return_type,
            possible_return_types,
            possible_return_types_state,
            multiplicity,
            vec![metadata("semantic-mode", "ternary-condition")],
        )
    })
}

fn is_ternary_child(
    child: &crate::nlaocs::skript_parser_addon::types::RegisteredExpressionChild,
) -> bool {
    child
        .registration_id
        .as_deref()
        .is_some_and(|registration_id| crate::runtime::handler_matches(HANDLER_ID, registration_id))
        || child
            .element_class
            .as_deref()
            .is_some_and(|element_class| element_class.ends_with(CLASS_SUFFIX))
}

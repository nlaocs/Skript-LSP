//! Intrinsic local-binding footprints of audited Skript 2.15.4 Conditions.
//! Children retain their own pre/post barriers. These promises cover no
//! comparator, containment handler, arbitrary converter or unknown callback.
use crate::nlaocs::skript_parser_addon::types::{
    ConditionCapture, ConditionPayload, DynamicMultiplicity, ExpressionPossibleReturnTypesState,
    RegisteredExpressionChild,
};

const CHANCE: &str = "ch.njol.skript.conditions.CondChance";
const EXISTS: &str = "ch.njol.skript.conditions.CondIsSet";
const MATCHES: &str = "ch.njol.skript.conditions.CondMatches";
const SCRIPT: &str = "ch.njol.skript.conditions.CondScriptLoaded";
const FINAL_NUMBERS: &[&str] = &[
    "java.lang.Byte",
    "java.lang.Short",
    "java.lang.Integer",
    "java.lang.Long",
    "java.lang.Float",
    "java.lang.Double",
];

pub(super) fn chance(payload: &mut ConditionPayload) {
    // check(): getSingle, final boxed Number.doubleValue, Math.random, booleans.
    // A general/custom Number may override doubleValue and is not certified.
    if expressions(payload, 1)
        && super::child(payload, 0).is_some_and(|child| {
            child.multiplicity == Some(DynamicMultiplicity::Single)
                && complete_types(child, FINAL_NUMBERS)
        })
    {
        mark(payload, CHANCE);
    }
}

pub(super) fn existence(payload: &mut ConditionPayload) {
    // check(): recursively getAll/getAnd, then array length and XOR. No value
    // conversion/comparison occurs; unknown child evaluation still clears facts.
    if expressions(payload, 1) {
        mark(payload, EXISTS);
    }
}

pub(super) fn string_matches(payload: &mut ConditionPayload) {
    // check(): two getAll/getAnd calls followed by final String/Pattern methods.
    // Require actual complete String results: no implicit conversion is assumed.
    if expressions(payload, 2)
        && payload
            .candidate
            .children
            .iter()
            .all(|child| complete_types(child, &["java.lang.String"]))
    {
        mark(payload, MATCHES);
    }
}

pub(super) fn current_script(payload: &mut ConditionPayload) {
    // The no-name branch queries the loaded Script set. The named branch invokes
    // Expression.check and deliberately does not receive this promise.
    if expressions(payload, 0) {
        mark(payload, SCRIPT);
    }
}

pub(super) fn event_flag(payload: &mut ConditionPayload) {
    if !expressions(payload, 0) {
        return;
    }
    let Some(class) = payload.candidate.element_class.as_deref() else {
        return;
    };
    let class = match class {
        "ch.njol.skript.conditions.CondElytraBoostConsume"
        | "org.skriptlang.skript.bukkit.fishing.elements.conditions.CondFishingLure"
        | "ch.njol.skript.conditions.CondLeashWillDrop"
        | "ch.njol.skript.conditions.CondRespawnLocation"
        | "ch.njol.skript.conditions.CondWillHatch" => class,
        "ch.njol.skript.conditions.CondIncendiary" if payload.candidate.pattern_index == 2 => class,
        _ => return,
    }
    .to_owned();
    // These exact branches read a primitive flag from their validated event.
    // Resource-pack state and entity/property-check branches are not included.
    mark(payload, &class);
}

fn expressions(payload: &ConditionPayload, count: usize) -> bool {
    payload.candidate.children.len() == count
        && payload.candidate.captures.len() == count
        && payload
            .candidate
            .captures
            .iter()
            .all(|capture| matches!(capture, ConditionCapture::Expression(_)))
        && (0..count).all(|index| super::child(payload, index).is_some())
}

fn complete_types(child: &RegisteredExpressionChild, allowed: &[&str]) -> bool {
    child.possible_return_types_state == ExpressionPossibleReturnTypesState::Complete
        && child
            .return_type
            .as_deref()
            .is_some_and(|value| allowed.contains(&value))
        && !child.possible_return_types.is_empty()
        && child
            .possible_return_types
            .iter()
            .all(|value| allowed.contains(&value.as_str()))
}

fn mark(payload: &mut ConditionPayload, expected_class: &str) {
    if crate::catalog::standard_condition_source(&payload.candidate, expected_class) {
        let typed_prefix: &[usize] = match expected_class {
            CHANCE | EXISTS => &[0],
            MATCHES => &[0, 1],
            _ => &[],
        };
        let Some(prefix) = typed_prefix
            .iter()
            .map(|index| super::child_capture_index(payload, *index))
            .collect::<Option<Vec<_>>>()
        else {
            return;
        };
        payload
            .candidate
            .metadata
            .push(crate::public_data::preserve_local_effects());
        payload
            .candidate
            .metadata
            .push(crate::public_data::operand_evaluation(&prefix));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nlaocs::skript_parser_addon::types::MappedSpan;
    fn child(class: &str) -> RegisteredExpressionChild {
        RegisteredExpressionChild {
            text: "opaque".into(),
            kind: "registered-expression".into(),
            parser_id: None,
            definition_id: None,
            registration_id: None,
            pattern_index: None,
            element_class: None,
            return_type: Some(class.into()),
            possible_return_types: vec![class.into()],
            possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
            multiplicity: Some(DynamicMultiplicity::Single),
            default_expression: None,
            public_data: Vec::new(),
            metadata: Vec::new(),
        }
    }
    #[test]
    fn condition_children_resolve_only_through_unique_native_slot_bindings() {
        use crate::nlaocs::skript_parser_addon::types::{
            ConditionCandidate, ConditionExpressionBinding, ConditionExpressionCapture,
            ConditionRegexCapture, ParseContext, TextRange, TypeCaptureState,
        };
        let span = MappedSpan {
            virtual_range: TextRange { start: 0, end: 1 },
            origins: Vec::new(),
        };
        let capture = |capture_index, resolution_id| {
            ConditionCapture::Expression(ConditionExpressionCapture {
                capture_index,
                state: TypeCaptureState::Explicit,
                pattern_span: TextRange { start: 0, end: 8 },
                expression: "%string%".into(),
                value: "x".into(),
                span: span.clone(),
                alternative_index: Some(0),
                resolution_id,
            })
        };
        let mut payload = ConditionPayload {
            input: "x".into(),
            context: ParseContext {
                syntax_context: 0,
                event_classes: Vec::new(),
                section_stack: Vec::new(),
                values: Vec::new(),
            },
            candidate: ConditionCandidate {
                definition_id: "test".into(),
                registration_id: "test".into(),
                element_class: None,
                priority: 0,
                registration_order: 0,
                pattern_index: 0,
                pattern: "test".into(),
                span: span.clone(),
                captures: vec![
                    ConditionCapture::Regex(ConditionRegexCapture {
                        capture_index: 3,
                        pattern_span: TextRange { start: 0, end: 3 },
                        value: "x".into(),
                        span: span.clone(),
                    }),
                    capture(7, None),
                    capture(9, Some("resolved".into())),
                ],
                tags: Vec::new(),
                mark: 0,
                marks: Vec::new(),
                handler: None,
                metadata: Vec::new(),
                children: vec![child("java.lang.String")],
                expression_bindings: vec![ConditionExpressionBinding {
                    capture_index: 9,
                    expression_index: 0,
                }],
            },
        };
        assert!(
            super::super::child(&payload, 0).is_none(),
            "missing resolution cannot shift the next child"
        );
        assert!(super::super::child(&payload, 1).is_some());
        assert_eq!(super::super::child_capture_index(&payload, 1), Some(9));
        let base = payload.clone();
        for (capture_index, expression_index) in [(9, 0), (7, 0), (3, 0), (9, 99)] {
            payload = base.clone();
            payload
                .candidate
                .expression_bindings
                .push(ConditionExpressionBinding {
                    capture_index,
                    expression_index,
                });
            assert!(
                super::super::child(&payload, 1).is_none(),
                "duplicate/bounds failure cannot fall back to child ordinal"
            );
        }
        payload = base;
        payload
            .candidate
            .captures
            .push(capture(9, Some("other".into())));
        assert!(super::super::child(&payload, 1).is_none());
    }

    #[test]
    fn chance_requires_final_boxed_numbers_not_open_number_or_partial_types() {
        for class in FINAL_NUMBERS {
            assert!(complete_types(&child(class), FINAL_NUMBERS));
        }
        for class in [
            "java.lang.Number",
            "addon.MutableNumber",
            "java.math.BigDecimal",
            "java.lang.String",
        ] {
            assert!(!complete_types(&child(class), FINAL_NUMBERS));
        }
        let mut value = child("java.lang.Double");
        value
            .possible_return_types
            .push("addon.MutableNumber".into());
        assert!(!complete_types(&value, FINAL_NUMBERS));
        let mut value = child("java.lang.Double");
        value.possible_return_types_state = ExpressionPossibleReturnTypesState::Partial;
        assert!(!complete_types(&value, FINAL_NUMBERS));
        value.possible_return_types_state = ExpressionPossibleReturnTypesState::Unresolved;
        assert!(!complete_types(&value, FINAL_NUMBERS));
    }
    #[test]
    fn string_matching_does_not_certify_object_to_string_conversion() {
        assert!(complete_types(
            &child("java.lang.String"),
            &["java.lang.String"]
        ));
        assert!(!complete_types(
            &child("java.lang.Object"),
            &["java.lang.String"]
        ));
        let mut value = child("java.lang.String");
        value.possible_return_types.clear();
        assert!(!complete_types(&value, &["java.lang.String"]));
    }
}

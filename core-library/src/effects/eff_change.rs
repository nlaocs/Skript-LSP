use crate::nlaocs::skript_parser_addon::types::{
    Diagnostic, DiagnosticSeverity, DynamicMultiplicity, EffectPayload,
    ExpressionPossibleReturnTypesState, HookDecision, HookOutput, HookPayload, MappedSpan,
    ParseSummary, ParsedCapture, RegisteredSyntaxHandler, RegisteredSyntaxHandlerTarget, Rejection,
    SyntaxKind,
};
use crate::{
    catalog::{self, ChangeContract},
    empty_effects,
};

const CLASS_SUFFIX: &str = ".EffChange";
const HANDLER_ID: &str = "core.effect.eff-change";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChangeMode {
    Add,
    Set,
    RemoveAll,
    Remove,
    Delete,
    Reset,
}

impl ChangeMode {
    fn catalog_key(self) -> &'static str {
        match self {
            Self::Add => "ADD",
            Self::Set => "SET",
            Self::RemoveAll => "REMOVE_ALL",
            Self::Remove => "REMOVE",
            Self::Delete => "DELETE",
            Self::Reset => "RESET",
        }
    }

    fn diagnostic_name(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Set => "set",
            Self::RemoveAll => "remove-all",
            Self::Remove => "remove",
            Self::Delete => "delete",
            Self::Reset => "reset",
        }
    }

    fn incompatible_message(self, source_type: &str, expected: &str) -> String {
        match self {
            Self::Add => format!(
                "cannot add {source_type} to this Expression; expected a value convertible to {expected}"
            ),
            Self::Set => format!(
                "cannot set this Expression to {source_type}; expected a value convertible to {expected}"
            ),
            Self::Remove | Self::RemoveAll => format!(
                "cannot remove {source_type} from this Expression; expected a value convertible to {expected}"
            ),
            Self::Delete | Self::Reset => {
                unreachable!("DELETE and RESET do not have a changer Expression")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChangeOperation {
    mode: ChangeMode,
    changed_capture: u64,
    changer_capture: Option<u64>,
}

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    handlers.push(RegisteredSyntaxHandler {
        handler_id: HANDLER_ID.to_owned(),
        kind: SyntaxKind::Effect,
        phase: crate::nlaocs::skript_parser_addon::types::HookPhase::Effect,
        targets: vec![RegisteredSyntaxHandlerTarget::ClassSuffix(
            CLASS_SUFFIX.to_owned(),
        )],
        pattern_indices: Vec::new(),
        pattern_sources: Vec::new(),
        required_tags: Vec::new(),
        forbidden_tags: Vec::new(),
        marks: Vec::new(),
        capture_parsers: Vec::new(),
        context_requirements: Vec::new(),
    });
}

pub(super) fn resolve(payload: EffectPayload) -> Option<HookOutput> {
    let candidate = payload.candidate.as_ref()?;
    if !crate::runtime::handler_matches(HANDLER_ID, &candidate.registration_id) {
        return None;
    }
    let Some(operation) = change_operation(candidate.pattern_index, &candidate.pattern) else {
        return Some(continue_with_warning(
            payload.clone(),
            "core.eff-change.unknown-pattern",
            "this EffChange pattern is not known to CoreLibrary, so its change contract was not verified",
            candidate.span.clone(),
        ));
    };

    // EffChange.init() first maps the matched pattern to ChangeMode and then swaps exprs[0]/exprs[1]
    // for the source-first ADD, REMOVE, and REMOVE_ALL forms. `change_operation` mirrors that Java
    // switch so every mode below can use the same changed/changer names as Skript.
    let changed = parsed_capture(candidate, operation.changed_capture);
    let changer = operation
        .changer_capture
        .and_then(|capture_index| parsed_capture(candidate, capture_index));

    // TODO: Once variable types are tracked across assignments, validate every variable change
    // against its established type here. This check only uses cardinality already present in the AST.
    let sets_multiple_into_single_variable = operation.mode == ChangeMode::Set
        && changed
            .and_then(|capture| capture.summary.as_ref())
            .is_some_and(|summary| {
                summary.kind == "variable"
                    && summary.multiplicity == Some(DynamicMultiplicity::Single)
            })
        && changer
            .and_then(|capture| capture.summary.as_ref())
            .is_some_and(|summary| summary.multiplicity == Some(DynamicMultiplicity::Multiple));

    if sets_multiple_into_single_variable {
        let message = "a single variable can only be set to one value, not more";
        return Some(reject_with(
            message,
            "core.eff-change.multiple-to-single-variable",
            changer
                .expect("the rejected candidate has a changer")
                .span
                .clone(),
        ));
    }

    if let Some(output) = validate_expression_change(&payload, operation.mode, changed, changer) {
        return Some(output);
    }
    Some(continue_with(payload))
}

fn parsed_capture(
    candidate: &crate::nlaocs::skript_parser_addon::types::EffectCandidate,
    capture_index: u64,
) -> Option<&ParsedCapture> {
    candidate
        .parsed_captures
        .iter()
        .find(|capture| capture.capture_index == capture_index)
}

fn validate_expression_change(
    payload: &EffectPayload,
    mode: ChangeMode,
    changed: Option<&ParsedCapture>,
    changer: Option<&ParsedCapture>,
) -> Option<HookOutput> {
    let changed = changed?;
    let changed_summary = changed.summary.as_ref()?;
    if changed_summary.kind == "variable" {
        return None;
    }
    if super::context_bool(&payload.context, super::DELAY_STATE_KEY) == Some(true)
        && has_event_value_changer(changed_summary, mode)
    {
        return Some(reject_with(
            "event values cannot be changed after the event has already passed",
            "core.eff-change.delayed-event-value",
            changed.span.clone(),
        ));
    }
    let contract_subject = changed_summary
        .registration_id
        .as_deref()
        .unwrap_or(changed.parser_id.as_str());
    let metadata_contract =
        match catalog::change_contract_from_metadata(&changed_summary.metadata, contract_subject) {
            Ok(contract) => contract,
            Err(reason) => {
                return Some(continue_with_warning(
                    payload.clone(),
                    "core.eff-change.conflicting-change-contract",
                    &format!("could not use the Expression's published change contract: {reason}"),
                    changed.span.clone(),
                ));
            }
        };
    let contract = match metadata_contract {
        Some(contract) => contract,
        None => match source_change_contract(changed_summary) {
            Ok(Some(contract)) => contract,
            Ok(None) => {
                return Some(continue_with_warning(
                    payload.clone(),
                    "core.eff-change.missing-change-contract",
                    "this parsed Expression does not publish enough data to resolve its change contract",
                    changed.span.clone(),
                ));
            }
            Err(reason) => {
                return Some(continue_with_warning(
                    payload.clone(),
                    "core.eff-change.change-contract-unavailable",
                    &format!("could not read this Expression's change contract: {reason}"),
                    changed.span.clone(),
                ));
            }
        },
    };
    let ChangeContract::Resolved { modes } = contract else {
        return Some(continue_with_warning(
            payload.clone(),
            "core.eff-change.unresolved-change-contract",
            &format!(
                "SSG could not resolve whether this Expression accepts {}",
                mode.catalog_key()
            ),
            changed.span.clone(),
        ));
    };
    let Some(accepted_types) = modes.get(mode.catalog_key()) else {
        let message = format!(
            "this Expression does not accept the {} change mode",
            mode.catalog_key()
        );
        return Some(reject_with(
            &message,
            &format!("core.eff-change.unsupported-{}", mode.diagnostic_name()),
            changed.span.clone(),
        ));
    };

    // Skript returns immediately after acceptChange(mode) for DELETE and RESET because those modes
    // have no delta Expression. The presence of the mode in the contract is therefore sufficient.
    let changer = changer?;
    let Some(changer_summary) = changer.summary.as_ref() else {
        return Some(continue_with_warning(
            payload.clone(),
            "core.eff-change.unresolved-input-type",
            &format!(
                "the value Expression's type is unresolved, so {} compatibility is unknown",
                mode.catalog_key()
            ),
            changer.span.clone(),
        ));
    };
    let source_types = source_types(changer_summary);
    let Some(source_type) = changer_summary.return_type.as_deref() else {
        return Some(continue_with_warning(
            payload.clone(),
            "core.eff-change.unresolved-input-type",
            &format!(
                "the value Expression's type is unresolved, so {} compatibility is unknown",
                mode.catalog_key()
            ),
            changer.span.clone(),
        ));
    };

    // This is the static equivalent of EffChange.init() flattening array classes, calling
    // getConvertedExpression(flatAcceptedTypes), and then checking canBeSingle(). SSG has already
    // flattened each accepted type into (class_name, multiple), while the host owns conversions.
    let compatibility = (|| {
        let mut compatible = Vec::new();
        let mut unknown = Vec::new();
        for accepted in accepted_types {
            let mut accepted_unknown = false;
            let mut accepted_compatible = false;
            for candidate in &source_types {
                match catalog::can_convert(candidate, &accepted.class_name)? {
                    catalog::TypeRelation::Compatible => accepted_compatible = true,
                    catalog::TypeRelation::Incompatible => {}
                    catalog::TypeRelation::Unknown => accepted_unknown = true,
                }
            }
            if accepted_compatible {
                compatible.push(accepted);
            } else if accepted_unknown {
                unknown.push(accepted);
            }
        }
        Ok::<_, String>((compatible, unknown))
    })();
    match compatibility {
        Ok((compatible, unknown)) if compatible.is_empty() && !unknown.is_empty() => {
            Some(continue_with_warning(
                payload.clone(),
                "core.eff-change.unknown-type-relation",
                &format!(
                    "SSG does not contain every Java class needed to verify this {} operation",
                    mode.catalog_key()
                ),
                changer.span.clone(),
            ))
        }
        Ok((compatible, unknown)) if compatible.is_empty() && unknown.is_empty() => {
            let expected = accepted_types
                .iter()
                .map(catalog::AcceptedChangeType::display_name)
                .collect::<Vec<_>>()
                .join(", ");
            let expected = if expected.is_empty() {
                "no value type".to_owned()
            } else {
                expected
            };
            let source_description = if source_types.len() == 1 {
                source_type.to_owned()
            } else {
                source_types.join(" | ")
            };
            let message = mode.incompatible_message(&source_description, &expected);
            Some(reject_with(
                &message,
                &format!(
                    "core.eff-change.incompatible-{}-type",
                    mode.diagnostic_name()
                ),
                changer.span.clone(),
            ))
        }
        Ok((compatible, unknown))
            if changer_summary.multiplicity == Some(DynamicMultiplicity::Multiple)
                && compatible.iter().all(|accepted| !accepted.multiple)
                && unknown.iter().any(|accepted| accepted.multiple) =>
        {
            Some(continue_with_warning(
                payload.clone(),
                "core.eff-change.unknown-multiple-type-relation",
                &format!(
                    "SSG lacks a Java class relation that may allow this multiple {} value",
                    mode.catalog_key()
                ),
                changer.span.clone(),
            ))
        }
        Ok((compatible, _))
            if changer_summary.multiplicity == Some(DynamicMultiplicity::Multiple)
                && compatible.iter().all(|accepted| !accepted.multiple) =>
        {
            let expected = compatible
                .iter()
                .map(|accepted| accepted.class_name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let message = format!(
                "this Expression accepts only one {expected} for {}, but the value Expression is always multiple",
                mode.catalog_key()
            );
            Some(reject_with(
                &message,
                &format!(
                    "core.eff-change.multiple-to-single-{}-expression",
                    mode.diagnostic_name()
                ),
                changer.span.clone(),
            ))
        }
        Ok((compatible, _))
            if changer_summary.multiplicity.is_none()
                && compatible.iter().all(|accepted| !accepted.multiple) =>
        {
            Some(continue_with_warning(
                payload.clone(),
                "core.eff-change.unresolved-input-multiplicity",
                &format!(
                    "the value Expression's multiplicity is unresolved, so {} cardinality is unknown",
                    mode.catalog_key()
                ),
                changer.span.clone(),
            ))
        }
        Ok(_) => None,
        Err(reason) => Some(continue_with_warning(
            payload.clone(),
            "core.eff-change.type-relation-unavailable",
            &format!(
                "could not verify {} type compatibility: {reason}",
                mode.catalog_key()
            ),
            changer.span.clone(),
        )),
    }
}

fn has_event_value_changer(summary: &ParseSummary, mode: ChangeMode) -> bool {
    super::metadata_value(&summary.metadata, "event-value-changer-modes").is_some_and(|value| {
        value
            .split(';')
            .any(|candidate| candidate == mode.catalog_key())
    })
}

pub(super) fn source_change_contract(
    summary: &ParseSummary,
) -> Result<Option<ChangeContract>, String> {
    let registered_contract = summary
        .registration_id
        .as_deref()
        .map(catalog::expression_change_contract)
        .transpose()?
        .flatten();
    if !can_use_type_change_fallback(
        &summary.kind,
        summary.element_class.as_deref(),
        registered_contract.as_ref(),
        catalog::is_class_assignable,
    )? {
        return Ok(registered_contract);
    }
    let Some(return_type) = summary.return_type.as_deref() else {
        return Ok(None);
    };
    // SimpleExpression and SimpleLiteral delegate acceptChange to the closest registered
    // ClassInfo changer. This also covers arithmetic and Function-call Expressions.
    catalog::type_change_contract(return_type)
}

fn can_use_type_change_fallback(
    kind: &str,
    element_class: Option<&str>,
    registered_contract: Option<&ChangeContract>,
    mut relation: impl FnMut(&str, &str) -> Result<catalog::TypeRelation, String>,
) -> Result<bool, String> {
    if registered_contract.is_some() || matches!(kind, "expression-list" | "variable") {
        return Ok(false);
    }
    let Some(element_class) = element_class else {
        // Native parser nodes such as literals, arithmetic, and Function calls mirror
        // Skript Expressions that delegate changes to their return ClassInfo.
        return Ok(true);
    };
    for base in [
        "ch.njol.skript.lang.util.SimpleExpression",
        "ch.njol.skript.lang.util.SimpleLiteral",
    ] {
        if relation(element_class, base)? == catalog::TypeRelation::Compatible {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn source_types(summary: &ParseSummary) -> Vec<&str> {
    let mut types = summary
        .possible_return_types
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    if (types.is_empty()
        || summary.possible_return_types_state != ExpressionPossibleReturnTypesState::Complete)
        && let Some(return_type) = summary.return_type.as_deref()
        && !types.contains(&return_type)
    {
        types.push(return_type);
    }
    types
}

fn change_operation(pattern_index: u64, pattern: &str) -> Option<ChangeOperation> {
    let (mode, changed_capture, changer_capture, known_patterns): (_, _, _, &[&str]) =
        match pattern_index {
            0 => (
                ChangeMode::Add,
                1,
                Some(0),
                &["(add|give) %objects% to %~objects%"],
            ),
            1 => (
                ChangeMode::Add,
                0,
                Some(1),
                &["increase %~objects% by %objects%"],
            ),
            2 => (ChangeMode::Add, 0, Some(1), &["give %~objects% %objects%"]),
            3 => (
                ChangeMode::Set,
                0,
                Some(1),
                &["set %~objects% to %objects%"],
            ),
            4 => (
                ChangeMode::RemoveAll,
                1,
                Some(0),
                &["remove (all|every) %objects% from %~objects%"],
            ),
            5 => (
                ChangeMode::Remove,
                1,
                Some(0),
                &["(remove|subtract) %objects% from %~objects%"],
            ),
            6 => (
                ChangeMode::Remove,
                0,
                Some(1),
                &[
                    "reduce %~objects% by %objects%",
                    "(reduce|decrease) %~objects% by %objects%",
                ],
            ),
            7 => (ChangeMode::Delete, 0, None, &["(delete|clear) %~objects%"]),
            8 => (ChangeMode::Reset, 0, None, &["reset %~objects%"]),
            _ => return None,
        };
    known_patterns
        .contains(&pattern.trim())
        .then_some(ChangeOperation {
            mode,
            changed_capture,
            changer_capture,
        })
}

fn reject_with(message: &str, code: &str, span: MappedSpan) -> HookOutput {
    HookOutput {
        decision: HookDecision::Reject(Rejection {
            reason: message.to_owned(),
            diagnostics: vec![Diagnostic {
                code: code.to_owned(),
                message: message.to_owned(),
                severity: DiagnosticSeverity::Error,
                span,
                related: Vec::new(),
            }],
        }),
        replacement: None,
        effects: empty_effects(),
    }
}

fn continue_with_warning(
    payload: EffectPayload,
    code: &str,
    message: &str,
    span: MappedSpan,
) -> HookOutput {
    let mut output = continue_with(payload);
    output.effects.diagnostics.push(Diagnostic {
        code: code.to_owned(),
        message: message.to_owned(),
        severity: DiagnosticSeverity::Warning,
        span,
        related: Vec::new(),
    });
    output
}

fn continue_with(payload: EffectPayload) -> HookOutput {
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Effect(payload)),
        effects: empty_effects(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ChangeMode, ChangeOperation, can_use_type_change_fallback, change_operation};
    use crate::catalog::{ChangeContract, TypeRelation};

    #[test]
    fn maps_every_skript_change_pattern_to_its_mode_and_capture_roles() {
        let cases = [
            (
                0,
                "(add|give) %objects% to %~objects%",
                ChangeMode::Add,
                1,
                Some(0),
            ),
            (
                1,
                "increase %~objects% by %objects%",
                ChangeMode::Add,
                0,
                Some(1),
            ),
            (2, "give %~objects% %objects%", ChangeMode::Add, 0, Some(1)),
            (
                3,
                "set %~objects% to %objects%",
                ChangeMode::Set,
                0,
                Some(1),
            ),
            (
                4,
                "remove (all|every) %objects% from %~objects%",
                ChangeMode::RemoveAll,
                1,
                Some(0),
            ),
            (
                5,
                "(remove|subtract) %objects% from %~objects%",
                ChangeMode::Remove,
                1,
                Some(0),
            ),
            (
                6,
                "(reduce|decrease) %~objects% by %objects%",
                ChangeMode::Remove,
                0,
                Some(1),
            ),
            (7, "(delete|clear) %~objects%", ChangeMode::Delete, 0, None),
            (8, "reset %~objects%", ChangeMode::Reset, 0, None),
        ];

        for (pattern_index, pattern, mode, changed_capture, changer_capture) in cases {
            assert_eq!(
                change_operation(pattern_index, pattern),
                Some(ChangeOperation {
                    mode,
                    changed_capture,
                    changer_capture
                })
            );
        }
    }

    #[test]
    fn recognizes_the_legacy_reduce_pattern_but_not_an_unrelated_pattern() {
        assert_eq!(
            change_operation(6, "reduce %~objects% by %objects%")
                .expect("Skript 2.6.4 reduce pattern")
                .mode,
            ChangeMode::Remove
        );
        assert!(change_operation(3, "set fire to rain").is_none());
        assert!(change_operation(99, "set %~objects% to %objects%").is_none());
    }

    #[test]
    fn only_missing_registered_contracts_can_use_the_type_fallback() {
        let resolved_empty = ChangeContract::Resolved {
            modes: std::collections::BTreeMap::new(),
        };
        let unresolved = ChangeContract::Unresolved;

        let relation = |source: &str, target: &str| {
            Ok(
                if source == "example.Simple" && target.ends_with("SimpleExpression") {
                    TypeRelation::Compatible
                } else {
                    TypeRelation::Incompatible
                },
            )
        };

        assert!(can_use_type_change_fallback("literal", None, None, relation).unwrap());
        assert!(
            can_use_type_change_fallback(
                "registered-expression",
                Some("example.Simple"),
                None,
                relation,
            )
            .unwrap()
        );
        assert!(
            !can_use_type_change_fallback(
                "registered-expression",
                Some("example.Custom"),
                None,
                relation,
            )
            .unwrap()
        );
        assert!(
            !can_use_type_change_fallback(
                "simple-expression",
                None,
                Some(&resolved_empty),
                relation,
            )
            .unwrap()
        );
        assert!(
            !can_use_type_change_fallback("simple-expression", None, Some(&unresolved), relation,)
                .unwrap()
        );
        assert!(!can_use_type_change_fallback("expression-list", None, None, relation).unwrap());
        assert!(!can_use_type_change_fallback("variable", None, None, relation).unwrap());
    }
}

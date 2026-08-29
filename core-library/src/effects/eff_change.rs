use crate::nlaocs::skript_parser_addon::types::{
    Diagnostic, DiagnosticSeverity, DynamicMultiplicity, EffectPayload, HookDecision, HookOutput,
    HookPayload, RegisteredSyntaxHandler, RegisteredSyntaxHandlerTarget, Rejection, SyntaxKind,
};
use crate::{
    catalog::{self, ChangeContract},
    empty_effects,
};

const CLASS_SUFFIX: &str = ".EffChange";
const HANDLER_ID: &str = "core.effect.eff-change";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    handlers.push(RegisteredSyntaxHandler {
        handler_id: HANDLER_ID.to_owned(),
        kind: SyntaxKind::Effect,
        target: RegisteredSyntaxHandlerTarget::ClassSuffix(CLASS_SUFFIX.to_owned()),
        capture_parsers: Vec::new(),
        context_requirements: Vec::new(),
    });
}

pub(super) fn resolve(payload: EffectPayload) -> Option<HookOutput> {
    let candidate = payload.candidate.as_ref()?;
    if !crate::runtime::handler_matches(HANDLER_ID, &candidate.registration_id) {
        return None;
    }
    if !is_set_pattern(&candidate.pattern) {
        return Some(continue_with(payload));
    }

    // TODO: Once variable types are tracked across assignments, validate SET values against the
    // variable's established type here. For now, only check facts available from the Expressions.
    let changed = candidate
        .parsed_captures
        .iter()
        .find(|capture| capture.capture_index == 0);
    let changer = candidate
        .parsed_captures
        .iter()
        .find(|capture| capture.capture_index == 1);
    let sets_multiple_into_single_variable = changed
        .and_then(|capture| capture.summary.as_ref())
        .is_some_and(|summary| {
            summary.kind == "variable" && summary.multiplicity == Some(DynamicMultiplicity::Single)
        })
        && changer
            .and_then(|capture| capture.summary.as_ref())
            .is_some_and(|summary| summary.multiplicity == Some(DynamicMultiplicity::Multiple));

    if !sets_multiple_into_single_variable {
        if let Some(output) = validate_expression_set(&payload, changed, changer) {
            return Some(output);
        }
        return Some(continue_with(payload));
    }

    let message = "a single variable can only be set to one value, not more";
    Some(HookOutput {
        decision: HookDecision::Reject(Rejection {
            reason: message.to_owned(),
            diagnostics: vec![Diagnostic {
                code: "core.eff-change.multiple-to-single-variable".to_owned(),
                message: message.to_owned(),
                severity: DiagnosticSeverity::Error,
                span: changer
                    .expect("the rejected candidate has a changer")
                    .span
                    .clone(),
                related: Vec::new(),
            }],
        }),
        replacement: None,
        effects: empty_effects(),
    })
}

fn validate_expression_set(
    payload: &EffectPayload,
    changed: Option<&crate::nlaocs::skript_parser_addon::types::ParsedCapture>,
    changer: Option<&crate::nlaocs::skript_parser_addon::types::ParsedCapture>,
) -> Option<HookOutput> {
    let changed = changed?;
    let changed_summary = changed
        .summary
        .as_ref()
        .filter(|summary| summary.kind == "registered-expression")?;
    let Some(registration_id) = changed_summary.registration_id.as_deref() else {
        return Some(continue_with_warning(
            payload.clone(),
            "core.eff-change.missing-registration-id",
            "the registered Expression has no registration ID, so its change contract cannot be verified",
            changed.span.clone(),
        ));
    };
    let metadata_contract =
        match catalog::change_contract_from_metadata(&changed_summary.metadata, registration_id) {
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
        None => match catalog::expression_change_contract(registration_id) {
            Ok(Some(contract)) => contract,
            Ok(None) => {
                return Some(continue_with_warning(
                    payload.clone(),
                    "core.eff-change.missing-change-contract",
                    "the source Catalog has no change contract for this Expression",
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
            "SSG could not resolve whether this Expression accepts SET",
            changed.span.clone(),
        ));
    };
    let Some(accepted_types) = modes.get("SET") else {
        let message = "this Expression does not accept the SET change mode";
        return Some(reject_with(
            message,
            "core.eff-change.unsupported-set",
            changed.span.clone(),
        ));
    };
    let changer = changer?;
    let Some(source_type) = changer
        .summary
        .as_ref()
        .and_then(|summary| summary.return_type.as_deref())
    else {
        return Some(continue_with_warning(
            payload.clone(),
            "core.eff-change.unresolved-input-type",
            "the value Expression's type is unresolved, so SET compatibility is unknown",
            changer.span.clone(),
        ));
    };

    let compatibility = (|| {
        let mut compatible = Vec::new();
        let mut unknown = Vec::new();
        for accepted in accepted_types {
            match catalog::can_convert(source_type, &accepted.class_name)? {
                catalog::TypeRelation::Compatible => compatible.push(accepted),
                catalog::TypeRelation::Incompatible => {}
                catalog::TypeRelation::Unknown => unknown.push(accepted),
            }
        }
        Ok::<_, String>((compatible, unknown))
    })();
    match compatibility {
        Ok((compatible, unknown)) if compatible.is_empty() && !unknown.is_empty() => {
            Some(continue_with_warning(
                payload.clone(),
                "core.eff-change.unknown-type-relation",
                "SSG does not contain every Java class needed to verify this SET operation",
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
            let message = format!(
                "cannot set this Expression to {source_type}; expected a value convertible to {expected}"
            );
            Some(reject_with(
                &message,
                "core.eff-change.incompatible-set-type",
                changer.span.clone(),
            ))
        }
        Ok((compatible, unknown))
            if changer.summary.as_ref().is_some_and(|summary| {
                summary.multiplicity == Some(DynamicMultiplicity::Multiple)
            }) && compatible.iter().all(|accepted| !accepted.multiple)
                && unknown.iter().any(|accepted| accepted.multiple) =>
        {
            Some(continue_with_warning(
                payload.clone(),
                "core.eff-change.unknown-multiple-type-relation",
                "SSG lacks a Java class relation that may allow this multiple SET value",
                changer.span.clone(),
            ))
        }
        Ok((compatible, _))
            if changer.summary.as_ref().is_some_and(|summary| {
                summary.multiplicity == Some(DynamicMultiplicity::Multiple)
            }) && compatible.iter().all(|accepted| !accepted.multiple) =>
        {
            let expected = compatible
                .iter()
                .map(|accepted| accepted.class_name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let message = format!(
                "this Expression accepts only one {expected}, but the value Expression is always multiple"
            );
            Some(reject_with(
                &message,
                "core.eff-change.multiple-to-single-expression",
                changer.span.clone(),
            ))
        }
        Ok((compatible, _))
            if changer
                .summary
                .as_ref()
                .is_some_and(|summary| summary.multiplicity.is_none())
                && compatible.iter().all(|accepted| !accepted.multiple) =>
        {
            Some(continue_with_warning(
                payload.clone(),
                "core.eff-change.unresolved-input-multiplicity",
                "the value Expression's multiplicity is unresolved, so SET cardinality is unknown",
                changer.span.clone(),
            ))
        }
        Ok(_) => None,
        Err(reason) => Some(continue_with_warning(
            payload.clone(),
            "core.eff-change.type-relation-unavailable",
            &format!("could not verify SET type compatibility: {reason}"),
            changer.span.clone(),
        )),
    }
}

fn is_set_pattern(pattern: &str) -> bool {
    pattern.trim_start().starts_with("set ")
}

fn reject_with(
    message: &str,
    code: &str,
    span: crate::nlaocs::skript_parser_addon::types::MappedSpan,
) -> HookOutput {
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
    span: crate::nlaocs::skript_parser_addon::types::MappedSpan,
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
    use super::is_set_pattern;

    #[test]
    fn recognizes_set_by_registered_pattern_instead_of_pattern_index() {
        assert!(is_set_pattern("set %~objects% to %objects%"));
        assert!(!is_set_pattern("increase %~objects% by %objects%"));
    }
}

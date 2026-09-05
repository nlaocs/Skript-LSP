use crate::catalog::TypeRelation;
use crate::nlaocs::skript_parser_addon::types::{
    Diagnostic, DynamicMultiplicity, EffectPayload, ParseContext, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".effects.EffReturn";
const HANDLER_ID: &str = "core.effect.eff-return";
const RETURN_HANDLER_AVAILABLE: &str = "core.return-handler.available";
const RETURN_HANDLER_TYPE: &str = "core.return-handler.return-type";
const RETURN_HANDLER_SINGLE: &str = "core.return-handler.single";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX);
}

pub(super) fn resolve(
    mut payload: EffectPayload,
) -> Option<crate::nlaocs::skript_parser_addon::types::HookOutput> {
    if !super::matches(&payload, HANDLER_ID) {
        return None;
    }
    let candidate_span = payload.candidate.as_ref()?.span.clone();
    super::annotate(&mut payload, "semantic-mode", "return-value");
    let mut diagnostics = Vec::<Diagnostic>::new();

    match return_handler_available(&payload.context) {
        Some(false) => {
            return Some(super::reject_with(
                "the return statement cannot be used here",
                "core.eff-return.missing-handler",
                candidate_span,
            ));
        }
        Some(true) => super::annotate(&mut payload, "return-handler-state", "available"),
        None => unresolved(
            &mut payload,
            &mut diagnostics,
            "core.eff-return.unresolved-handler",
            "the parse context does not identify the active ReturnHandler",
            candidate_span.clone(),
        ),
    }

    match super::context_bool(&payload.context, super::DELAY_STATE_KEY) {
        Some(true) => {
            return Some(super::reject_with(
                "a return statement after a delay cannot return a value to its caller",
                "core.eff-return.after-delay",
                candidate_span,
            ));
        }
        Some(false) => {}
        None => {
            return Some(super::reject_with(
                "the return statement cannot be validated because the delay state is unknown",
                "core.eff-return.unresolved-delay-state",
                candidate_span,
            ));
        }
    }

    let return_type =
        super::context_value(&payload.context, RETURN_HANDLER_TYPE).map(str::to_owned);
    // An explicit void marker is a known ReturnHandler contract. A missing key means the host
    // did not publish the contract and must remain unresolved.
    if is_known_void_return_type(return_type.as_deref()) {
        return Some(super::reject_with(
            "the active ReturnHandler does not return a value",
            "core.eff-return.void-handler",
            candidate_span,
        ));
    }
    let Some(value) = super::parsed_capture(&payload, 0) else {
        return Some(unresolved_output(
            payload,
            diagnostics,
            "core.eff-return.missing-value",
            "the return value was not parsed",
            candidate_span,
        ));
    };
    let value_span = value.span.clone();
    let value_summary = value.summary.clone();
    let Some(summary) = value_summary.as_ref() else {
        return Some(unresolved_output(
            payload,
            diagnostics,
            "core.eff-return.unresolved-value",
            "the return value has no semantic summary",
            value_span,
        ));
    };

    if let Some(return_type) = return_type.as_deref() {
        super::annotate(&mut payload, "return-handler-type", return_type);
        match conversion_verdict(summary, return_type) {
            Ok(ConversionVerdict::Compatible) => {}
            Ok(ConversionVerdict::Incompatible) => {
                return Some(super::reject_with(
                    format!("the return value cannot be converted to {return_type}"),
                    "core.eff-return.incompatible-type",
                    value_span,
                ));
            }
            Ok(ConversionVerdict::Unresolved) => unresolved(
                &mut payload,
                &mut diagnostics,
                "core.eff-return.unresolved-type",
                "the return value's possible types are insufficient to validate the declared return type",
                value_span.clone(),
            ),
            Err(reason) => unresolved(
                &mut payload,
                &mut diagnostics,
                "core.eff-return.type-relation-unavailable",
                &format!("return type compatibility could not be checked: {reason}"),
                value_span.clone(),
            ),
        }
    } else {
        unresolved(
            &mut payload,
            &mut diagnostics,
            "core.eff-return.unresolved-return-type",
            "the active ReturnHandler does not expose its return type",
            value_span.clone(),
        );
    }

    let handler_single = super::context_bool(&payload.context, RETURN_HANDLER_SINGLE);
    match multiplicity_verdict(handler_single, summary.multiplicity) {
        MultiplicityVerdict::Compatible => {}
        MultiplicityVerdict::Incompatible => {
            return Some(super::reject_with(
                "the active ReturnHandler accepts one value, but this return can produce multiple values",
                "core.eff-return.multiple-to-single",
                value_span,
            ));
        }
        MultiplicityVerdict::Unresolved => unresolved(
            &mut payload,
            &mut diagnostics,
            "core.eff-return.unresolved-multiplicity",
            "the ReturnHandler or return value multiplicity is unresolved",
            value_span,
        ),
    }
    Some(super::continue_with_diagnostics(payload, diagnostics))
}

fn return_handler_available(context: &ParseContext) -> Option<bool> {
    // ReturnHandler is an explicit parser contract. Function structure/event metadata alone is
    // not enough because another structure or addon may use the same event class.
    super::context_bool(context, RETURN_HANDLER_AVAILABLE)
}

fn is_known_void_return_type(return_type: Option<&str>) -> bool {
    return_type.is_some_and(|value| {
        let value = value.trim();
        value.eq_ignore_ascii_case("none") || value.eq_ignore_ascii_case("void")
    })
}

fn conversion_verdict(
    summary: &crate::nlaocs::skript_parser_addon::types::ParseSummary,
    target: &str,
) -> Result<ConversionVerdict, String> {
    let source_types = super::eff_change::source_types(summary);
    if source_types.is_empty() {
        return Ok(ConversionVerdict::Unresolved);
    }
    let mut unknown = false;
    for source in source_types {
        let relation = if source == target {
            TypeRelation::Compatible
        } else {
            crate::catalog::can_convert(source, target)?
        };
        match relation {
            TypeRelation::Compatible => return Ok(ConversionVerdict::Compatible),
            TypeRelation::Incompatible => {}
            TypeRelation::Unknown => unknown = true,
        }
    }
    Ok(if unknown {
        ConversionVerdict::Unresolved
    } else {
        ConversionVerdict::Incompatible
    })
}

fn unresolved(
    payload: &mut EffectPayload,
    diagnostics: &mut Vec<Diagnostic>,
    code: &str,
    message: &str,
    span: crate::nlaocs::skript_parser_addon::types::MappedSpan,
) {
    super::mark_unresolved(payload, code);
    diagnostics.push(super::warning(code, message, span));
}

fn unresolved_output(
    mut payload: EffectPayload,
    mut diagnostics: Vec<Diagnostic>,
    code: &str,
    message: &str,
    span: crate::nlaocs::skript_parser_addon::types::MappedSpan,
) -> crate::nlaocs::skript_parser_addon::types::HookOutput {
    unresolved(&mut payload, &mut diagnostics, code, message, span);
    super::continue_with_diagnostics(payload, diagnostics)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConversionVerdict {
    Compatible,
    Incompatible,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MultiplicityVerdict {
    Compatible,
    Incompatible,
    Unresolved,
}

fn multiplicity_verdict(
    handler_single: Option<bool>,
    value: Option<DynamicMultiplicity>,
) -> MultiplicityVerdict {
    match (handler_single, value) {
        (Some(true), Some(DynamicMultiplicity::Multiple)) => MultiplicityVerdict::Incompatible,
        (Some(true), Some(DynamicMultiplicity::Single)) | (Some(false), _) => {
            MultiplicityVerdict::Compatible
        }
        (Some(true), Some(DynamicMultiplicity::Both) | None)
        | (None, Some(DynamicMultiplicity::Multiple | DynamicMultiplicity::Both) | None) => {
            MultiplicityVerdict::Unresolved
        }
        (None, Some(DynamicMultiplicity::Single)) => MultiplicityVerdict::Compatible,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MultiplicityVerdict, is_known_void_return_type, multiplicity_verdict,
        return_handler_available,
    };
    use crate::nlaocs::skript_parser_addon::types::{
        DynamicMultiplicity, ParseContext, ParseContextValue,
    };

    #[test]
    fn rejects_only_a_known_multiple_value_for_a_single_handler() {
        assert_eq!(
            multiplicity_verdict(Some(true), Some(DynamicMultiplicity::Multiple)),
            MultiplicityVerdict::Incompatible
        );
        assert_eq!(
            multiplicity_verdict(Some(true), Some(DynamicMultiplicity::Single)),
            MultiplicityVerdict::Compatible
        );
        assert_eq!(
            multiplicity_verdict(Some(true), Some(DynamicMultiplicity::Both)),
            MultiplicityVerdict::Unresolved
        );
    }

    #[test]
    fn distinguishes_explicit_void_from_missing_return_metadata() {
        assert!(is_known_void_return_type(Some("void")));
        assert!(is_known_void_return_type(Some(" NONE ")));
        assert!(!is_known_void_return_type(Some("java.lang.Object")));
        assert!(!is_known_void_return_type(None));
    }

    #[test]
    fn return_handler_availability_requires_an_explicit_contract() {
        let inferred_function_context = ParseContext {
            syntax_context: 0,
            event_classes: vec!["ch.njol.skript.lang.function.FunctionEvent".to_owned()],
            section_stack: Vec::new(),
            values: Vec::new(),
        };
        assert_eq!(return_handler_available(&inferred_function_context), None);

        let available = ParseContext {
            syntax_context: 0,
            event_classes: Vec::new(),
            section_stack: Vec::new(),
            values: vec![ParseContextValue {
                key: "core.return-handler.available".to_owned(),
                value: "true".to_owned(),
            }],
        };
        assert_eq!(return_handler_available(&available), Some(true));

        let unavailable = ParseContext {
            syntax_context: 0,
            event_classes: Vec::new(),
            section_stack: Vec::new(),
            values: vec![ParseContextValue {
                key: "core.return-handler.available".to_owned(),
                value: "false".to_owned(),
            }],
        };
        assert_eq!(return_handler_available(&unavailable), Some(false));
    }
}

use crate::empty_effects;
use crate::nlaocs::skript_parser_addon::types::{
    Diagnostic, DiagnosticSeverity, DynamicMultiplicity, EffectPayload, HookDecision, HookOutput,
    HookPayload, RegisteredSyntaxHandler, RegisteredSyntaxHandlerTarget, Rejection, SyntaxKind,
};

const CLASS_SUFFIX: &str = ".EffChange";
const HANDLER_ID: &str = "core.effect.eff-change";
const SET_PATTERN: u64 = 3;

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
    if candidate.pattern_index != SET_PATTERN {
        return Some(continue_with(payload));
    }

    let changed = candidate
        .parsed_captures
        .iter()
        .find(|capture| capture.capture_index == 0)
        .and_then(|capture| capture.summary.as_ref());
    let changer = candidate
        .parsed_captures
        .iter()
        .find(|capture| capture.capture_index == 1);
    let sets_multiple_into_single_variable = changed.is_some_and(|summary| {
        summary.kind == "variable" && summary.multiplicity == Some(DynamicMultiplicity::Single)
    }) && changer
        .and_then(|capture| capture.summary.as_ref())
        .is_some_and(|summary| summary.multiplicity == Some(DynamicMultiplicity::Multiple));

    if !sets_multiple_into_single_variable {
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

fn continue_with(payload: EffectPayload) -> HookOutput {
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Effect(payload)),
        effects: empty_effects(),
    }
}

use crate::nlaocs::skript_parser_addon::types::{
    CaptureParserBinding, EffectPayload, HookDecision, HookOutput, HookPayload, ParseResultStatus,
    RegisteredSyntaxHandler, SyntaxKind,
};
use crate::{empty_effects, reject};

const CLASS_SUFFIX: &str = ".EffDoIf";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    handlers.push(RegisteredSyntaxHandler {
        kind: SyntaxKind::Effect,
        class_suffix: CLASS_SUFFIX.to_owned(),
        capture_parsers: vec![binding(0, "host.effect"), binding(1, "host.condition")],
        context_requirements: Vec::new(),
    });
}

pub(super) fn resolve(payload: EffectPayload) -> Option<HookOutput> {
    let candidate = payload.candidate.as_ref()?;
    if candidate
        .element_class
        .as_deref()
        .is_none_or(|class| !class.ends_with(CLASS_SUFFIX))
    {
        return None;
    }
    let valid_condition = candidate.parsed_captures.iter().any(|capture| {
        capture.capture_index == 1
            && capture.parser_id == "host.condition"
            && capture.status == ParseResultStatus::Success
    });
    let nested_effect = candidate.parsed_captures.iter().find(|capture| {
        capture.capture_index == 0
            && capture.parser_id == "host.effect"
            && capture.status == ParseResultStatus::Success
    });
    if !valid_condition || nested_effect.is_none() {
        return Some(reject(
            "conditional Effect requires an Effect and a Condition",
        ));
    }
    if nested_effect
        .and_then(|capture| capture.summary.as_ref())
        .and_then(|summary| summary.element_class.as_deref())
        .is_some_and(|class| class.ends_with(CLASS_SUFFIX))
    {
        return Some(reject("conditional Effects may not be nested"));
    }
    Some(HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Effect(payload)),
        effects: empty_effects(),
    })
}

fn binding(capture_index: u64, parser_id: &str) -> CaptureParserBinding {
    CaptureParserBinding {
        capture_index,
        parser_id: parser_id.to_owned(),
        required: true,
        options: Vec::new(),
    }
}

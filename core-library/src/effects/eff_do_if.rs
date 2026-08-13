use crate::nlaocs::skript_parser_addon::types::{
    EffectPayload, HookDecision, HookOutput, HookPayload, RegisteredCaptureKind,
    RegisteredSyntaxHandler, SyntaxKind,
};
use crate::{empty_effects, reject};

const CLASS_SUFFIX: &str = ".EffDoIf";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    handlers.push(RegisteredSyntaxHandler {
        kind: SyntaxKind::Effect,
        class_suffix: CLASS_SUFFIX.to_owned(),
        regex_captures: vec![
            RegisteredCaptureKind::Effect,
            RegisteredCaptureKind::Condition,
        ],
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
    let valid_condition = candidate
        .conditions
        .iter()
        .any(|capture| capture.capture_index == 1);
    let nested_effect = candidate
        .effects
        .iter()
        .find(|capture| capture.capture_index == 0);
    if !valid_condition || nested_effect.is_none() {
        return Some(reject(
            "conditional Effect requires an Effect and a Condition",
        ));
    }
    if nested_effect
        .and_then(|capture| capture.element_class.as_deref())
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

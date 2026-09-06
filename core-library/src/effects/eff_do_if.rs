use crate::nlaocs::skript_parser_addon::types::{
    CaptureParserBinding, EffectPayload, HookDecision, HookOutput, HookPayload, ParseResultStatus,
    RegisteredSyntaxHandler, RegisteredSyntaxHandlerTarget, SyntaxKind,
};
use crate::{empty_effects, reject};

const CLASS_SUFFIX: &str = ".EffDoIf";
const HANDLER_ID: &str = "core.effect.eff-do-if";
const SKRIPT_DEFINITION_PREFIX: &str = "effect:skript:";

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
        capture_parsers: vec![binding(0, "host.effect"), binding(1, "host.condition")],
        context_requirements: Vec::new(),
    });
}

pub(super) fn resolve(payload: EffectPayload) -> Option<HookOutput> {
    let candidate = payload.candidate.as_ref()?;
    if !crate::runtime::handler_matches(HANDLER_ID, &candidate.registration_id)
        || !is_skript_definition(&candidate.definition_id)
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
    let Some(nested_summary) = nested_effect.and_then(|capture| capture.summary.as_ref()) else {
        return Some(reject(
            "conditional Effect's nested Effect has no syntax identity",
        ));
    };
    // EffDoIf.init() uses instanceof, so a class suffix alone is insufficient here: an addon can
    // legally define another class with the same suffix. The resolved registration identity plus
    // the Skript definition namespace is the generic-WASM equivalent of that Java identity.
    let nested_do_if = nested_summary
        .registration_id
        .as_deref()
        .zip(nested_summary.definition_id.as_deref())
        .is_some_and(|(registration_id, definition_id)| {
            crate::runtime::handler_matches(HANDLER_ID, registration_id)
                && is_skript_definition(definition_id)
        });
    if nested_do_if {
        return Some(reject("conditional Effects may not be nested"));
    }
    Some(HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Effect(payload)),
        effects: empty_effects(),
    })
}

fn is_skript_definition(definition_id: &str) -> bool {
    definition_id.starts_with(SKRIPT_DEFINITION_PREFIX)
}

fn binding(capture_index: u64, parser_id: &str) -> CaptureParserBinding {
    CaptureParserBinding {
        capture_index,
        parser_id: parser_id.to_owned(),
        required: true,
        options: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::is_skript_definition;

    #[test]
    fn addon_definition_with_the_same_class_suffix_is_not_owned_by_core() {
        assert!(is_skript_definition("effect:skript:definition"));
        assert!(!is_skript_definition("effect:skript-reflect:definition"));
        assert!(!is_skript_definition("effect:dummy-addon:definition"));
    }
}

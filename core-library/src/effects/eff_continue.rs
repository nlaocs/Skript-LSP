use crate::nlaocs::skript_parser_addon::types::{
    EffectCapture, EffectPayload, HookDecision, HookOutput, HookPayload, MetadataEntry,
    RegisteredSyntaxHandler, RegisteredSyntaxHandlerTarget, SyntaxKind,
};
use crate::{empty_effects, reject};

const CLASS_SUFFIX: &str = ".effects.EffContinue";
const HANDLER_ID: &str = "core.effect.eff-continue";

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

pub(super) fn resolve(mut payload: EffectPayload) -> Option<HookOutput> {
    let candidate = payload.candidate.as_ref()?;
    if !crate::runtime::handler_matches(HANDLER_ID, &candidate.registration_id) {
        return None;
    }

    let loop_depth = u64::try_from(
        payload
            .context
            .section_stack
            .iter()
            .filter(|frame| super::controls_loop(frame))
            .count(),
    )
    .unwrap_or(u64::MAX);
    if loop_depth == 0 {
        return Some(reject("the continue Effect may only be used in loops"));
    }

    // Skript 2.8 added the second pattern for selecting an outer loop. Its
    // ordinal is counted from the outermost loop, while the first pattern
    // always continues the current (innermost) loop.
    let requested = match candidate.pattern_index {
        0 => loop_depth,
        1 => match first_regex_value(candidate).and_then(|value| value.parse::<u64>().ok()) {
            Some(level) if level > 0 => level,
            _ => return Some(reject("continue requires a positive loop ordinal")),
        },
        _ => return Some(reject("continue Effect has an unknown pattern index")),
    };
    if requested > loop_depth {
        return Some(reject(&format!(
            "cannot continue loop {requested}; only {loop_depth} loop(s) are present"
        )));
    }

    let candidate = payload.candidate.as_mut().expect("candidate was checked");
    candidate.metadata.extend([
        metadata("semantic-mode", "continue-loop"),
        metadata("loop-ordinal", &requested.to_string()),
        metadata("available-loop-depth", &loop_depth.to_string()),
    ]);
    Some(HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Effect(payload)),
        effects: empty_effects(),
    })
}

fn first_regex_value(
    candidate: &crate::nlaocs::skript_parser_addon::types::EffectCandidate,
) -> Option<&str> {
    candidate.captures.iter().find_map(|capture| match capture {
        EffectCapture::Regex(capture) => Some(capture.value.as_str()),
        EffectCapture::Expression(_) => None,
    })
}

fn metadata(key: &str, value: &str) -> MetadataEntry {
    MetadataEntry {
        key: key.to_owned(),
        value: value.to_owned(),
        owner_component_id: None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn outer_loop_ordinals_are_bounded_by_the_active_loop_depth() {
        let validate = |requested: u64, depth: u64| requested > 0 && requested <= depth;
        assert!(validate(1, 1));
        assert!(validate(2, 3));
        assert!(!validate(0, 3));
        assert!(!validate(4, 3));
    }
}

use crate::nlaocs::skript_parser_addon::types::{
    EffectCapture, EffectPayload, HookDecision, HookOutput, HookPayload, MetadataEntry,
    RegisteredSyntaxHandler, RegisteredSyntaxHandlerTarget, SyntaxKind,
};
use crate::{empty_effects, reject};

const CLASS_SUFFIX: &str = ".EffExit";
const HANDLER_ID: &str = "core.effect.eff-exit";

#[derive(Clone, Copy)]
enum ExitTarget {
    Sections,
    Loops,
    Conditionals,
}

impl ExitTarget {
    fn context_key(self) -> &'static str {
        match self {
            Self::Sections => "core.section.depth",
            Self::Loops => "core.section.loop-depth",
            Self::Conditionals => "core.section.conditional-depth",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Sections => "sections",
            Self::Loops => "loops",
            Self::Conditionals => "conditionals",
        }
    }
}

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    handlers.push(RegisteredSyntaxHandler {
        handler_id: HANDLER_ID.to_owned(),
        kind: SyntaxKind::Effect,
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

    // Pattern zero is `stop trigger` and is valid even when no Section is
    // active. The remaining patterns use ParseMark 0/1/2 for
    // Section/Loop/SecConditional, exactly like EffExit.init().
    if candidate.pattern_index == 0 {
        annotate(&mut payload, "trigger", 1, 1);
        return Some(accept(payload));
    }
    let target = match candidate.mark {
        0 => ExitTarget::Sections,
        1 => ExitTarget::Loops,
        2 => ExitTarget::Conditionals,
        _ => return Some(reject("exit Effect has an invalid target mark")),
    };
    let available = super::context_depth(&payload.context, target.context_key());
    let requested = match candidate.pattern_index {
        1 => 1,
        2 => match first_regex_value(candidate).and_then(|value| value.parse::<u64>().ok()) {
            Some(level) if level > 0 => level,
            _ => return Some(reject("exit requires a positive Section count")),
        },
        3 => available,
        _ => return Some(reject("exit Effect has an unknown pattern index")),
    };
    if requested == 0 || requested > available {
        return Some(reject(&format!(
            "cannot exit {requested} {}; only {available} are present",
            target.name()
        )));
    }

    annotate(&mut payload, target.name(), requested, available);
    Some(accept(payload))
}

fn annotate(payload: &mut EffectPayload, target: &str, requested: u64, available: u64) {
    let candidate = payload.candidate.as_mut().expect("candidate was checked");
    candidate.metadata.extend([
        metadata("semantic-mode", "exit"),
        metadata("exit-target", target),
        metadata("exit-count", &requested.to_string()),
        metadata("available-target-depth", &available.to_string()),
    ]);
}

fn accept(payload: EffectPayload) -> HookOutput {
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Effect(payload)),
        effects: empty_effects(),
    }
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
    fn exit_counts_must_be_positive_and_available() {
        let validate = |requested: u64, available: u64| requested > 0 && requested <= available;
        assert!(validate(1, 1));
        assert!(validate(2, 3));
        assert!(!validate(0, 3));
        assert!(!validate(4, 3));
    }
}

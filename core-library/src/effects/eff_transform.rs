use crate::nlaocs::skript_parser_addon::types::{
    CaptureParserBinding, DynamicMultiplicity, EffectPayload, HookDecision, HookOutput,
    HookPayload, MetadataEntry, ParseResultStatus, RegisteredSyntaxHandler,
    RegisteredSyntaxHandlerTarget, SyntaxKind,
};
use crate::{empty_effects, reject};

const CLASS_SUFFIX: &str = ".EffTransform";
const HANDLER_ID: &str = "core.effect.eff-transform";

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
        capture_parsers: vec![input_expression_binding(true, "all")],
        context_requirements: Vec::new(),
    });
}

pub(super) fn resolve(mut payload: EffectPayload) -> Option<HookOutput> {
    let candidate = payload.candidate.as_ref()?;
    if !crate::runtime::handler_matches(HANDLER_ID, &candidate.registration_id) {
        return None;
    }
    let Some(target) = expression_capture(candidate, 0) else {
        return Some(reject("transform requires a list variable"));
    };
    let Some(summary) = target.summary.as_ref() else {
        return Some(reject("transform target has no Expression summary"));
    };
    if summary.kind != "variable" || summary.multiplicity != Some(DynamicMultiplicity::Multiple) {
        return Some(reject("you can only transform list variables"));
    }
    if !parsed_input_expression(candidate, true) {
        return Some(reject("transform mapping is not a valid Expression"));
    }

    payload
        .candidate
        .as_mut()
        .expect("candidate was checked")
        .metadata
        .push(metadata("semantic-mode", "transform-list"));
    Some(accept(payload))
}

pub(super) fn input_expression_binding(required: bool, mode: &str) -> CaptureParserBinding {
    CaptureParserBinding {
        // Capture 0 is the target `%~objects%`; the mapping `<.+>` is capture 1.
        capture_index: 1,
        parser_id: "host.expression".to_owned(),
        required,
        options: vec![
            metadata("parse.mode", mode),
            metadata("context.value.core.input-source.available", "true"),
            metadata("context.value.core.input-source.has-indices", "true"),
            metadata(
                "context.value.core.input-source.value-types",
                "java.lang.Object",
            ),
            metadata(
                "context.value-from-child.core.input-source.value-types",
                "0.possible-return-types",
            ),
        ],
    }
}

pub(super) fn expression_capture(
    candidate: &crate::nlaocs::skript_parser_addon::types::EffectCandidate,
    index: u64,
) -> Option<&crate::nlaocs::skript_parser_addon::types::ParsedCapture> {
    let span = candidate
        .captures
        .iter()
        .filter_map(|capture| match capture {
            crate::nlaocs::skript_parser_addon::types::EffectCapture::Expression(capture) => {
                Some(&capture.span.virtual_range)
            }
            crate::nlaocs::skript_parser_addon::types::EffectCapture::Regex(_) => None,
        })
        .nth(index as usize)?;
    candidate.parsed_captures.iter().find(|capture| {
        capture.parser_id == "host.expression"
            && capture.status == ParseResultStatus::Success
            && capture.span.virtual_range.start == span.start
            && capture.span.virtual_range.end == span.end
    })
}

pub(super) fn mapping_capture(
    candidate: &crate::nlaocs::skript_parser_addon::types::EffectCandidate,
) -> Option<&crate::nlaocs::skript_parser_addon::types::ParsedCapture> {
    let span = candidate
        .captures
        .iter()
        .find_map(|capture| match capture {
            crate::nlaocs::skript_parser_addon::types::EffectCapture::Regex(capture) => {
                Some(&capture.span.virtual_range)
            }
            crate::nlaocs::skript_parser_addon::types::EffectCapture::Expression(_) => None,
        })?;
    candidate.parsed_captures.iter().find(|capture| {
        capture.parser_id == "host.expression"
            && capture.status == ParseResultStatus::Success
            && capture.span.virtual_range.start == span.start
            && capture.span.virtual_range.end == span.end
    })
}

pub(super) fn parsed_input_expression(
    candidate: &crate::nlaocs::skript_parser_addon::types::EffectCandidate,
    required: bool,
) -> bool {
    let has_regex = candidate.captures.iter().any(|capture| {
        matches!(
            capture,
            crate::nlaocs::skript_parser_addon::types::EffectCapture::Regex(_)
        )
    });
    (!required && !has_regex) || mapping_capture(candidate).is_some()
}

pub(super) fn metadata(key: &str, value: &str) -> MetadataEntry {
    MetadataEntry {
        key: key.to_owned(),
        value: value.to_owned(),
        owner_component_id: None,
    }
}

pub(super) fn accept(payload: EffectPayload) -> HookOutput {
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Effect(payload)),
        effects: empty_effects(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn transform_routes_its_regex_mapping_as_an_expression() {
        let binding = super::input_expression_binding(true, "all");
        assert!(binding.required);
        assert_eq!(binding.capture_index, 1);
        assert!(binding.options.iter().any(|entry| {
            entry.key == "context.value-from-child.core.input-source.value-types"
                && entry.value == "0.possible-return-types"
        }));
    }
}

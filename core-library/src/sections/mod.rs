mod sec_conditional;
mod sec_while;

use crate::nlaocs::skript_parser_addon::types::{
    AddonError, AddonErrorKind, HookInvocation, HookOutput, HookPayload, HookPhase,
    RegisteredSyntaxHandler,
};
use crate::{addon_error, continue_without_replacement};

pub(crate) fn handlers() -> Vec<RegisteredSyntaxHandler> {
    let mut handlers = Vec::new();
    sec_conditional::register(&mut handlers);
    sec_while::register(&mut handlers);
    handlers
}

pub(crate) fn parse(input: HookInvocation) -> Result<HookOutput, AddonError> {
    if !matches!(input.phase, HookPhase::Section) {
        return Err(addon_error(
            AddonErrorKind::InvalidPayload,
            "CoreLibrary Section semantics require the Section phase",
        ));
    }
    let HookPayload::Section(payload) = input.payload else {
        return Err(addon_error(
            AddonErrorKind::InvalidPayload,
            "CoreLibrary Section semantics require a Section payload",
        ));
    };
    let output = if sec_conditional::matches(&payload) {
        sec_conditional::resolve(input.context, payload)
    } else if sec_while::matches(&payload) {
        sec_while::resolve(input.context, payload)
    } else {
        continue_without_replacement()
    };
    Ok(output)
}

fn register_handler(handlers: &mut Vec<RegisteredSyntaxHandler>, class_suffix: &str) {
    use crate::nlaocs::skript_parser_addon::types::{RegisteredCaptureKind, SyntaxKind};

    handlers.push(RegisteredSyntaxHandler {
        kind: SyntaxKind::Section,
        class_suffix: class_suffix.to_owned(),
        regex_captures: vec![RegisteredCaptureKind::Condition],
    });
}

fn resolve_condition_section(
    context: crate::nlaocs::skript_parser_addon::types::InvocationContext,
    payload: crate::nlaocs::skript_parser_addon::types::SectionPayload,
) -> HookOutput {
    use crate::nlaocs::skript_parser_addon::types::{
        ContextUpdate, HookDecision, HookEffects, SectionTiming,
    };
    use crate::reject;

    if matches!(payload.timing, SectionTiming::EnterChildren)
        && !payload.candidate.regex_captures.is_empty()
        && payload.candidate.conditions.len() != payload.candidate.regex_captures.len()
    {
        return reject("Section requires every condition capture to parse");
    }
    let context_updates = if matches!(payload.timing, SectionTiming::EnterChildren) {
        let mut updates = vec![ContextUpdate {
            syntax_context: context.syntax_context,
            key: "core.section.class".to_owned(),
            value: payload
                .candidate
                .element_class
                .as_ref()
                .map(|class| class.as_bytes().to_vec()),
        }];
        if payload.candidate.loop_section {
            updates.push(ContextUpdate {
                syntax_context: context.syntax_context,
                key: "core.section.loop".to_owned(),
                value: Some(b"true".to_vec()),
            });
        }
        updates
    } else {
        Vec::new()
    };
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Section(payload)),
        effects: HookEffects {
            diagnostics: Vec::new(),
            context_updates,
            parse_requests: Vec::new(),
        },
    }
}

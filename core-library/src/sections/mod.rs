mod eff_sec_shoot;
mod eff_sec_spawn;
mod sec_catch_errors;
mod sec_conditional;
mod sec_filter;
mod sec_loop;
mod sec_while;

use crate::addon_error;
use crate::nlaocs::skript_parser_addon::types::{
    AddonError, AddonErrorKind, CaptureParserBinding, ContextUpdate, HookDecision, HookEffects,
    HookInvocation, HookOutput, HookPayload, HookPhase, InvocationContext, MetadataEntry,
    ParseRequest, RegisteredSyntaxHandler, RegisteredSyntaxHandlerTarget, SectionPayload,
    SectionTiming, SyntaxKind,
};

pub(crate) fn handlers() -> Vec<RegisteredSyntaxHandler> {
    let mut handlers = Vec::new();
    sec_conditional::register(&mut handlers);
    sec_filter::register(&mut handlers);
    sec_loop::register(&mut handlers);
    sec_while::register(&mut handlers);
    sec_catch_errors::register(&mut handlers);
    eff_sec_shoot::register(&mut handlers);
    eff_sec_spawn::register(&mut handlers);
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
        sec_conditional::resolve(input.context, payload, &input.parse_results)
    } else if sec_filter::matches(&payload) {
        sec_filter::resolve(input.context, payload)
    } else if sec_loop::matches(&payload) {
        sec_loop::resolve(input.context, payload)
    } else if sec_while::matches(&payload) {
        sec_while::resolve(input.context, payload)
    } else if sec_catch_errors::matches(&payload) {
        sec_catch_errors::resolve(input.context, payload)
    } else if eff_sec_shoot::matches(&payload) {
        eff_sec_shoot::resolve(input.context, payload)
    } else if eff_sec_spawn::matches(&payload) {
        eff_sec_spawn::resolve(input.context, payload)
    } else {
        continue_with_section_context(&input.context, payload, [], Vec::new())
    };
    Ok(output)
}

pub(super) fn request_parses(
    payload: SectionPayload,
    parse_requests: Vec<ParseRequest>,
) -> HookOutput {
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Section(payload)),
        effects: HookEffects {
            diagnostics: Vec::new(),
            context_updates: Vec::new(),
            parse_requests,
            parse_results: Vec::new(),
        },
    }
}

fn register_handler(
    handlers: &mut Vec<RegisteredSyntaxHandler>,
    handler_id: &str,
    class_suffix: &str,
    capture_parsers: Vec<CaptureParserBinding>,
) {
    register_handler_targets(handlers, handler_id, &[class_suffix], capture_parsers);
}

pub(super) fn register_handler_targets(
    handlers: &mut Vec<RegisteredSyntaxHandler>,
    handler_id: &str,
    class_suffixes: &[&str],
    capture_parsers: Vec<CaptureParserBinding>,
) {
    register_pattern_handler_targets(
        handlers,
        handler_id,
        class_suffixes,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        capture_parsers,
    );
}

#[allow(clippy::too_many_arguments)]
fn register_pattern_handler(
    handlers: &mut Vec<RegisteredSyntaxHandler>,
    handler_id: &str,
    class_suffix: &str,
    pattern_sources: Vec<String>,
    required_tags: Vec<String>,
    forbidden_tags: Vec<String>,
    marks: Vec<i32>,
    capture_parsers: Vec<CaptureParserBinding>,
) {
    register_pattern_handler_targets(
        handlers,
        handler_id,
        &[class_suffix],
        pattern_sources,
        required_tags,
        forbidden_tags,
        marks,
        capture_parsers,
    );
}

#[allow(clippy::too_many_arguments)]
fn register_pattern_handler_targets(
    handlers: &mut Vec<RegisteredSyntaxHandler>,
    handler_id: &str,
    class_suffixes: &[&str],
    pattern_sources: Vec<String>,
    required_tags: Vec<String>,
    forbidden_tags: Vec<String>,
    marks: Vec<i32>,
    capture_parsers: Vec<CaptureParserBinding>,
) {
    handlers.push(RegisteredSyntaxHandler {
        handler_id: handler_id.to_owned(),
        kind: SyntaxKind::Section,
        targets: class_suffixes
            .iter()
            .map(|suffix| RegisteredSyntaxHandlerTarget::ClassSuffix((*suffix).to_owned()))
            .collect(),
        pattern_indices: Vec::new(),
        pattern_sources,
        required_tags,
        forbidden_tags,
        marks,
        capture_parsers,
        context_requirements: Vec::new(),
    });
}

pub(super) fn parsed_capture<'a>(
    payload: &'a SectionPayload,
    capture_index: u64,
    parser_id: &str,
) -> Option<&'a crate::nlaocs::skript_parser_addon::types::ParsedCapture> {
    payload
        .candidate
        .parsed_captures
        .iter()
        .find(|capture| capture.capture_index == capture_index && capture.parser_id == parser_id)
}

pub(super) fn context_value_update(
    context: &InvocationContext,
    key: &str,
    value: &str,
) -> ContextUpdate {
    ContextUpdate {
        syntax_context: context.syntax_context,
        key: key.to_owned(),
        value: Some(value.as_bytes().to_vec()),
    }
}

pub(super) fn warning(
    code: impl Into<String>,
    message: impl Into<String>,
    span: crate::nlaocs::skript_parser_addon::types::MappedSpan,
) -> crate::nlaocs::skript_parser_addon::types::Diagnostic {
    crate::nlaocs::skript_parser_addon::types::Diagnostic {
        code: code.into(),
        message: message.into(),
        severity: crate::nlaocs::skript_parser_addon::types::DiagnosticSeverity::Warning,
        span,
        related: Vec::new(),
    }
}

fn condition_binding() -> CaptureParserBinding {
    CaptureParserBinding {
        capture_index: 0,
        parser_id: CONDITION_PARSER_ID.to_owned(),
        required: true,
        options: Vec::new(),
    }
}

fn parse_condition_binding(event_classes: &str) -> CaptureParserBinding {
    CaptureParserBinding {
        options: vec![
            MetadataEntry {
                key: "context.event-classes".to_owned(),
                value: event_classes.to_owned(),
                owner_component_id: None,
            },
            MetadataEntry {
                key: "context.value.parser.event-name".to_owned(),
                value: "parse".to_owned(),
                owner_component_id: None,
            },
        ],
        ..condition_binding()
    }
}

pub(super) const CONDITION_PARSER_ID: &str = "host.condition";

pub(super) fn condition_captures_are_parsed(
    payload: &SectionPayload,
    expected: usize,
) -> Result<(), String> {
    let actual = payload.candidate.regex_captures.len();
    if actual != expected {
        return Err(format!(
            "Section expected {expected} condition capture(s), but the matched pattern produced {actual}"
        ));
    }
    use crate::nlaocs::skript_parser_addon::types::ParseResultStatus;
    for index in 0..expected {
        let parsed = payload.candidate.parsed_captures.iter().any(|capture| {
            capture.capture_index == index as u64
                && capture.parser_id == CONDITION_PARSER_ID
                && capture.status == ParseResultStatus::Success
        });
        if !parsed {
            return Err(format!(
                "Section requires condition capture {index} to parse successfully"
            ));
        }
    }
    if expected == 0
        && payload
            .candidate
            .parsed_captures
            .iter()
            .any(|capture| capture.parser_id == CONDITION_PARSER_ID)
    {
        return Err(
            "Section without a condition capture received parsed condition data".to_owned(),
        );
    }
    Ok(())
}

pub(super) fn continue_with_section_context(
    context: &InvocationContext,
    mut payload: SectionPayload,
    metadata: impl IntoIterator<Item = (&'static str, String)>,
    extra_updates: Vec<ContextUpdate>,
) -> HookOutput {
    let entering = matches!(payload.timing, SectionTiming::EnterChildren);
    if entering {
        for (key, value) in metadata {
            if let Some(existing) = payload
                .candidate
                .metadata
                .iter_mut()
                .find(|entry| entry.owner_component_id.is_none() && entry.key == key)
            {
                existing.value = value;
            } else {
                payload.candidate.metadata.push(
                    crate::nlaocs::skript_parser_addon::types::MetadataEntry {
                        key: key.to_owned(),
                        value,
                        owner_component_id: None,
                    },
                );
            }
        }
    }

    let context_updates = if entering {
        let mut updates = vec![ContextUpdate {
            syntax_context: context.syntax_context,
            key: "core.section.class".to_owned(),
            value: payload
                .candidate
                .element_class
                .as_ref()
                .map(|class| class.as_bytes().to_vec()),
        }];
        updates.push(increment_context_update(
            &payload.context,
            context.syntax_context,
            "core.section.depth",
        ));
        if payload.candidate.loop_section
            && !extra_updates
                .iter()
                .any(|update| update.key == "core.section.loop-depth")
        {
            updates.push(increment_context_update(
                &payload.context,
                context.syntax_context,
                "core.section.loop-depth",
            ));
        }
        updates.extend(extra_updates);
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
            parse_results: Vec::new(),
        },
    }
}

pub(super) fn increment_context_update(
    context: &crate::nlaocs::skript_parser_addon::types::ParseContext,
    syntax_context: u64,
    key: &str,
) -> ContextUpdate {
    let current = context
        .values
        .iter()
        .rfind(|value| value.key == key)
        .and_then(|value| value.value.parse::<u64>().ok())
        .unwrap_or(0);
    ContextUpdate {
        syntax_context,
        key: key.to_owned(),
        value: Some(current.saturating_add(1).to_string().into_bytes()),
    }
}

pub(super) fn reject_section(reason: impl Into<String>) -> HookOutput {
    HookOutput {
        decision: HookDecision::Reject(crate::nlaocs::skript_parser_addon::types::Rejection {
            reason: reason.into(),
            diagnostics: Vec::new(),
        }),
        replacement: None,
        effects: HookEffects {
            diagnostics: Vec::new(),
            context_updates: Vec::new(),
            parse_requests: Vec::new(),
            parse_results: Vec::new(),
        },
    }
}

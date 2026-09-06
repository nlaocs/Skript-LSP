mod legacy;
mod options_macro;
mod struct_aliases;
mod struct_auto_reload;
mod struct_command;
mod struct_event;
mod struct_example;
mod struct_function;
mod struct_options;
mod struct_using;
mod struct_variables;

use crate::nlaocs::skript_parser_addon::types::{
    AddonError, AddonErrorKind, CaptureParserBinding, Diagnostic, DiagnosticSeverity, HookDecision,
    HookEffects, HookInvocation, HookOutput, HookPayload, HookPhase, InvocationContext, MappedSpan,
    MetadataEntry, ParseContext, ParseRequest, RawNodeKind, RawTreeNode, RegisteredSyntaxHandler,
    RegisteredSyntaxHandlerTarget, SourceOrigin, StructureBodyMode, StructurePayload,
    StructureTiming, SyntaxKind, TextRange,
};
use crate::{addon_error, not_applicable};

pub(crate) fn handlers() -> Vec<RegisteredSyntaxHandler> {
    let mut handlers = Vec::new();
    struct_aliases::register(&mut handlers);
    struct_auto_reload::register(&mut handlers);
    struct_event::register(&mut handlers);
    struct_example::register(&mut handlers);
    struct_function::register(&mut handlers);
    struct_command::register(&mut handlers);
    struct_options::register(&mut handlers);
    struct_using::register(&mut handlers);
    struct_variables::register(&mut handlers);
    handlers
}

pub(crate) fn register_missing(skript_version: &str) -> Result<(), String> {
    legacy::register_missing(skript_version)
}

pub(crate) fn expand_options(
    input: crate::nlaocs::skript_parser_addon::types::TreeMacroInput,
) -> crate::nlaocs::skript_parser_addon::types::TreeMacroOutput {
    options_macro::expand(input)
}

pub(crate) fn parse(input: HookInvocation) -> Result<HookOutput, AddonError> {
    if !matches!(input.phase, HookPhase::Structure) {
        return Err(addon_error(
            AddonErrorKind::InvalidPayload,
            "CoreLibrary Structure semantics require the Structure phase",
        ));
    }
    let HookPayload::Structure(payload) = input.payload else {
        return Err(addon_error(
            AddonErrorKind::InvalidPayload,
            "CoreLibrary Structure semantics require a Structure payload",
        ));
    };
    Ok(if struct_aliases::matches(&payload) {
        struct_aliases::resolve(input.context, payload)
    } else if struct_auto_reload::matches(&payload) {
        struct_auto_reload::resolve(input.context, payload)
    } else if struct_event::matches(&payload) {
        struct_event::resolve(input.context, payload)
    } else if struct_example::matches(&payload) {
        struct_example::resolve(input.context, payload)
    } else if struct_function::matches(&payload) {
        struct_function::resolve(input.context, payload, &input.parse_results)
    } else if struct_command::matches(&payload) {
        struct_command::resolve(input.context, payload, &input.parse_results)
    } else if struct_options::matches(&payload) {
        struct_options::resolve(input.context, payload)
    } else if struct_using::matches(&payload) {
        struct_using::resolve(input.context, payload)
    } else if struct_variables::matches(&payload) {
        struct_variables::resolve(input.context, payload, &input.parse_results)
    } else {
        not_applicable()
    })
}

fn register_handler(
    handlers: &mut Vec<RegisteredSyntaxHandler>,
    handler_id: &str,
    class_suffix: &str,
    capture_parsers: Vec<CaptureParserBinding>,
) {
    handlers.push(RegisteredSyntaxHandler {
        handler_id: handler_id.to_owned(),
        kind: SyntaxKind::Structure,
        phase: crate::nlaocs::skript_parser_addon::types::HookPhase::Structure,
        targets: vec![
            RegisteredSyntaxHandlerTarget::ClassSuffix(class_suffix.to_owned()),
            RegisteredSyntaxHandlerTarget::DynamicHandler(handler_id.to_owned()),
        ],
        pattern_indices: Vec::new(),
        pattern_sources: Vec::new(),
        required_tags: Vec::new(),
        forbidden_tags: Vec::new(),
        marks: Vec::new(),
        capture_parsers,
        context_requirements: Vec::new(),
    });
}

fn continue_with_mode(
    context: &InvocationContext,
    mut payload: StructurePayload,
    mode: StructureBodyMode,
    semantic_mode: &str,
    context_key: &str,
) -> HookOutput {
    let entering = matches!(payload.timing, StructureTiming::EnterBody);
    if entering {
        payload.candidate.body_mode = mode;
        append_metadata(&mut payload, "semantic-mode", semantic_mode);
    }
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Structure(payload)),
        effects: HookEffects {
            diagnostics: Vec::new(),
            context_updates: entering
                .then(
                    || crate::nlaocs::skript_parser_addon::types::ContextUpdate {
                        syntax_context: context.syntax_context,
                        key: context_key.to_owned(),
                        value: Some(b"true".to_vec()),
                    },
                )
                .into_iter()
                .collect(),
            parse_requests: Vec::new(),
            parse_results: Vec::new(),
        },
    }
}

fn continue_unresolved(mut payload: StructurePayload, diagnostics: Vec<Diagnostic>) -> HookOutput {
    append_metadata(&mut payload, "semantic-state", "unresolved");
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Structure(payload)),
        effects: HookEffects {
            diagnostics,
            context_updates: Vec::new(),
            parse_requests: Vec::new(),
            parse_results: Vec::new(),
        },
    }
}

pub(super) fn append_metadata(payload: &mut StructurePayload, key: &str, value: &str) {
    if let Some(existing) = payload
        .candidate
        .metadata
        .iter_mut()
        .find(|entry| entry.owner_component_id.is_none() && entry.key == key)
    {
        existing.value = value.to_owned();
    } else {
        payload.candidate.metadata.push(MetadataEntry {
            key: key.to_owned(),
            value: value.to_owned(),
            owner_component_id: None,
        });
    }
}

pub(super) fn structure_diagnostic(
    code: impl Into<String>,
    message: impl Into<String>,
    severity: DiagnosticSeverity,
    span: MappedSpan,
) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        message: message.into(),
        severity,
        span,
        related: Vec::new(),
    }
}

pub(super) fn structure_warning(
    code: impl Into<String>,
    message: impl Into<String>,
    span: MappedSpan,
) -> Diagnostic {
    structure_diagnostic(code, message, DiagnosticSeverity::Warning, span)
}

pub(super) fn structure_error(
    code: impl Into<String>,
    message: impl Into<String>,
    span: MappedSpan,
) -> Diagnostic {
    structure_diagnostic(code, message, DiagnosticSeverity::Error, span)
}

pub(super) fn context_value_update(
    context: &InvocationContext,
    key: &str,
    value: &str,
) -> crate::nlaocs::skript_parser_addon::types::ContextUpdate {
    crate::nlaocs::skript_parser_addon::types::ContextUpdate {
        syntax_context: context.syntax_context,
        key: key.to_owned(),
        value: Some(value.as_bytes().to_vec()),
    }
}

pub(super) fn direct_body_nodes(payload: &StructurePayload) -> Vec<&RawTreeNode> {
    payload
        .body_tree
        .nodes
        .iter()
        .filter(|node| node.parent == Some(payload.candidate.raw_node_id))
        .collect()
}

pub(super) fn is_trivia(node: &RawTreeNode) -> bool {
    matches!(node.kind, RawNodeKind::Blank | RawNodeKind::Comment)
}

pub(super) fn context_value<'a>(payload: &'a StructurePayload, key: &str) -> Option<&'a str> {
    payload
        .context
        .values
        .iter()
        .rfind(|entry| entry.key == key)
        .map(|entry| entry.value.as_str())
}

pub(super) fn request_parses(
    payload: StructurePayload,
    parse_requests: Vec<ParseRequest>,
) -> HookOutput {
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Structure(payload)),
        effects: HookEffects {
            diagnostics: Vec::new(),
            context_updates: Vec::new(),
            parse_requests,
            parse_results: Vec::new(),
        },
    }
}

pub(super) fn pending_parse_requests(
    requests: &[ParseRequest],
    results: &[crate::nlaocs::skript_parser_addon::types::ParseResult],
) -> Vec<ParseRequest> {
    requests
        .iter()
        .filter(|request| {
            !results.iter().any(|result| {
                result.request_id == request.request_id && result.parser_id == request.parser_id
            })
        })
        .cloned()
        .collect()
}

pub(crate) fn parse_context_options(context: &ParseContext) -> Vec<MetadataEntry> {
    let mut options = Vec::new();
    if !context.event_classes.is_empty() {
        options.push(MetadataEntry {
            key: "context.event-classes".to_owned(),
            value: context.event_classes.join(";"),
            owner_component_id: None,
        });
    }
    options.extend(context.values.iter().map(|entry| MetadataEntry {
        key: format!("context.value.{}", entry.key),
        value: entry.value.clone(),
        owner_component_id: None,
    }));
    options
}

fn parse_context_options_with_event_classes(
    context: &ParseContext,
    event_classes: &[&str],
) -> Vec<MetadataEntry> {
    let mut options = parse_context_options(context);
    options.push(MetadataEntry {
        key: "context.event-classes".to_owned(),
        value: event_classes.join(";"),
        owner_component_id: None,
    });
    options
}

fn mapped_subspan(parent: &MappedSpan, start: u64, end: u64) -> MappedSpan {
    let relative_start = start.saturating_sub(parent.virtual_range.start);
    let relative_end = end.saturating_sub(parent.virtual_range.start);
    let parent_len = parent
        .virtual_range
        .end
        .saturating_sub(parent.virtual_range.start);
    MappedSpan {
        virtual_range: TextRange { start, end },
        origins: parent
            .origins
            .iter()
            .map(|origin| SourceOrigin {
                original_range: if origin
                    .original_range
                    .end
                    .saturating_sub(origin.original_range.start)
                    >= parent_len
                {
                    TextRange {
                        start: origin.original_range.start.saturating_add(relative_start),
                        end: origin.original_range.start.saturating_add(relative_end),
                    }
                } else {
                    origin.original_range
                },
                kind: origin.kind,
                expansion: origin.expansion,
            })
            .collect(),
    }
}

pub(super) fn reject_structure(reason: impl Into<String>) -> HookOutput {
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

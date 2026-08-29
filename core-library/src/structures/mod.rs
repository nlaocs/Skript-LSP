mod struct_command;
mod struct_event;
mod struct_function;

use crate::nlaocs::skript_parser_addon::types::{
    AddonError, AddonErrorKind, CaptureParserBinding, HookDecision, HookEffects, HookInvocation,
    HookOutput, HookPayload, HookPhase, MetadataEntry, RegisteredSyntaxHandler,
    RegisteredSyntaxHandlerTarget, StructureBodyMode, StructurePayload, StructureTiming,
    SyntaxKind,
};
use crate::{addon_error, not_applicable};

pub(crate) fn handlers() -> Vec<RegisteredSyntaxHandler> {
    let mut handlers = Vec::new();
    struct_event::register(&mut handlers);
    struct_function::register(&mut handlers);
    struct_command::register(&mut handlers);
    handlers
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
    Ok(if struct_event::matches(&payload) {
        struct_event::resolve(input.context, payload)
    } else if struct_function::matches(&payload) {
        struct_function::resolve(input.context, payload)
    } else if struct_command::matches(&payload) {
        struct_command::resolve(input.context, payload)
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
        target: RegisteredSyntaxHandlerTarget::ClassSuffix(class_suffix.to_owned()),
        capture_parsers,
        context_requirements: Vec::new(),
    });
}

fn continue_with_mode(
    context: &crate::nlaocs::skript_parser_addon::types::InvocationContext,
    mut payload: StructurePayload,
    mode: StructureBodyMode,
    semantic_mode: &str,
    context_key: &str,
) -> HookOutput {
    let entering = matches!(payload.timing, StructureTiming::EnterBody);
    if entering {
        payload.candidate.body_mode = mode;
        payload.candidate.metadata.push(MetadataEntry {
            key: "semantic-mode".to_owned(),
            value: semantic_mode.to_owned(),
            owner_component_id: None,
        });
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

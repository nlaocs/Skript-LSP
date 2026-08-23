use super::register_handler;
use crate::nlaocs::skript_parser_addon::types::{
    CaptureParserBinding, ContextUpdate, HookDecision, HookEffects, HookOutput, HookPayload,
    InvocationContext, RegisteredSyntaxHandler, StructureBodyMode, StructurePayload,
    StructureTiming,
};

const CLASS_SUFFIX: &str = ".StructEvent";
const HANDLER_ID: &str = "core.structure.struct-event";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(
        handlers,
        HANDLER_ID,
        CLASS_SUFFIX,
        vec![CaptureParserBinding {
            capture_index: 0,
            parser_id: "host.event".to_owned(),
            required: true,
            options: Vec::new(),
        }],
    );
}

pub(super) fn matches(payload: &StructurePayload) -> bool {
    crate::runtime::handler_matches(HANDLER_ID, &payload.candidate.registration_id)
}

pub(super) fn resolve(context: InvocationContext, mut payload: StructurePayload) -> HookOutput {
    if !matches!(payload.timing, StructureTiming::EnterBody) {
        return super::continue_with_mode(
            &context,
            payload,
            StructureBodyMode::Trigger,
            "event-structure",
            "core.structure.event",
        );
    }
    let Some(event) = payload
        .candidate
        .parsed_captures
        .iter()
        .find(|capture| capture.parser_id == "host.event")
    else {
        return crate::reject("StructEvent requires its Event capture to parse");
    };
    let reference_classes = event
        .summary
        .as_ref()
        .and_then(|summary| {
            summary.metadata.iter().find(|entry| {
                entry.owner_component_id.is_none() && entry.key == "parser.event.reference-classes"
            })
        })
        .map(|entry| entry.value.as_bytes().to_vec());
    payload.candidate.body_mode = StructureBodyMode::Trigger;
    payload
        .candidate
        .metadata
        .push(crate::nlaocs::skript_parser_addon::types::MetadataEntry {
            key: "semantic-mode".to_owned(),
            value: "event-structure".to_owned(),
            owner_component_id: None,
        });
    let mut context_updates = vec![ContextUpdate {
        syntax_context: context.syntax_context,
        key: "core.structure.event".to_owned(),
        value: Some(b"true".to_vec()),
    }];
    if let Some(value) = reference_classes {
        context_updates.push(ContextUpdate {
            syntax_context: context.syntax_context,
            key: "parser.event-classes".to_owned(),
            value: Some(value),
        });
    }
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Structure(payload)),
        effects: HookEffects {
            diagnostics: Vec::new(),
            context_updates,
            parse_requests: Vec::new(),
            parse_results: Vec::new(),
        },
    }
}

mod delay;
mod eff_cancel_event;
mod eff_change;
mod eff_command;
mod eff_continue;
mod eff_copy;
mod eff_do_if;
mod eff_enchant;
mod eff_entity_visibility;
mod eff_exit;
mod eff_open_inventory;
mod eff_register_tag;
mod eff_replace;
mod eff_respawn;
mod eff_return;
mod eff_sort;
mod eff_suppress_type_hints;
mod eff_suppress_warnings;
mod eff_toggle;
mod eff_transform;
mod eff_zombify;
mod event_context;
mod platform_guards;
mod potion_property;

use crate::nlaocs::skript_parser_addon::types::{
    AddonError, AddonErrorKind, ContextUpdate, Diagnostic, DiagnosticSeverity, EffectPayload,
    EffectTiming, HookDecision, HookEffects, HookInvocation, HookOutput, HookPayload, HookPhase,
    MappedSpan, MetadataEntry, ParseContext, ParsedCapture, RegisteredSyntaxHandler,
    RegisteredSyntaxHandlerTarget, Rejection, SyntaxKind,
};
use crate::{addon_error, empty_effects, not_applicable};

pub(super) const DELAY_STATE_KEY: &str = "parser.delay-state";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ContractVerdict {
    Accepted,
    Rejected,
    Unresolved,
}

pub(crate) fn handlers() -> Vec<RegisteredSyntaxHandler> {
    let mut handlers = Vec::new();
    delay::register(&mut handlers);
    eff_cancel_event::register(&mut handlers);
    eff_change::register(&mut handlers);
    eff_command::register(&mut handlers);
    eff_continue::register(&mut handlers);
    eff_copy::register(&mut handlers);
    eff_do_if::register(&mut handlers);
    eff_entity_visibility::register(&mut handlers);
    eff_exit::register(&mut handlers);
    eff_enchant::register(&mut handlers);
    eff_open_inventory::register(&mut handlers);
    eff_replace::register(&mut handlers);
    eff_register_tag::register(&mut handlers);
    eff_respawn::register(&mut handlers);
    eff_return::register(&mut handlers);
    eff_sort::register(&mut handlers);
    eff_suppress_type_hints::register(&mut handlers);
    eff_suppress_warnings::register(&mut handlers);
    eff_toggle::register(&mut handlers);
    eff_transform::register(&mut handlers);
    eff_zombify::register(&mut handlers);
    event_context::register(&mut handlers);
    platform_guards::register(&mut handlers);
    potion_property::register(&mut handlers);
    handlers
}

pub(crate) fn parse(input: HookInvocation) -> Result<HookOutput, AddonError> {
    if !matches!(input.phase, HookPhase::Effect) {
        return Err(addon_error(
            AddonErrorKind::InvalidPayload,
            "CoreLibrary Effect semantics require the Effect phase",
        ));
    }
    let HookPayload::Effect(payload) = input.payload else {
        return Err(addon_error(
            AddonErrorKind::InvalidPayload,
            "CoreLibrary Effect semantics require an Effect payload",
        ));
    };
    if matches!(payload.timing, EffectTiming::Before) && payload.candidate.is_none() {
        let syntax_context = payload.context.syntax_context;
        let initialize_delay = context_value(&payload.context, DELAY_STATE_KEY).is_none();
        let mut output = accept(payload);
        if initialize_delay {
            output.effects.context_updates.push(ContextUpdate {
                syntax_context,
                key: DELAY_STATE_KEY.to_owned(),
                value: Some(b"false".to_vec()),
            });
        }
        return Ok(output);
    }
    Ok(resolve_handlers(payload))
}

type EffectResolver = fn(EffectPayload) -> Option<HookOutput>;

const RESOLVERS: &[EffectResolver] = &[
    delay::resolve,
    eff_cancel_event::resolve,
    eff_change::resolve,
    eff_command::resolve,
    eff_continue::resolve,
    eff_copy::resolve,
    eff_do_if::resolve,
    eff_entity_visibility::resolve,
    eff_exit::resolve,
    eff_enchant::resolve,
    eff_open_inventory::resolve,
    eff_replace::resolve,
    eff_register_tag::resolve,
    eff_respawn::resolve,
    eff_return::resolve,
    eff_sort::resolve,
    eff_suppress_type_hints::resolve,
    eff_suppress_warnings::resolve,
    eff_toggle::resolve,
    eff_transform::resolve,
    eff_zombify::resolve,
    event_context::resolve,
    potion_property::resolve,
    platform_guards::resolve,
];

fn resolve_handlers(mut payload: EffectPayload) -> HookOutput {
    let mut effects = empty_effects();
    let mut matched = false;
    for resolver in RESOLVERS {
        let Some(mut output) = resolver(payload.clone()) else {
            continue;
        };
        matched = true;
        merge_effects(
            &mut effects,
            std::mem::replace(&mut output.effects, empty_effects()),
        );
        if matches!(&output.decision, HookDecision::Reject(_)) {
            output.effects = effects;
            return output;
        }
        if let Some(HookPayload::Effect(next)) = output.replacement {
            payload = next;
        }
    }
    if !matched {
        return not_applicable();
    }
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Effect(payload)),
        effects,
    }
}

fn merge_effects(target: &mut HookEffects, mut source: HookEffects) {
    target.diagnostics.append(&mut source.diagnostics);
    target.context_updates.append(&mut source.context_updates);
    target.parse_requests.append(&mut source.parse_requests);
    target.parse_results.append(&mut source.parse_results);
}

fn register_handler(
    handlers: &mut Vec<RegisteredSyntaxHandler>,
    handler_id: &str,
    class_suffix: &str,
) {
    register_handler_targets(handlers, handler_id, &[class_suffix]);
}

fn register_handler_targets(
    handlers: &mut Vec<RegisteredSyntaxHandler>,
    handler_id: &str,
    class_suffixes: &[&str],
) {
    handlers.push(RegisteredSyntaxHandler {
        handler_id: handler_id.to_owned(),
        kind: SyntaxKind::Effect,
        phase: crate::nlaocs::skript_parser_addon::types::HookPhase::Effect,
        targets: class_suffixes
            .iter()
            .map(|suffix| RegisteredSyntaxHandlerTarget::ClassSuffix((*suffix).to_owned()))
            .collect(),
        pattern_indices: Vec::new(),
        pattern_sources: Vec::new(),
        required_tags: Vec::new(),
        forbidden_tags: Vec::new(),
        marks: Vec::new(),
        capture_parsers: Vec::new(),
        context_requirements: Vec::new(),
    });
}

fn matches(payload: &EffectPayload, handler_id: &str) -> bool {
    payload.candidate.as_ref().is_some_and(|candidate| {
        crate::runtime::handler_matches(handler_id, &candidate.registration_id)
    })
}

fn parsed_capture(payload: &EffectPayload, capture_index: u64) -> Option<&ParsedCapture> {
    payload
        .candidate
        .as_ref()?
        .parsed_captures
        .iter()
        .find(|capture| capture.capture_index == capture_index)
}

fn context_value<'a>(context: &'a ParseContext, key: &str) -> Option<&'a str> {
    context
        .values
        .iter()
        .rfind(|entry| entry.key == key)
        .map(|entry| entry.value.as_str())
}

pub(super) fn add_context_update(
    output: &mut HookOutput,
    syntax_context: u64,
    key: &str,
    value: Option<&[u8]>,
) {
    output.effects.context_updates.push(ContextUpdate {
        syntax_context,
        key: key.to_owned(),
        value: value.map(ToOwned::to_owned),
    });
}

fn context_bool(context: &ParseContext, key: &str) -> Option<bool> {
    match context_value(context, key)? {
        value if value.eq_ignore_ascii_case("true") => Some(true),
        value if value.eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

fn metadata_value<'a>(metadata: &'a [MetadataEntry], key: &str) -> Option<&'a str> {
    metadata
        .iter()
        .rfind(|entry| entry.key == key)
        .map(|entry| entry.value.as_str())
}

fn metadata(key: &str, value: &str) -> MetadataEntry {
    MetadataEntry {
        key: key.to_owned(),
        value: value.to_owned(),
        owner_component_id: None,
    }
}

fn annotate(payload: &mut EffectPayload, key: &str, value: &str) {
    if let Some(candidate) = payload.candidate.as_mut() {
        if let Some(entry) = candidate
            .metadata
            .iter_mut()
            .rfind(|entry| entry.key == key)
        {
            entry.value = value.to_owned();
        } else {
            candidate.metadata.push(metadata(key, value));
        }
    }
}

fn mark_unresolved(payload: &mut EffectPayload, code: &str) {
    let Some(candidate) = payload.candidate.as_mut() else {
        return;
    };
    if !candidate
        .metadata
        .iter()
        .any(|entry| entry.key == "semantic-state" && entry.value == "unresolved")
    {
        candidate
            .metadata
            .push(metadata("semantic-state", "unresolved"));
    }
    candidate
        .metadata
        .push(metadata("semantic-unresolved", code));
}

fn warning(code: &str, message: impl Into<String>, span: MappedSpan) -> Diagnostic {
    Diagnostic {
        code: code.to_owned(),
        message: message.into(),
        severity: DiagnosticSeverity::Warning,
        span,
        related: Vec::new(),
    }
}

fn continue_with_diagnostics(payload: EffectPayload, diagnostics: Vec<Diagnostic>) -> HookOutput {
    let mut effects = empty_effects();
    effects.diagnostics = diagnostics;
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Effect(payload)),
        effects,
    }
}

fn accept(payload: EffectPayload) -> HookOutput {
    continue_with_diagnostics(payload, Vec::new())
}

fn reject_with(message: impl Into<String>, code: &str, span: MappedSpan) -> HookOutput {
    let message = message.into();
    HookOutput {
        decision: HookDecision::Reject(Rejection {
            reason: message.clone(),
            diagnostics: vec![Diagnostic {
                code: code.to_owned(),
                message,
                severity: DiagnosticSeverity::Error,
                span,
                related: Vec::new(),
            }],
        }),
        replacement: None,
        effects: empty_effects(),
    }
}

fn event_relation(
    context: &ParseContext,
    target_class: &str,
) -> Result<crate::catalog::TypeRelation, String> {
    use crate::catalog::TypeRelation;

    if context.event_classes.is_empty() {
        return Ok(TypeRelation::Incompatible);
    }
    let mut unknown = false;
    for event_class in &context.event_classes {
        if event_class == target_class {
            return Ok(TypeRelation::Compatible);
        }
        match crate::catalog::is_class_assignable(event_class, target_class)? {
            TypeRelation::Compatible => return Ok(TypeRelation::Compatible),
            TypeRelation::Incompatible => {}
            TypeRelation::Unknown => unknown = true,
        }
    }
    Ok(if unknown {
        TypeRelation::Unknown
    } else {
        TypeRelation::Incompatible
    })
}

fn set_contract_verdict(
    capture: &ParsedCapture,
    required_source_types: &[&str],
) -> Result<ContractVerdict, String> {
    set_contract_verdict_with(
        capture,
        required_source_types,
        crate::catalog::is_class_assignable,
    )
}

fn set_contract_with_converters_verdict(
    capture: &ParsedCapture,
    required_source_types: &[&str],
) -> Result<ContractVerdict, String> {
    set_contract_verdict_with(capture, required_source_types, crate::catalog::can_convert)
}

fn set_contract_verdict_with(
    capture: &ParsedCapture,
    required_source_types: &[&str],
    relation: impl Fn(&str, &str) -> Result<crate::catalog::TypeRelation, String>,
) -> Result<ContractVerdict, String> {
    use crate::catalog::ChangeContract;

    let Some(summary) = capture.summary.as_ref() else {
        return Ok(ContractVerdict::Unresolved);
    };
    if summary.kind == "variable" {
        return Ok(ContractVerdict::Accepted);
    }
    let subject_id = summary
        .registration_id
        .as_deref()
        .unwrap_or(capture.parser_id.as_str());
    let contract =
        match crate::catalog::change_contract_from_metadata(&summary.metadata, subject_id)? {
            Some(contract) => Some(contract),
            None => eff_change::source_change_contract(summary)?,
        };
    let Some(ChangeContract::Resolved { modes }) = contract else {
        return Ok(ContractVerdict::Unresolved);
    };
    let Some(accepted_types) = modes.get("SET") else {
        return Ok(ContractVerdict::Rejected);
    };
    accepted_types_verdict(accepted_types, required_source_types, relation)
}

fn accepted_types_verdict(
    accepted_types: &[crate::catalog::AcceptedChangeType],
    required_source_types: &[&str],
    relation: impl Fn(&str, &str) -> Result<crate::catalog::TypeRelation, String>,
) -> Result<ContractVerdict, String> {
    use crate::catalog::TypeRelation;

    let mut unresolved = false;
    for source_type in required_source_types {
        let mut compatible = false;
        let mut source_unresolved = false;
        for accepted in accepted_types {
            match relation(source_type, &accepted.class_name)? {
                TypeRelation::Compatible => compatible = true,
                TypeRelation::Incompatible => {}
                TypeRelation::Unknown => source_unresolved = true,
            }
        }
        if !compatible {
            if source_unresolved {
                unresolved = true;
            } else {
                return Ok(ContractVerdict::Rejected);
            }
        }
    }
    Ok(if unresolved {
        ContractVerdict::Unresolved
    } else {
        ContractVerdict::Accepted
    })
}

fn controls_loop(frame: &crate::nlaocs::skript_parser_addon::types::SectionScopeFrame) -> bool {
    frame.loop_section
        || frame
            .metadata
            .iter()
            .any(|entry| entry.key == "while-loop-control" && entry.value == "true")
}

#[cfg(test)]
mod tests {
    use super::{ContractVerdict, accepted_types_verdict, event_relation};
    use crate::catalog::{AcceptedChangeType, TypeRelation};
    use crate::nlaocs::skript_parser_addon::types::ParseContext;

    #[test]
    fn an_empty_event_stack_is_outside_every_event() {
        let context = ParseContext {
            syntax_context: 0,
            event_classes: Vec::new(),
            section_stack: Vec::new(),
            values: Vec::new(),
        };
        assert_eq!(
            event_relation(&context, "org.bukkit.event.Event").unwrap(),
            TypeRelation::Incompatible
        );
    }

    #[test]
    fn converter_aware_change_checks_preserve_unknown_relations() {
        let accepted = vec![AcceptedChangeType {
            class_name: "java.lang.String".to_owned(),
            multiple: false,
        }];
        let relation = |source: &str, target: &str| {
            Ok(
                if source == "example.TextLike" && target == "java.lang.String" {
                    TypeRelation::Compatible
                } else {
                    TypeRelation::Unknown
                },
            )
        };

        assert_eq!(
            accepted_types_verdict(&accepted, &["example.TextLike"], relation).unwrap(),
            ContractVerdict::Accepted
        );
        assert_eq!(
            accepted_types_verdict(&accepted, &["example.Unknown"], relation).unwrap(),
            ContractVerdict::Unresolved
        );
    }
}

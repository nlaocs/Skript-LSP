mod cond_can_hold;
mod cond_chance;
mod cond_compare;
mod cond_is_jumping;
mod cond_is_pressing_key;
mod cond_is_set;
mod cond_matches;
mod cond_script_loaded;
mod event_context;
mod prop_cond_contains;

use crate::nlaocs::skript_parser_addon::types::{
    AddonError, AddonErrorKind, ConditionCapture, ConditionPayload, Diagnostic, DiagnosticSeverity,
    HookDecision, HookInvocation, HookOutput, HookPayload, HookPhase, MappedSpan, MetadataEntry,
    RegisteredExpressionChild, RegisteredSyntaxHandler, RegisteredSyntaxHandlerTarget, Rejection,
    SyntaxKind,
};
use crate::{addon_error, empty_effects, not_applicable};

pub(crate) fn handlers() -> Vec<RegisteredSyntaxHandler> {
    let mut handlers = Vec::new();
    cond_chance::register(&mut handlers);
    cond_can_hold::register(&mut handlers);
    cond_compare::register(&mut handlers);
    cond_is_jumping::register(&mut handlers);
    cond_is_pressing_key::register(&mut handlers);
    cond_is_set::register(&mut handlers);
    cond_matches::register(&mut handlers);
    cond_script_loaded::register(&mut handlers);
    prop_cond_contains::register(&mut handlers);
    event_context::register(&mut handlers);
    handlers
}

pub(crate) fn parse(input: HookInvocation) -> Result<HookOutput, AddonError> {
    if !matches!(input.phase, HookPhase::Condition) {
        return Err(addon_error(
            AddonErrorKind::InvalidPayload,
            "CoreLibrary Condition semantics require the Condition phase",
        ));
    }
    let HookPayload::Condition(payload) = input.payload else {
        return Err(addon_error(
            AddonErrorKind::InvalidPayload,
            "CoreLibrary Condition semantics require a Condition payload",
        ));
    };
    Ok(cond_chance::resolve(payload.clone())
        .or_else(|| cond_can_hold::resolve(payload.clone()))
        .or_else(|| cond_compare::resolve(payload.clone(), &input.parse_results))
        .or_else(|| cond_is_jumping::resolve(payload.clone()))
        .or_else(|| cond_is_pressing_key::resolve(payload.clone()))
        .or_else(|| cond_is_set::resolve(payload.clone()))
        .or_else(|| cond_matches::resolve(payload.clone()))
        .or_else(|| cond_script_loaded::resolve(payload.clone()))
        .or_else(|| prop_cond_contains::resolve(payload.clone()))
        .or_else(|| event_context::resolve(payload))
        .unwrap_or_else(not_applicable))
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
        kind: SyntaxKind::Condition,
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

fn event_relation(
    context: &crate::nlaocs::skript_parser_addon::types::ParseContext,
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

fn matches(payload: &ConditionPayload, handler_id: &str) -> bool {
    crate::runtime::handler_matches(handler_id, &payload.candidate.registration_id)
}

fn child(payload: &ConditionPayload, index: usize) -> Option<&RegisteredExpressionChild> {
    payload.candidate.children.get(index)
}

fn child_span(payload: &ConditionPayload, index: usize) -> MappedSpan {
    payload
        .candidate
        .captures
        .iter()
        .filter_map(|capture| match capture {
            ConditionCapture::Expression(capture) => Some(&capture.span),
            ConditionCapture::Regex(_) => None,
        })
        .nth(index)
        .cloned()
        .unwrap_or_else(|| payload.candidate.span.clone())
}

fn child_types(child: &RegisteredExpressionChild) -> Vec<&str> {
    let mut types = child
        .possible_return_types
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    if let Some(return_type) = child.return_type.as_deref()
        && !types.contains(&return_type)
    {
        types.push(return_type);
    }
    types
}

fn metadata(key: &str, value: &str) -> MetadataEntry {
    MetadataEntry {
        key: key.to_owned(),
        value: value.to_owned(),
        owner_component_id: None,
    }
}

fn annotate(payload: &mut ConditionPayload, key: &str, value: &str) {
    payload.candidate.metadata.push(metadata(key, value));
}

fn mark_unresolved(payload: &mut ConditionPayload, code: &str) {
    if !payload
        .candidate
        .metadata
        .iter()
        .any(|entry| entry.key == "semantic-state" && entry.value == "unresolved")
    {
        annotate(payload, "semantic-state", "unresolved");
    }
    annotate(payload, "semantic-unresolved", code);
}

fn accept(payload: ConditionPayload) -> HookOutput {
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Condition(payload)),
        effects: empty_effects(),
    }
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

#[cfg(test)]
mod tests {
    use super::event_relation;
    use crate::catalog::TypeRelation;
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
}

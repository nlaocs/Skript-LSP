use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, EffectPayload, HookOutput, MappedSpan, RegisteredSyntaxHandler,
    RegisteredSyntaxHandlerTarget, SyntaxKind,
};
use crate::reject;

const CLASS_SUFFIX: &str = ".EffSort";
const HANDLER_ID: &str = "core.effect.eff-sort";

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
        capture_parsers: vec![super::eff_transform::input_expression_binding(
            false,
            "expressions-only",
        )],
        context_requirements: Vec::new(),
    });
}

pub(super) fn resolve(mut payload: EffectPayload) -> Option<HookOutput> {
    let candidate = payload.candidate.as_ref()?;
    if !crate::runtime::handler_matches(HANDLER_ID, &candidate.registration_id) {
        return None;
    }
    let Some(target) = super::eff_transform::expression_capture(candidate, 0) else {
        return Some(reject("sort requires a list variable"));
    };
    let target_span = target.span.clone();
    let Some(summary) = target.summary.as_ref() else {
        return Some(unresolved(
            payload,
            "core.eff-sort.unresolved-target-multiplicity",
            "the sort target's multiplicity is unresolved, so list-variable validation was deferred",
            target_span,
        ));
    };
    if summary.kind != "variable" {
        return Some(reject("you can only sort list variables"));
    }
    match list_multiplicity_verdict(summary.multiplicity) {
        MultiplicityVerdict::Accepted => {}
        MultiplicityVerdict::Rejected => {
            return Some(reject("you can only sort list variables"));
        }
        MultiplicityVerdict::Unresolved => {
            return Some(unresolved(
                payload,
                "core.eff-sort.unresolved-target-multiplicity",
                "the sort target's multiplicity is unresolved, so list-variable validation was deferred",
                target_span,
            ));
        }
    }
    if !super::eff_transform::parsed_input_expression(candidate, false) {
        return Some(reject("sort mapping is not a valid Expression"));
    }
    if let Some(mapping) = super::eff_transform::mapping_capture(candidate) {
        let mapping_span = mapping.span.clone();
        let Some(summary) = mapping.summary.as_ref() else {
            return Some(unresolved(
                payload,
                "core.eff-sort.unresolved-mapping-multiplicity",
                "the sort mapping's multiplicity is unresolved, so single-value validation was deferred",
                mapping_span,
            ));
        };
        match single_multiplicity_verdict(summary.multiplicity) {
            MultiplicityVerdict::Accepted => {}
            MultiplicityVerdict::Rejected => {
                return Some(reject(
                    "the mapping Expression in the sort Effect must return a single value",
                ));
            }
            MultiplicityVerdict::Unresolved => {
                return Some(unresolved(
                    payload,
                    "core.eff-sort.unresolved-mapping-multiplicity",
                    "the sort mapping's multiplicity is unresolved, so single-value validation was deferred",
                    mapping_span,
                ));
            }
        }
    }
    let descending = candidate.tags.iter().any(|tag| tag.value == "descending");

    payload
        .candidate
        .as_mut()
        .expect("candidate was checked")
        .metadata
        .extend([
            super::eff_transform::metadata("semantic-mode", "sort-list"),
            super::eff_transform::metadata(
                "sort-order",
                if descending {
                    "descending"
                } else {
                    "ascending"
                },
            ),
        ]);
    Some(super::eff_transform::accept(payload))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MultiplicityVerdict {
    Accepted,
    Rejected,
    Unresolved,
}

fn list_multiplicity_verdict(multiplicity: Option<DynamicMultiplicity>) -> MultiplicityVerdict {
    match multiplicity {
        Some(DynamicMultiplicity::Multiple) => MultiplicityVerdict::Accepted,
        Some(DynamicMultiplicity::Single) => MultiplicityVerdict::Rejected,
        Some(DynamicMultiplicity::Both) | None => MultiplicityVerdict::Unresolved,
    }
}

fn single_multiplicity_verdict(multiplicity: Option<DynamicMultiplicity>) -> MultiplicityVerdict {
    match multiplicity {
        Some(DynamicMultiplicity::Single) => MultiplicityVerdict::Accepted,
        Some(DynamicMultiplicity::Multiple) => MultiplicityVerdict::Rejected,
        Some(DynamicMultiplicity::Both) | None => MultiplicityVerdict::Unresolved,
    }
}

fn unresolved(
    mut payload: EffectPayload,
    code: &str,
    message: &str,
    span: MappedSpan,
) -> HookOutput {
    super::mark_unresolved(&mut payload, code);
    super::continue_with_diagnostics(payload, vec![super::warning(code, message, span)])
}

#[cfg(test)]
mod tests {
    use super::{
        DynamicMultiplicity, MultiplicityVerdict, list_multiplicity_verdict,
        single_multiplicity_verdict,
    };

    #[test]
    fn sort_mapping_is_optional_and_expression_only() {
        let binding =
            super::super::eff_transform::input_expression_binding(false, "expressions-only");
        assert!(!binding.required);
        assert!(
            binding
                .options
                .iter()
                .any(|entry| { entry.key == "parse.mode" && entry.value == "expressions-only" })
        );
    }

    #[test]
    fn unknown_and_both_multiplicity_are_unresolved() {
        assert_eq!(
            list_multiplicity_verdict(None),
            MultiplicityVerdict::Unresolved
        );
        assert_eq!(
            list_multiplicity_verdict(Some(DynamicMultiplicity::Both)),
            MultiplicityVerdict::Unresolved
        );
        assert_eq!(
            single_multiplicity_verdict(None),
            MultiplicityVerdict::Unresolved
        );
        assert_eq!(
            single_multiplicity_verdict(Some(DynamicMultiplicity::Both)),
            MultiplicityVerdict::Unresolved
        );
    }
}

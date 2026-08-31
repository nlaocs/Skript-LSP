use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, EffectPayload, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".EffCopy";
const HANDLER_ID: &str = "core.effect.eff-copy";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX);
}

pub(super) fn resolve(
    mut payload: EffectPayload,
) -> Option<crate::nlaocs::skript_parser_addon::types::HookOutput> {
    if !super::matches(&payload, HANDLER_ID) {
        return None;
    }
    let candidate_span = payload.candidate.as_ref()?.span.clone();
    super::annotate(&mut payload, "semantic-mode", "copy-to-variable");
    let Some(source) = super::parsed_capture(&payload, 0) else {
        return Some(unresolved(
            payload,
            "core.eff-copy.missing-source",
            "the copy source was not parsed",
            candidate_span,
        ));
    };
    let source_multiplicity = source
        .summary
        .as_ref()
        .and_then(|summary| summary.multiplicity);
    let Some(destination) = super::parsed_capture(&payload, 1) else {
        return Some(unresolved(
            payload,
            "core.eff-copy.missing-destination",
            "the copy destination was not parsed",
            candidate_span,
        ));
    };
    let destination_span = destination.span.clone();
    let Some(destination_summary) = destination.summary.as_ref() else {
        return Some(unresolved(
            payload,
            "core.eff-copy.unresolved-destination",
            "the copy destination has no semantic summary",
            destination_span,
        ));
    };

    match copy_decision(
        destination_summary.kind.as_str(),
        source_multiplicity,
        destination_summary.multiplicity,
        super::metadata_value(
            &destination_summary.metadata,
            "parser.expression-list.all-variables",
        )
        .and_then(parse_bool),
        super::metadata_value(
            &destination_summary.metadata,
            "parser.expression-list.any-single-variable",
        )
        .and_then(parse_bool),
    ) {
        CopyDecision::Accepted => Some(super::accept(payload)),
        CopyDecision::RejectDestination => Some(super::reject_with(
            "objects can only be copied into variables",
            "core.eff-copy.non-variable-destination",
            destination_span,
        )),
        CopyDecision::RejectMultiplicity => Some(super::reject_with(
            "multiple objects cannot be copied into a single variable",
            "core.eff-copy.multiple-to-single",
            destination_span,
        )),
        CopyDecision::UnresolvedExpressionList => Some(unresolved(
            payload,
            "core.eff-copy.unresolved-expression-list",
            "the destination is an Expression list, but child summaries are unavailable to verify that every element is a variable",
            destination_span,
        )),
        CopyDecision::UnresolvedMultiplicity => Some(unresolved(
            payload,
            "core.eff-copy.unresolved-multiplicity",
            "the source or destination multiplicity is unresolved",
            destination_span,
        )),
    }
}

fn unresolved(
    mut payload: EffectPayload,
    code: &str,
    message: &str,
    span: crate::nlaocs::skript_parser_addon::types::MappedSpan,
) -> crate::nlaocs::skript_parser_addon::types::HookOutput {
    super::mark_unresolved(&mut payload, code);
    super::continue_with_diagnostics(payload, vec![super::warning(code, message, span)])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyDecision {
    Accepted,
    RejectDestination,
    RejectMultiplicity,
    UnresolvedExpressionList,
    UnresolvedMultiplicity,
}

fn copy_decision(
    destination_kind: &str,
    source: Option<DynamicMultiplicity>,
    destination: Option<DynamicMultiplicity>,
    all_variables: Option<bool>,
    any_single_variable: Option<bool>,
) -> CopyDecision {
    let destination_single = match destination_kind {
        "variable" => destination == Some(DynamicMultiplicity::Single),
        "expression-list" => match (all_variables, any_single_variable) {
            (Some(true), Some(single)) => single,
            (Some(false), _) => return CopyDecision::RejectDestination,
            _ => return CopyDecision::UnresolvedExpressionList,
        },
        _ => return CopyDecision::RejectDestination,
    };
    match source {
        Some(DynamicMultiplicity::Multiple) if destination_single => {
            CopyDecision::RejectMultiplicity
        }
        Some(DynamicMultiplicity::Single | DynamicMultiplicity::Multiple) => CopyDecision::Accepted,
        _ => CopyDecision::UnresolvedMultiplicity,
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value {
        value if value.eq_ignore_ascii_case("true") => Some(true),
        value if value.eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{CopyDecision, copy_decision};
    use crate::nlaocs::skript_parser_addon::types::DynamicMultiplicity;

    #[test]
    fn enforces_variable_destinations_and_cardinality() {
        assert_eq!(
            copy_decision(
                "variable",
                Some(DynamicMultiplicity::Multiple),
                Some(DynamicMultiplicity::Single),
                None,
                None,
            ),
            CopyDecision::RejectMultiplicity
        );
        assert_eq!(
            copy_decision(
                "registered-expression",
                Some(DynamicMultiplicity::Single),
                Some(DynamicMultiplicity::Single),
                None,
                None,
            ),
            CopyDecision::RejectDestination
        );
        assert_eq!(
            copy_decision(
                "expression-list",
                Some(DynamicMultiplicity::Single),
                Some(DynamicMultiplicity::Multiple),
                None,
                None,
            ),
            CopyDecision::UnresolvedExpressionList
        );
        assert_eq!(
            copy_decision(
                "expression-list",
                Some(DynamicMultiplicity::Multiple),
                Some(DynamicMultiplicity::Multiple),
                Some(true),
                Some(true),
            ),
            CopyDecision::RejectMultiplicity
        );
    }
}

use crate::nlaocs::skript_parser_addon::types::{
    ContextUpdate, Diagnostic, EffectPayload, HookOutput, RegisteredSyntaxHandler,
};

const CLASS_SUFFIX: &str = ".Delay";
const HANDLER_ID: &str = "core.effect.delay";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX);
}

pub(super) fn resolve(
    mut payload: EffectPayload,
) -> Option<crate::nlaocs::skript_parser_addon::types::HookOutput> {
    if !super::matches(&payload, HANDLER_ID) {
        return None;
    }
    if payload
        .context
        .values
        .iter()
        .rfind(|entry| entry.key == "core.section.catch-errors")
        .is_some_and(|entry| entry.value == "true")
    {
        let span = payload.candidate.as_ref()?.span.clone();
        return Some(super::reject_with(
            "delays cannot be used within a catch errors section",
            "core.delay.catch-errors-section",
            span,
        ));
    }
    super::annotate(&mut payload, "semantic-mode", "delay");

    let Some(duration) = super::parsed_capture(&payload, 0) else {
        let span = payload.candidate.as_ref()?.span.clone();
        super::mark_unresolved(&mut payload, "core.delay.missing-duration");
        return Some(accept_delayed(
            payload,
            vec![super::warning(
                "core.delay.missing-duration",
                "the delay duration was not parsed, so literal delay checks were skipped",
                span,
            )],
        ));
    };
    let span = duration.span.clone();
    let Some(summary) = duration.summary.as_ref() else {
        super::mark_unresolved(&mut payload, "core.delay.unresolved-duration");
        return Some(accept_delayed(
            payload,
            vec![super::warning(
                "core.delay.unresolved-duration",
                "the delay duration has no semantic summary",
                span,
            )],
        ));
    };
    if summary.kind != "literal" {
        return Some(accept_delayed(payload, Vec::new()));
    }

    let infinite =
        super::metadata_value(&summary.metadata, "timespan-infinite").and_then(parse_bool);
    let millis = super::metadata_value(&summary.metadata, "timespan-milliseconds")
        .and_then(|value| value.parse::<u64>().ok());
    let modern_infinite_guard = crate::runtime::skript_at_least(2, 12);
    match duration_semantics(infinite, millis, modern_infinite_guard) {
        DurationSemantics::RejectInfinite => Some(super::reject_with(
            "delaying for an eternity is not allowed; use the stop Effect instead",
            "core.delay.infinite-duration",
            span,
        )),
        DurationSemantics::WarnSubTick => Some(accept_delayed(
            payload,
            vec![Diagnostic {
                code: "core.delay.sub-tick-duration".to_owned(),
                message: "delays shorter than one tick are rounded up to one tick".to_owned(),
                severity: crate::nlaocs::skript_parser_addon::types::DiagnosticSeverity::Warning,
                span,
                related: Vec::new(),
            }],
        )),
        DurationSemantics::Unresolved => {
            super::mark_unresolved(&mut payload, "core.delay.unresolved-literal-duration");
            Some(accept_delayed(
                payload,
                vec![super::warning(
                    "core.delay.unresolved-literal-duration",
                    "the literal timespan does not expose enough data for Skript's delay sanity checks",
                    span,
                )],
            ))
        }
        DurationSemantics::Accepted => Some(accept_delayed(payload, Vec::new())),
    }
}

fn accept_delayed(mut payload: EffectPayload, diagnostics: Vec<Diagnostic>) -> HookOutput {
    if crate::runtime::skript_version_is("2.15.4") && payload.candidate.as_ref().is_some_and(|candidate|
        crate::catalog::standard_effect_source(candidate,"ch.njol.skript.effects.Delay")) {
        super::annotate(&mut payload,"skript.semantic.binding-effects",r#"{"version":1,"local":"preserve","shared":"suspend"}"#);
        super::annotate(&mut payload,crate::public_data::OPERAND_EVALUATION_METADATA_KEY,r#"{"version":1,"prefix":[0]}"#);
        let composition=super::parsed_capture(&payload,0).and_then(|capture|capture.summary.as_ref())
            .and_then(|summary|crate::storage::native_inputs::compose_effect_input(summary,0,&["skript.semantic.binding-effects",crate::public_data::OPERAND_EVALUATION_METADATA_KEY]));
        if let Some(composition)=composition {super::annotate(&mut payload,crate::storage::native_inputs::COMPOSE,&composition);}
    }
    let syntax_context = payload.context.syntax_context;
    let mut output = super::continue_with_diagnostics(payload, diagnostics);
    output.effects.context_updates.push(ContextUpdate {
        syntax_context,
        key: super::DELAY_STATE_KEY.to_owned(),
        value: Some(b"true".to_vec()),
    });
    output
}

fn parse_bool(value: &str) -> Option<bool> {
    match value {
        value if value.eq_ignore_ascii_case("true") => Some(true),
        value if value.eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DurationSemantics {
    Accepted,
    WarnSubTick,
    RejectInfinite,
    Unresolved,
}

fn duration_semantics(
    infinite: Option<bool>,
    millis: Option<u64>,
    modern_infinite_guard: Option<bool>,
) -> DurationSemantics {
    if infinite == Some(true) {
        return match modern_infinite_guard {
            Some(true) => DurationSemantics::RejectInfinite,
            Some(false) => DurationSemantics::Accepted,
            None => DurationSemantics::Unresolved,
        };
    }
    match millis {
        Some(0..=49) => DurationSemantics::WarnSubTick,
        Some(_) => DurationSemantics::Accepted,
        None => DurationSemantics::Unresolved,
    }
}

#[cfg(test)]
mod tests {
    use super::{DurationSemantics, duration_semantics};

    #[test]
    fn mirrors_literal_delay_guards_across_supported_generations() {
        assert_eq!(
            duration_semantics(Some(true), None, Some(false)),
            DurationSemantics::Accepted
        );
        assert_eq!(
            duration_semantics(Some(true), None, Some(true)),
            DurationSemantics::RejectInfinite
        );
        assert_eq!(
            duration_semantics(Some(false), Some(49), Some(true)),
            DurationSemantics::WarnSubTick
        );
        assert_eq!(
            duration_semantics(Some(false), Some(50), Some(true)),
            DurationSemantics::Accepted
        );
    }
}

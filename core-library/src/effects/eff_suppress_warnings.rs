use crate::nlaocs::skript_parser_addon::types::{EffectPayload, RegisteredSyntaxHandler};

const CLASS_SUFFIX: &str = ".EffSuppressWarnings";
const HANDLER_ID: &str = "core.effect.eff-suppress-warnings";
const ACTIVE_SCRIPT_KEY: &str = "parser.active";
const SUPPRESSED_WARNINGS_KEY: &str = "parser.suppressed-warnings";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    super::register_handler(handlers, HANDLER_ID, CLASS_SUFFIX);
}

pub(super) fn resolve(
    mut payload: EffectPayload,
) -> Option<crate::nlaocs::skript_parser_addon::types::HookOutput> {
    if !super::matches(&payload, HANDLER_ID) {
        return None;
    }
    let candidate = payload.candidate.as_ref()?;
    let span = candidate.span.clone();
    let mark = candidate.mark;
    let pattern = candidate.pattern.clone();
    super::annotate(&mut payload, "semantic-mode", "suppress-script-warning");
    super::annotate(&mut payload, "script-warning-mark", &mark.to_string());
    match active_state(super::context_bool(&payload.context, ACTIVE_SCRIPT_KEY)) {
        ActiveState::Active => Some(apply_warning_state(payload, pattern.as_str(), mark)),
        ActiveState::Inactive => Some(super::reject_with(
            "warnings cannot be suppressed outside of a script",
            "core.eff-suppress-warnings.inactive-script",
            span,
        )),
        ActiveState::Unresolved => {
            super::mark_unresolved(&mut payload, "core.eff-suppress-warnings.unresolved-script");
            Some(super::continue_with_diagnostics(
                payload,
                vec![super::warning(
                    "core.eff-suppress-warnings.unresolved-script",
                    "the parser did not expose whether a script is active",
                    span,
                )],
            ))
        }
    }
}

fn apply_warning_state(
    mut payload: EffectPayload,
    pattern: &str,
    mark: i32,
) -> crate::nlaocs::skript_parser_addon::types::HookOutput {
    let syntax_context = payload.context.syntax_context;
    match warning_action(pattern, mark) {
        WarningAction::Suppress(name) => {
            super::annotate(&mut payload, "script-warning-name", name);
            let warnings = add_warning(
                super::context_value(&payload.context, SUPPRESSED_WARNINGS_KEY),
                name,
            );
            let mut output = super::accept(payload);
            super::add_context_update(
                &mut output,
                syntax_context,
                SUPPRESSED_WARNINGS_KEY,
                Some(warnings.as_bytes()),
            );
            output
        }
        WarningAction::Deprecated { name, message } => {
            super::annotate(&mut payload, "script-warning-name", name);
            let span = payload_span(&payload);
            super::continue_with_diagnostics(
                payload,
                vec![super::warning(
                    "core.eff-suppress-warnings.deprecated",
                    message,
                    span,
                )],
            )
        }
        WarningAction::Unresolved => {
            let span = payload_span(&payload);
            super::mark_unresolved(
                &mut payload,
                "core.eff-suppress-warnings.unresolved-warning",
            );
            super::continue_with_diagnostics(
                payload,
                vec![super::warning(
                    "core.eff-suppress-warnings.unresolved-warning",
                    "the warning mark could not be mapped to a known Skript warning",
                    span,
                )],
            )
        }
    }
}

fn payload_span(payload: &EffectPayload) -> crate::nlaocs::skript_parser_addon::types::MappedSpan {
    payload
        .candidate
        .as_ref()
        .map(|candidate| candidate.span.clone())
        .unwrap_or_else(|| payload.span.clone())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WarningAction {
    Suppress(&'static str),
    Deprecated {
        name: &'static str,
        message: &'static str,
    },
    Unresolved,
}

fn warning_action(pattern: &str, mark: i32) -> WarningAction {
    let modern = if pattern.contains("(0:conflict") || pattern.contains("(0\u{00a6}conflict") {
        Some(true)
    } else if pattern.contains("(1:conflict") || pattern.contains("(1\u{00a6}conflict") {
        Some(false)
    } else {
        crate::runtime::skript_at_least(2, 10)
    };
    let index = match modern {
        Some(true) => usize::try_from(mark).ok(),
        Some(false) => mark
            .checked_sub(1)
            .and_then(|value| usize::try_from(value).ok()),
        None => None,
    };
    let Some(index) = index else {
        return WarningAction::Unresolved;
    };
    let names = if modern == Some(true) {
        &[
            "variable-conflict",
            "variable-save",
            "missing-conjunction",
            "starting-expression",
            "variable-contains-colon",
            "deprecated-syntax",
            "unreachable-code",
            "constant-condition",
        ][..]
    } else {
        &[
            "variable-conflict",
            "variable-save",
            "missing-conjunction",
            "starting-expression",
            "deprecated-syntax",
        ][..]
    };
    match names.get(index).copied() {
        Some("variable-conflict") => WarningAction::Deprecated {
            name: "variable-conflict",
            message: "variable conflict warnings no longer need suppression, as they have been removed altogether",
        },
        Some(name) => WarningAction::Suppress(name),
        None => WarningAction::Unresolved,
    }
}

fn add_warning(existing: Option<&str>, warning: &str) -> String {
    let mut warnings = existing
        .unwrap_or_default()
        .split(';')
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if !warnings.iter().any(|value| value == warning) {
        warnings.push(warning.to_owned());
    }
    warnings.join(";")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveState {
    Active,
    Inactive,
    Unresolved,
}

fn active_state(active: Option<bool>) -> ActiveState {
    match active {
        Some(true) => ActiveState::Active,
        Some(false) => ActiveState::Inactive,
        None => ActiveState::Unresolved,
    }
}

#[cfg(test)]
mod tests {
    use super::{ActiveState, WarningAction, active_state, add_warning, warning_action};

    #[test]
    fn rejects_only_an_explicitly_inactive_parser() {
        assert_eq!(active_state(Some(true)), ActiveState::Active);
        assert_eq!(active_state(Some(false)), ActiveState::Inactive);
        assert_eq!(active_state(None), ActiveState::Unresolved);
    }

    #[test]
    fn maps_modern_and_legacy_marks_without_suppressing_deprecated_conflicts() {
        assert_eq!(
            warning_action("[local] suppress (0:conflict|1:variable save)", 1),
            WarningAction::Suppress("variable-save")
        );
        assert!(matches!(
            warning_action("[local] suppress (0:conflict|1:variable save)", 0),
            WarningAction::Deprecated { .. }
        ));
        assert_eq!(
            warning_action("[local] suppress (1:conflict|2:variable save)", 2),
            WarningAction::Suppress("variable-save")
        );
        assert_eq!(
            warning_action(
                "[local] suppress (0\u{00a6}conflict|1\u{00a6}variable save)",
                1
            ),
            WarningAction::Suppress("variable-save")
        );
    }

    #[test]
    fn warning_state_is_a_deduplicated_set() {
        assert_eq!(
            add_warning(None, "missing-conjunction"),
            "missing-conjunction"
        );
        assert_eq!(
            add_warning(
                Some("variable-save;missing-conjunction"),
                "missing-conjunction"
            ),
            "variable-save;missing-conjunction"
        );
    }
}

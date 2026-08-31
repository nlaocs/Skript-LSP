use super::{
    context_value_update, continue_with_section_context, register_handler, reject_section,
};
use crate::nlaocs::skript_parser_addon::types::{
    HookOutput, InvocationContext, SectionBodyMode, SectionPayload, SectionRawNodeKind,
    SectionTiming,
};

const CLASS_SUFFIX: &str = ".SecCatchErrors";
const HANDLER_ID: &str = "core.section.sec-catch-errors";

pub(super) fn register(
    handlers: &mut Vec<crate::nlaocs::skript_parser_addon::types::RegisteredSyntaxHandler>,
) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn matches(payload: &SectionPayload) -> bool {
    crate::runtime::handler_matches(HANDLER_ID, &payload.candidate.registration_id)
}

pub(super) fn resolve(context: InvocationContext, mut payload: SectionPayload) -> HookOutput {
    payload.candidate.body_mode = SectionBodyMode::Trigger;
    if matches!(payload.timing, SectionTiming::ExitChildren) {
        let delayed = payload
            .context
            .values
            .iter()
            .rfind(|entry| entry.key == "parser.delay-state")
            .is_some_and(|entry| entry.value == "true");
        if should_reject_delayed_exit(payload.timing, delayed) {
            return reject_section("delays cannot be used within a catch errors section");
        }
        return continue_with_section_context(&context, payload, [], Vec::new());
    }
    if !crate::experiments::enabled(&payload.context, "catch runtime errors") {
        return reject_section("the `catch runtime errors` experiment is not enabled");
    }
    let has_body = has_meaningful_body(payload.raw_children.iter().map(|child| &child.kind));
    if !has_body {
        return reject_section("catch errors sections must contain at least one statement");
    }

    let metadata = [
        ("semantic-mode", "catch-errors".to_owned()),
        ("catch-errors.feature", "enabled".to_owned()),
        ("catch-errors.delay-check", "enabled".to_owned()),
    ];
    let updates = vec![
        context_value_update(&context, "core.section.catch-errors", "true"),
        context_value_update(&context, "parser.delay-state", "false"),
    ];
    continue_with_section_context(&context, payload, metadata, updates)
}

fn has_meaningful_body<'a>(kinds: impl IntoIterator<Item = &'a SectionRawNodeKind>) -> bool {
    kinds.into_iter().any(|kind| {
        !matches!(
            kind,
            SectionRawNodeKind::Blank | SectionRawNodeKind::Comment
        )
    })
}

fn should_reject_delayed_exit(timing: SectionTiming, delayed: bool) -> bool {
    matches!(timing, SectionTiming::ExitChildren) && delayed
}

#[cfg(test)]
mod tests {
    use super::{has_meaningful_body, should_reject_delayed_exit};
    use crate::nlaocs::skript_parser_addon::types::{SectionRawNodeKind, SectionTiming};

    #[test]
    fn empty_body_matches_section_node_void_node_semantics() {
        let empty = [SectionRawNodeKind::Blank, SectionRawNodeKind::Comment];
        assert!(!has_meaningful_body(empty.iter()));

        let with_statement = [SectionRawNodeKind::Comment, SectionRawNodeKind::Simple];
        assert!(has_meaningful_body(with_statement.iter()));
    }

    #[test]
    fn delayed_exit_is_rejected_instead_of_continued_with_a_diagnostic() {
        assert!(should_reject_delayed_exit(
            SectionTiming::ExitChildren,
            true
        ));
        assert!(!should_reject_delayed_exit(
            SectionTiming::ExitChildren,
            false
        ));
        assert!(!should_reject_delayed_exit(
            SectionTiming::EnterChildren,
            true
        ));
    }
}

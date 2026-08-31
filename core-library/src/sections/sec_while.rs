use super::{
    condition_binding, condition_captures_are_parsed, continue_with_section_context,
    register_handler, reject_section,
};
use crate::nlaocs::skript_parser_addon::types::{
    ContextUpdate, HookOutput, InvocationContext, RegisteredSyntaxHandler, SectionPayload,
};

const CLASS_SUFFIX: &str = ".SecWhile";
const HANDLER_ID: &str = "core.section.sec-while";

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(
        handlers,
        HANDLER_ID,
        CLASS_SUFFIX,
        vec![condition_binding()],
    );
}

pub(super) fn matches(payload: &SectionPayload) -> bool {
    crate::runtime::handler_matches(HANDLER_ID, &payload.candidate.registration_id)
}

pub(super) fn resolve(context: InvocationContext, payload: SectionPayload) -> HookOutput {
    // SecWhile switched to LoopSection and the `:do` tag in Skript 2.8.0;
    // 2.6.4 and 2.7.x still use Section plus the legacy ParseMark.
    let Some(modern) = crate::runtime::skript_at_least(2, 8) else {
        return unresolved_version(context, payload);
    };
    let Some(do_while) = resolve_do_while(
        modern,
        payload.candidate.mark,
        payload.candidate.tags.iter().any(|tag| tag.value == "do"),
    ) else {
        return reject_section(if modern {
            "2.16 SecWhile cannot carry a legacy ParseMark or inconsistent do tag"
        } else {
            "2.6 SecWhile uses a ParseMark rather than a do tag"
        });
    };

    if modern {
        if !payload.candidate.marks.is_empty() {
            return reject_section("2.16 SecWhile cannot carry a legacy ParseMark");
        }
    } else if payload
        .candidate
        .marks
        .iter()
        .any(|capture| capture.value != 1)
    {
        return reject_section("2.6 SecWhile has an invalid do-while ParseMark capture");
    }

    if let Err(reason) = condition_captures_are_parsed(&payload, 1) {
        return reject_section(reason);
    }

    // Before SecWhile extended LoopSection in 2.8, Skript's continue/exit
    // effects still special-cased SecWhile. Loop control is therefore valid
    // in every supported version; only the LoopSection iteration value is a
    // modern capability.
    let loop_capabilities = loop_capabilities(modern, payload.candidate.loop_section);
    let loop_iteration = loop_capabilities.iteration;
    let mut updates = vec![ContextUpdate {
        syntax_context: context.syntax_context,
        key: "core.section.while.do".to_owned(),
        value: Some(if do_while {
            b"true".to_vec()
        } else {
            b"false".to_vec()
        }),
    }];
    updates.push(ContextUpdate {
        syntax_context: context.syntax_context,
        key: "core.section.while.loop-context".to_owned(),
        value: Some(
            (if loop_iteration {
                "loop-section"
            } else {
                "loop-control-only"
            })
            .as_bytes()
            .to_vec(),
        ),
    });
    if !payload.candidate.loop_section {
        updates.push(super::increment_context_update(
            &payload.context,
            context.syntax_context,
            "core.section.loop-depth",
        ));
    }
    updates.push(ContextUpdate {
        syntax_context: context.syntax_context,
        key: "core.section.loop".to_owned(),
        value: Some(loop_capabilities.control.to_string().into_bytes()),
    });
    updates.push(ContextUpdate {
        syntax_context: context.syntax_context,
        key: "core.section.loop.iteration-value".to_owned(),
        value: Some(if loop_iteration {
            b"true".to_vec()
        } else {
            b"false".to_vec()
        }),
    });
    let metadata = [
        ("semantic-mode", "while".to_owned()),
        ("while-do", do_while.to_string()),
        (
            "while-loop-context",
            if loop_iteration {
                "loop-section"
            } else {
                "loop-control-only"
            }
            .to_owned(),
        ),
        ("while-loop-control", "true".to_owned()),
        ("while-loop-iteration-value", loop_iteration.to_string()),
        (
            "while-condition-parser",
            super::CONDITION_PARSER_ID.to_owned(),
        ),
    ];
    continue_with_section_context(&context, payload, metadata, updates)
}

/// Maps `SecWhile`'s version-specific optional `do` syntax.
///
/// Skript 2.6 declares `[(1¦do)] while <.+>` and reads `parseResult.mark`.
/// Modern Skript declares `[:do] while <.+>` and reads `parseResult.hasTag("do")`.
fn resolve_do_while(modern: bool, mark: i32, do_tag: bool) -> Option<bool> {
    if modern {
        (mark == 0).then_some(do_tag)
    } else if !do_tag && matches!(mark, 0 | 1) {
        Some(mark == 1)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LoopCapabilities {
    control: bool,
    iteration: bool,
}

fn loop_capabilities(modern: bool, candidate_loop_section: bool) -> LoopCapabilities {
    LoopCapabilities {
        control: true,
        iteration: modern && candidate_loop_section,
    }
}

fn unresolved_version(context: InvocationContext, payload: SectionPayload) -> HookOutput {
    let span = payload.candidate.span.clone();
    let mut output = continue_with_section_context(
        &context,
        payload,
        [
            ("semantic-mode", "while".to_owned()),
            ("semantic-state", "unresolved".to_owned()),
        ],
        Vec::new(),
    );
    output.effects.diagnostics.push(super::warning(
        "core.section.while.unresolved-version",
        "the Skript version is unavailable, so while syntax semantics were not selected",
        span,
    ));
    output
}

#[cfg(test)]
mod tests {
    use super::{LoopCapabilities, loop_capabilities, resolve_do_while};

    #[test]
    fn legacy_do_while_uses_mark_one() {
        assert_eq!(resolve_do_while(false, 0, false), Some(false));
        assert_eq!(resolve_do_while(false, 1, false), Some(true));
        assert_eq!(resolve_do_while(false, 1, true), None);
    }

    #[test]
    fn modern_do_while_uses_tag_and_rejects_marks() {
        assert_eq!(resolve_do_while(true, 0, true), Some(true));
        assert_eq!(resolve_do_while(true, 0, false), Some(false));
        assert_eq!(resolve_do_while(true, 1, true), None);
    }

    #[test]
    fn legacy_while_supports_loop_control_without_an_iteration_value() {
        assert_eq!(
            loop_capabilities(false, false),
            LoopCapabilities {
                control: true,
                iteration: false
            }
        );
        assert_eq!(
            loop_capabilities(true, true),
            LoopCapabilities {
                control: true,
                iteration: true
            }
        );
    }
}

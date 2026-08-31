use crate::nlaocs::skript_parser_addon::types::{EffectPayload, RegisteredSyntaxHandler};

const CLASS_SUFFIX: &str = ".EffReplace";
const HANDLER_ID: &str = "core.effect.eff-replace";
const STRING: &str = "java.lang.String";

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
    let modern = crate::runtime::skript_at_least(2, 10)
        .unwrap_or_else(|| candidate.pattern.contains("regex"));
    let Some(mode) = replace_mode(
        modern,
        candidate.pattern_index,
        candidate.mark,
        candidate.tags.iter().any(|tag| tag.value == "first"),
        candidate.tags.iter().any(|tag| tag.value == "case"),
    ) else {
        return Some(super::reject_with(
            "replace Effect has an unknown pattern index",
            "core.eff-replace.unknown-pattern",
            candidate.span.clone(),
        ));
    };
    super::annotate(
        &mut payload,
        "semantic-mode",
        if mode.string {
            "replace-text"
        } else {
            "replace-items"
        },
    );
    super::annotate(&mut payload, "replace-regex", bool_name(mode.regex));
    super::annotate(&mut payload, "replace-first", bool_name(mode.first));
    super::annotate(
        &mut payload,
        "replace-case-sensitive",
        bool_name(mode.case_sensitive),
    );
    if !mode.string {
        return Some(super::accept(payload));
    }

    let Some(haystack) = super::parsed_capture(&payload, mode.haystack_capture) else {
        let span = payload.candidate.as_ref()?.span.clone();
        return Some(unresolved(
            payload,
            "core.eff-replace.missing-haystack",
            "the replace target was not parsed",
            span,
        ));
    };
    let span = haystack.span.clone();
    match super::set_contract_with_converters_verdict(haystack, &[STRING]) {
        Ok(super::ContractVerdict::Accepted) => Some(super::accept(payload)),
        Ok(super::ContractVerdict::Rejected) => Some(super::reject_with(
            "the replace target cannot be changed to text",
            "core.eff-replace.immutable-text-target",
            span,
        )),
        Ok(super::ContractVerdict::Unresolved) => Some(unresolved(
            payload,
            "core.eff-replace.unresolved-change-contract",
            "the replace target does not expose enough change data to verify SET String",
            span,
        )),
        Err(reason) => Some(unresolved(
            payload,
            "core.eff-replace.change-contract-unavailable",
            &format!("the replace target's change contract could not be read: {reason}"),
            span,
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

fn bool_name(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReplaceMode {
    haystack_capture: u64,
    string: bool,
    regex: bool,
    first: bool,
    case_sensitive: bool,
}

fn replace_mode(
    modern: bool,
    pattern_index: u64,
    mark: i32,
    first_tag: bool,
    case_tag: bool,
) -> Option<ReplaceMode> {
    if pattern_index > 5 {
        return None;
    }
    Some(ReplaceMode {
        haystack_capture: 1 + pattern_index % 2,
        string: pattern_index < 4,
        regex: modern && matches!(pattern_index, 2 | 3),
        first: if modern {
            first_tag
        } else {
            matches!(pattern_index, 2 | 3)
        },
        case_sensitive: if modern { case_tag } else { mark == 1 },
    })
}

#[cfg(test)]
mod tests {
    use super::{ReplaceMode, replace_mode};

    #[test]
    fn keeps_legacy_first_and_modern_regex_patterns_distinct() {
        assert_eq!(
            replace_mode(false, 2, 1, false, false),
            Some(ReplaceMode {
                haystack_capture: 1,
                string: true,
                regex: false,
                first: true,
                case_sensitive: true,
            })
        );
        assert_eq!(
            replace_mode(true, 2, 0, false, false),
            Some(ReplaceMode {
                haystack_capture: 1,
                string: true,
                regex: true,
                first: false,
                case_sensitive: false,
            })
        );
        assert_eq!(replace_mode(true, 6, 0, false, false), None);
    }
}

use super::{
    context_value_update, continue_with_section_context, parsed_capture, register_handler,
    reject_section, warning,
};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionPossibleReturnTypesState, HookDecision, HookEffects, HookOutput,
    HookPayload, InvocationContext, MetadataEntry, ParseResultStatus, SectionBodyMode,
    SectionPayload, SectionRawNodeKind, SectionTiming,
};

const CLASS_SUFFIX: &str = ".SecFilter";
const HANDLER_ID: &str = "core.section.sec-filter";
const EXPRESSION_PARSER_ID: &str = "host.expression";
const SKRIPT_DEFINITION_PREFIX: &str = "section:skript:";
const LATEST_SUPPORTED_VERSION: (u64, u64) = (2, 16);

pub(super) fn register(
    handlers: &mut Vec<crate::nlaocs::skript_parser_addon::types::RegisteredSyntaxHandler>,
) {
    register_handler(handlers, HANDLER_ID, CLASS_SUFFIX, Vec::new());
}

pub(super) fn matches(payload: &SectionPayload) -> bool {
    crate::runtime::handler_matches(HANDLER_ID, &payload.candidate.registration_id)
        && is_skript_definition(&payload.candidate.definition_id)
}

pub(super) fn resolve(context: InvocationContext, mut payload: SectionPayload) -> HookOutput {
    match version_knowledge() {
        VersionKnowledge::Unknown => {
            return unresolved_version(
                payload,
                "core.sec-filter.unresolved-version",
                "the Skript version is unavailable, so filter semantics were not selected",
            );
        }
        VersionKnowledge::Future(major, minor) => {
            return unresolved_version(
                payload,
                "core.sec-filter.future-version",
                format!(
                    "Skript {major}.{minor} is newer than the supported semantic model, so filter semantics were not selected"
                ),
            );
        }
        VersionKnowledge::Known(version) if version < (2, 10) => {
            return reject_section("filter sections are not available before Skript 2.10");
        }
        VersionKnowledge::Known(_) => {}
    }

    if !matches!(payload.timing, SectionTiming::EnterChildren) {
        payload.candidate.body_mode = SectionBodyMode::Conditions;
        return continue_with_section_context(&context, payload, [], Vec::new());
    }

    let Some(source) = parsed_capture(&payload, 0, EXPRESSION_PARSER_ID).cloned() else {
        return unresolved_section(
            payload,
            "core.sec-filter.unresolved-source",
            "the filter source Expression was not provided by the host; list-variable validation is unresolved",
            None,
        );
    };
    if source.status == ParseResultStatus::Failed {
        return reject_section("filter Section source Expression failed to parse");
    }
    if source.status != ParseResultStatus::Success {
        return unresolved_section(
            payload,
            "core.sec-filter.unresolved-source",
            "the filter source Expression is only partially resolved; list-variable validation is incomplete",
            Some(source.span.clone()),
        );
    }
    let Some(source_summary) = source.summary.as_ref() else {
        return unresolved_section(
            payload,
            "core.sec-filter.unresolved-source-kind",
            "the filter source kind and multiplicity are unavailable; Skript list-variable validation is unresolved",
            Some(source.span.clone()),
        );
    };
    if source_summary.possible_return_types_state != ExpressionPossibleReturnTypesState::Complete {
        return unresolved_section(
            payload,
            "core.sec-filter.unresolved-input-types",
            "the filter source possible return types are incomplete; filter semantics were not selected",
            Some(source.span.clone()),
        );
    }
    let Some(return_type) = source_summary.return_type.clone() else {
        return unresolved_section(
            payload,
            "core.sec-filter.unresolved-input-type",
            "the filtered value type is unavailable; filter semantics were not selected",
            Some(source.span.clone()),
        );
    };
    if !source_summary.kind.eq_ignore_ascii_case("variable") {
        return reject_section("filter sections can only filter list variables");
    }
    match source_summary.multiplicity {
        Some(DynamicMultiplicity::Single) => {
            return reject_section("filter sections can only filter list variables");
        }
        Some(DynamicMultiplicity::Both) => {
            return unresolved_section(
                payload,
                "core.sec-filter.unresolved-multiplicity",
                "the filter source may be single at runtime; Skript list-variable validation cannot be proven statically",
                Some(source.span.clone()),
            );
        }
        Some(DynamicMultiplicity::Multiple) => {}
        None => {
            return unresolved_section(
                payload,
                "core.sec-filter.unresolved-multiplicity",
                "the filter source multiplicity is unavailable; Skript list-variable validation is unresolved",
                Some(source.span.clone()),
            );
        }
    }

    let meaningful_children = payload
        .raw_children
        .iter()
        .filter(|child| {
            !matches!(
                child.kind,
                SectionRawNodeKind::Blank | SectionRawNodeKind::Comment
            )
        })
        .collect::<Vec<_>>();
    if meaningful_children.is_empty() {
        return reject_section("filter sections must contain at least one condition");
    }
    if meaningful_children
        .iter()
        .any(|child| matches!(child.kind, SectionRawNodeKind::Section))
    {
        return reject_section("filter sections may not contain other sections");
    }
    if meaningful_children
        .iter()
        .any(|child| !matches!(child.kind, SectionRawNodeKind::Simple))
    {
        return reject_section("filter sections may only contain simple conditions");
    }

    let value_types = if source_summary.possible_return_types.is_empty() {
        return_type
    } else {
        source_summary.possible_return_types.join(";")
    };
    let filter_mode = if has_tag(&payload, "any") {
        "any"
    } else {
        "all"
    };
    let metadata = [
        ("semantic-mode", "filter-section".to_owned()),
        ("filter-mode", filter_mode.to_owned()),
        ("input-source.has-indices", "true".to_owned()),
    ];
    payload.candidate.body_mode = SectionBodyMode::Conditions;
    let updates = vec![
        context_value_update(&context, "core.input-source.available", "true"),
        context_value_update(&context, "core.input-source.has-indices", "true"),
        context_value_update(&context, "core.input-source.value-types", &value_types),
        context_value_update(&context, "core.section.filter", "true"),
        context_value_update(&context, "core.section.filter.mode", filter_mode),
    ];
    continue_with_section_context(&context, payload, metadata, updates)
}

fn has_tag(payload: &SectionPayload, value: &str) -> bool {
    payload
        .candidate
        .tags
        .iter()
        .any(|tag| tag.value.eq_ignore_ascii_case(value))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VersionKnowledge {
    Known((u64, u64)),
    Unknown,
    Future(u64, u64),
}

fn version_knowledge() -> VersionKnowledge {
    let Some(profile) = crate::runtime::current() else {
        return VersionKnowledge::Unknown;
    };
    let Some(version) = profile.skript_version.as_deref() else {
        return VersionKnowledge::Unknown;
    };
    let Some((major, minor)) = crate::runtime::parse_skript_version(version) else {
        return VersionKnowledge::Unknown;
    };
    if (major, minor) > LATEST_SUPPORTED_VERSION {
        VersionKnowledge::Future(major, minor)
    } else {
        VersionKnowledge::Known((major, minor))
    }
}

fn unresolved_version(
    payload: SectionPayload,
    code: &str,
    message: impl Into<String>,
) -> HookOutput {
    unresolved_section(payload, code, message, None)
}

fn unresolved_section(
    mut payload: SectionPayload,
    code: &str,
    message: impl Into<String>,
    span: Option<crate::nlaocs::skript_parser_addon::types::MappedSpan>,
) -> HookOutput {
    payload.candidate.metadata.push(MetadataEntry {
        key: "semantic-state".to_owned(),
        value: "unresolved".to_owned(),
        owner_component_id: None,
    });
    let diagnostic_span = span.unwrap_or_else(|| payload.candidate.span.clone());
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Section(payload)),
        effects: HookEffects {
            diagnostics: vec![warning(code, message, diagnostic_span)],
            context_updates: Vec::new(),
            parse_requests: Vec::new(),
            parse_results: Vec::new(),
        },
    }
}

fn is_skript_definition(definition_id: &str) -> bool {
    definition_id
        .strip_prefix(SKRIPT_DEFINITION_PREFIX)
        .is_some_and(|id| !id.is_empty())
}

#[cfg(test)]
fn version_knowledge_for(version: Option<&str>) -> VersionKnowledge {
    let Some(version) = version else {
        return VersionKnowledge::Unknown;
    };
    let Some((major, minor)) = crate::runtime::parse_skript_version(version) else {
        return VersionKnowledge::Unknown;
    };
    if (major, minor) > LATEST_SUPPORTED_VERSION {
        VersionKnowledge::Future(major, minor)
    } else {
        VersionKnowledge::Known((major, minor))
    }
}

#[cfg(test)]
fn has_filter_mode(any: bool) -> &'static str {
    if any { "any" } else { "all" }
}

#[cfg(test)]
mod tests {
    use super::{VersionKnowledge, version_knowledge_for};

    #[test]
    fn filter_without_any_tag_uses_skript_all_semantics() {
        assert_eq!(super::has_filter_mode(false), "all");
        assert_eq!(super::has_filter_mode(true), "any");
    }

    #[test]
    fn filter_version_boundary_is_explicit() {
        assert_eq!(
            version_knowledge_for(Some("2.9.5")),
            VersionKnowledge::Known((2, 9))
        );
        assert_eq!(
            version_knowledge_for(Some("2.17.0")),
            VersionKnowledge::Future(2, 17)
        );
        assert_eq!(version_knowledge_for(None), VersionKnowledge::Unknown);
    }
}

use super::{
    context_value_update, continue_with_section_context, parsed_capture, register_handler_targets,
    reject_section, warning,
};
use crate::nlaocs::skript_parser_addon::types::{
    DynamicMultiplicity, ExpressionPossibleReturnTypesState, HookDecision, HookEffects, HookOutput,
    HookPayload, InvocationContext, MetadataEntry, ParseResultStatus, ParseSummary,
    SectionBodyMode, SectionPayload,
};

const LOOP_CLASS_SUFFIX: &str = ".SecLoop";
const FOR_CLASS_SUFFIX: &str = ".SecFor";
const HANDLER_ID: &str = "core.section.sec-loop";
const EXPRESSION_PARSER_ID: &str = "host.expression";
const KEY_PROVIDER_CAPABILITY: &str = "expression.capability.key-provider";
const LOOP_PEEKING_CAPABILITY: &str = "expression.capability.loop-peeking";
const KEYED_ITERABLE_EXPRESSION: &str = "ch.njol.skript.lang.KeyedIterableExpression";
const SKRIPT_DEFINITION_PREFIX: &str = "section:skript:";
const LATEST_SUPPORTED_VERSION: (u64, u64) = (2, 16);

pub(super) fn register(
    handlers: &mut Vec<crate::nlaocs::skript_parser_addon::types::RegisteredSyntaxHandler>,
) {
    register_handler_targets(
        handlers,
        HANDLER_ID,
        &[LOOP_CLASS_SUFFIX, FOR_CLASS_SUFFIX],
        Vec::new(),
    );
}

pub(super) fn matches(payload: &SectionPayload) -> bool {
    crate::runtime::handler_matches(HANDLER_ID, &payload.candidate.registration_id)
        && is_skript_definition(&payload.candidate.definition_id)
}

pub(super) fn resolve(context: InvocationContext, payload: SectionPayload) -> HookOutput {
    let is_for = payload
        .candidate
        .element_class
        .as_deref()
        .is_some_and(|class| class.ends_with(FOR_CLASS_SUFFIX));
    match version_knowledge() {
        VersionKnowledge::Unknown => {
            return unresolved_version(
                payload,
                "core.sec-loop.unresolved-version",
                "the Skript version is unavailable, so loop semantics were not selected",
            );
        }
        VersionKnowledge::Future(major, minor) => {
            return unresolved_version(
                payload,
                "core.sec-loop.future-version",
                format!(
                    "Skript {major}.{minor} is newer than the supported semantic model, so loop semantics were not selected"
                ),
            );
        }
        VersionKnowledge::Known(version) => {
            if is_for && version < (2, 10) {
                return reject_section("for each loops are not available before Skript 2.10");
            }
            if is_for
                && for_loop_experiment_required(version)
                && !crate::experiments::enabled(&payload.context, "for loop")
            {
                return reject_section(
                    "for each loops require the `for loop` experiment before Skript 2.14",
                );
            }
        }
    }

    let source_index = if is_for {
        if payload.candidate.pattern_index == 2 {
            2
        } else {
            1
        }
    } else {
        0
    };
    let Some(source) = parsed_capture(&payload, source_index, EXPRESSION_PARSER_ID).cloned() else {
        return unresolved_section(
            payload,
            "core.sec-loop.unresolved-source",
            "the loop source Expression is unavailable; iterable and multiplicity checks are unresolved",
            None,
        );
    };
    if source.status == ParseResultStatus::Failed {
        return reject_section("loop source Expression failed to parse");
    }
    if source.status != ParseResultStatus::Success {
        return unresolved_section(
            payload,
            "core.sec-loop.unresolved-source",
            "the loop source Expression is only partially resolved; iterable validation is incomplete",
            Some(source.span.clone()),
        );
    }

    let Some(summary) = source.summary.as_ref() else {
        return unresolved_section(
            payload,
            "core.sec-loop.unresolved-source-summary",
            "the loop source return type and multiplicity are unavailable",
            Some(source.span.clone()),
        );
    };
    if summary.possible_return_types_state != ExpressionPossibleReturnTypesState::Complete {
        return unresolved_section(
            payload,
            "core.sec-loop.unresolved-source-types",
            "the loop source possible return types are incomplete; loop semantics were not selected",
            Some(source.span.clone()),
        );
    }
    if summary.return_type.is_none() {
        return unresolved_section(
            payload,
            "core.sec-loop.unresolved-source-type",
            "the loop source return type is unavailable; loop semantics were not selected",
            Some(source.span.clone()),
        );
    }
    let mut effective_type = summary.return_type.clone();
    let mut effective_types = if summary.possible_return_types.is_empty() {
        summary.return_type.clone().into_iter().collect()
    } else {
        summary.possible_return_types.clone()
    };
    let Some(mut effective_multiplicity) = summary.multiplicity else {
        return unresolved_section(
            payload,
            "core.sec-loop.unresolved-multiplicity",
            "the loop source multiplicity is unavailable; loopability cannot be verified",
            Some(source.span.clone()),
        );
    };
    let source_is_variable = summary.kind.eq_ignore_ascii_case("variable");
    let source_has_indices = key_provider_capability(summary);
    let source_supports_peeking = capability_value(&summary.metadata, LOOP_PEEKING_CAPABILITY)
        .or_else(|| supports_loop_peeking(summary));
    if !source_is_variable && let Some(return_type) = effective_type.clone() {
        match container_semantics(&return_type) {
            ContainerSemantics::Element(element_type) => {
                effective_type = Some(element_type.clone());
                effective_types = vec![element_type];
                effective_multiplicity = DynamicMultiplicity::Multiple;
            }
            ContainerSemantics::MissingAnnotation => {
                return reject_section(format!(
                    "{return_type} implements Container but is missing the required ContainerType annotation"
                ));
            }
            ContainerSemantics::Unresolved => {
                return unresolved_section(
                    payload,
                    "core.sec-loop.unresolved-container",
                    "the loop source may be a Container, but its ContainerType metadata is unavailable",
                    Some(source.span.clone()),
                );
            }
            ContainerSemantics::NotContainer => {}
        }
    }

    if effective_multiplicity == DynamicMultiplicity::Single {
        if !crate::experiments::enabled(&payload.context, "queues") {
            return reject_section(
                "you can only loop over expressions that return multiple values",
            );
        }
        match source_is_variable
            .then_some(true)
            .or_else(|| effective_type.as_deref().and_then(iterable_type))
        {
            Some(true) => {}
            Some(false) => {
                return reject_section("you can only loop over lists or Iterable values");
            }
            None => {
                return unresolved_section(
                    payload,
                    "core.sec-loop.unresolved-iterable",
                    "the loop source is single, but its Iterable relationship is unavailable",
                    Some(source.span.clone()),
                );
            }
        }
    }
    if effective_multiplicity == DynamicMultiplicity::Both {
        return unresolved_section(
            payload,
            "core.sec-loop.unresolved-multiplicity",
            "the loop source may be single at runtime; iterable validation is unresolved",
            Some(source.span.clone()),
        );
    }

    if is_for {
        let variable_indices: &[u64] = match payload.candidate.pattern_index {
            0 | 1 => &[0],
            2 => &[0, 1],
            _ => &[],
        };
        for index in variable_indices {
            let Some(variable) = parsed_capture(&payload, *index, EXPRESSION_PARSER_ID).cloned()
            else {
                return unresolved_section(
                    payload,
                    "core.sec-for.unresolved-variable",
                    "the `for` target is unavailable; Variable validation is unresolved",
                    None,
                );
            };
            if variable.status == ParseResultStatus::Failed {
                return reject_section("for targets must be variables");
            }
            if variable.status != ParseResultStatus::Success {
                return unresolved_section(
                    payload,
                    "core.sec-for.unresolved-variable",
                    "the `for` target is only partially resolved; Variable validation is unresolved",
                    Some(variable.span.clone()),
                );
            }
            let Some(summary) = variable.summary.as_ref() else {
                return unresolved_section(
                    payload,
                    "core.sec-for.unresolved-variable",
                    "the `for` target kind is unavailable; Variable validation is unresolved",
                    Some(variable.span.clone()),
                );
            };
            if !summary.kind.eq_ignore_ascii_case("variable") {
                return reject_section("for targets must be variables");
            }
        }
    }

    let Some(effective_type) = effective_type else {
        return unresolved_section(
            payload,
            "core.sec-loop.unresolved-source-type",
            "the loop source return type became unavailable during container resolution",
            Some(source.span.clone()),
        );
    };
    finish(
        context,
        payload,
        is_for,
        LoopSourceSemantics {
            return_type: effective_type,
            possible_return_types: effective_types,
            multiplicity: effective_multiplicity,
            expression: source.text,
            has_indices: source_has_indices,
            supports_peeking: source_supports_peeking,
        },
    )
}

struct LoopSourceSemantics {
    return_type: String,
    possible_return_types: Vec<String>,
    multiplicity: DynamicMultiplicity,
    expression: String,
    has_indices: Option<bool>,
    supports_peeking: Option<bool>,
}

fn finish(
    context: InvocationContext,
    mut payload: SectionPayload,
    is_for: bool,
    source: LoopSourceSemantics,
) -> HookOutput {
    let LoopSourceSemantics {
        return_type: source_type,
        possible_return_types: mut source_types,
        multiplicity: source_multiplicity,
        expression: source_expression,
        has_indices: source_has_indices,
        supports_peeking: source_supports_peeking,
    } = source;
    payload.candidate.body_mode = SectionBodyMode::Trigger;
    let previous_frames = payload
        .context
        .values
        .iter()
        .rfind(|entry| entry.key == crate::loop_context::CONTEXT_KEY)
        .map(|entry| entry.value.as_str());
    if source_types.is_empty() {
        source_types.push(source_type.clone());
    }
    let source_types = source_types.join(";");
    let multiplicity = multiplicity_name(source_multiplicity);
    let key_target = is_for && matches!(payload.candidate.pattern_index, 1 | 2);
    let frame = crate::loop_context::LoopFrame {
        source: source_expression.trim().to_owned(),
        return_type: source_type.clone(),
        possible_return_types: source_types
            .split(';')
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect(),
        keyed: source_has_indices,
        supports_peeking: source_supports_peeking,
    };
    let frames = loop_frames(previous_frames, Some(frame));
    let metadata = [
        (
            "semantic-mode",
            if is_for { "for" } else { "loop" }.to_owned(),
        ),
        ("loop-key-target", key_target.to_string()),
        (
            "loop-keyed",
            capability_state(source_has_indices).to_owned(),
        ),
        (
            "loop-peeking-state",
            capability_state(source_supports_peeking).to_owned(),
        ),
        ("loop-source-available", "true".to_owned()),
        ("loop-source-type", source_type.clone()),
        ("loop-source-multiplicity", multiplicity.to_owned()),
    ];
    let updates = vec![
        context_value_update(&context, "core.section.loop", "true"),
        context_value_update(
            &context,
            "core.loop.kind",
            if is_for { "for" } else { "loop" },
        ),
        context_value_update(
            &context,
            "core.loop.keyed",
            capability_state(source_has_indices),
        ),
        context_value_update(&context, "core.loop.expression.return-type", &source_type),
        context_value_update(&context, "core.loop.expression.multiplicity", multiplicity),
        context_value_update(&context, "core.loop.value-types", &source_types),
        context_value_update(&context, crate::loop_context::CONTEXT_KEY, &frames),
    ];
    continue_with_section_context(&context, payload, metadata, updates)
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
    version_knowledge_for(profile.skript_version.as_deref())
}

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

fn for_loop_experiment_required(version: (u64, u64)) -> bool {
    version.0 == 2 && (10..=13).contains(&version.1)
}

fn loop_frames(
    previous_frames: Option<&str>,
    frame: Option<crate::loop_context::LoopFrame>,
) -> String {
    frame.map_or_else(
        || previous_frames.unwrap_or_default().to_owned(),
        |frame| crate::loop_context::push(previous_frames, frame),
    )
}

fn capability_value(metadata: &[MetadataEntry], key: &str) -> Option<bool> {
    metadata
        .iter()
        .find(|entry| entry.key.ends_with(key))
        .and_then(|entry| match entry.value.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        })
}

fn key_provider_capability(summary: &ParseSummary) -> Option<bool> {
    if let Some(value) = capability_value(&summary.metadata, KEY_PROVIDER_CAPABILITY) {
        return Some(value);
    }
    if matches!(
        summary.kind.as_str(),
        "expression-list" | "literal" | "arithmetic"
    ) {
        return Some(false);
    }
    let element_class = summary.element_class.as_deref()?;
    match crate::catalog::is_class_assignable(element_class, KEYED_ITERABLE_EXPRESSION) {
        Ok(crate::catalog::TypeRelation::Incompatible) => Some(false),
        Ok(crate::catalog::TypeRelation::Compatible | crate::catalog::TypeRelation::Unknown)
        | Err(_) => None,
    }
}

fn capability_state(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "unknown",
    }
}

fn supports_loop_peeking(summary: &ParseSummary) -> Option<bool> {
    match summary.kind.as_str() {
        "variable" | "arithmetic" => Some(true),
        "literal" | "function" | "expression-list" => Some(false),
        "registered-expression" => summary.element_class.as_deref().and_then(|class_name| {
            match crate::catalog::is_class_assignable(
                class_name,
                "ch.njol.skript.lang.util.SimpleExpression",
            ) {
                Ok(crate::catalog::TypeRelation::Compatible) => Some(true),
                Ok(crate::catalog::TypeRelation::Incompatible) => Some(false),
                Ok(crate::catalog::TypeRelation::Unknown) | Err(_) => None,
            }
        }),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ContainerSemantics {
    NotContainer,
    Element(String),
    MissingAnnotation,
    Unresolved,
}

fn container_semantics(class_name: &str) -> ContainerSemantics {
    let Ok(relation) =
        crate::catalog::is_class_assignable(class_name, "ch.njol.skript.util.Container")
    else {
        return ContainerSemantics::Unresolved;
    };
    let element_type = match crate::catalog::container_element_type(class_name) {
        Ok(value) => value,
        Err(_) => return ContainerSemantics::Unresolved,
    };
    classify_container(
        relation,
        element_type,
        crate::runtime::snapshot_schema_at_least(5) == Some(true),
    )
}

fn classify_container(
    relation: crate::catalog::TypeRelation,
    element_type: Option<String>,
    metadata_authoritative: bool,
) -> ContainerSemantics {
    match relation {
        crate::catalog::TypeRelation::Compatible => match element_type {
            Some(element) => ContainerSemantics::Element(element),
            None if metadata_authoritative => ContainerSemantics::MissingAnnotation,
            None => ContainerSemantics::Unresolved,
        },
        crate::catalog::TypeRelation::Incompatible => ContainerSemantics::NotContainer,
        crate::catalog::TypeRelation::Unknown => ContainerSemantics::Unresolved,
    }
}

fn iterable_type(class_name: &str) -> Option<bool> {
    match crate::catalog::is_class_assignable(class_name, "java.lang.Iterable") {
        Ok(crate::catalog::TypeRelation::Compatible) => Some(true),
        Ok(crate::catalog::TypeRelation::Incompatible) => Some(false),
        Ok(crate::catalog::TypeRelation::Unknown) | Err(_) => None,
    }
}

fn multiplicity_name(value: DynamicMultiplicity) -> &'static str {
    match value {
        DynamicMultiplicity::Single => "single",
        DynamicMultiplicity::Multiple => "multiple",
        DynamicMultiplicity::Both => "both",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ContainerSemantics, KEY_PROVIDER_CAPABILITY, capability_state, capability_value,
        classify_container, for_loop_experiment_required, iterable_type, loop_frames,
        multiplicity_name, supports_loop_peeking, version_knowledge_for,
    };
    use crate::catalog::TypeRelation;
    use crate::nlaocs::skript_parser_addon::types::{
        DynamicMultiplicity, ExpressionPossibleReturnTypesState, MetadataEntry, ParseSummary,
    };

    fn summary(kind: &str) -> ParseSummary {
        ParseSummary {
            kind: kind.to_owned(),
            definition_id: None,
            registration_id: None,
            element_class: None,
            pattern_index: None,
            return_type: None,
            possible_return_types: Vec::new(),
            possible_return_types_state: ExpressionPossibleReturnTypesState::Unresolved,
            multiplicity: None,
            public_data: Vec::new(),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn only_known_iterables_are_accepted_as_single_loop_sources() {
        assert_eq!(iterable_type("java.lang.Iterable"), Some(true));
        assert_eq!(iterable_type("java.lang.Object"), None);
        assert_eq!(iterable_type("java.lang.Long"), None);
    }

    #[test]
    fn unresolved_multiplicity_is_distinct_from_both() {
        assert_eq!(multiplicity_name(DynamicMultiplicity::Both), "both");
    }

    #[test]
    fn container_annotations_replace_the_outer_loop_type() {
        assert_eq!(
            classify_container(
                TypeRelation::Compatible,
                Some("org.bukkit.inventory.ItemStack".to_owned()),
                true,
            ),
            ContainerSemantics::Element("org.bukkit.inventory.ItemStack".to_owned())
        );
        assert_eq!(
            classify_container(TypeRelation::Compatible, None, true),
            ContainerSemantics::MissingAnnotation
        );
        assert_eq!(
            classify_container(TypeRelation::Compatible, None, false),
            ContainerSemantics::Unresolved
        );
    }

    #[test]
    fn missing_loop_source_keeps_existing_frames() {
        let previous = r#"[{"source":"all players","return_type":"org.bukkit.entity.Player","possible_return_types":["org.bukkit.entity.Player"],"keyed":false,"supports_peeking":true}]"#;
        assert_eq!(loop_frames(Some(previous), None), previous);
        assert_eq!(loop_frames(None, None), "");
    }

    #[test]
    fn capability_unknown_is_distinct_from_false() {
        assert_eq!(capability_value(&[], KEY_PROVIDER_CAPABILITY), None);
        assert_eq!(capability_state(None), "unknown");
        assert_eq!(capability_state(Some(false)), "false");
        assert_eq!(
            capability_value(
                &[MetadataEntry {
                    key: KEY_PROVIDER_CAPABILITY.to_owned(),
                    value: "false".to_owned(),
                    owner_component_id: None,
                }],
                KEY_PROVIDER_CAPABILITY,
            ),
            Some(false)
        );
    }

    #[test]
    fn expression_lists_do_not_claim_loop_peeking_support() {
        assert_eq!(
            supports_loop_peeking(&summary("expression-list")),
            Some(false)
        );
        assert_eq!(supports_loop_peeking(&summary("literal")), Some(false));
        assert_eq!(supports_loop_peeking(&summary("unresolved")), None);
    }

    #[test]
    fn for_loop_experiment_is_required_until_the_stable_release() {
        assert!(for_loop_experiment_required((2, 10)));
        assert!(for_loop_experiment_required((2, 13)));
        assert!(!for_loop_experiment_required((2, 9)));
        assert!(!for_loop_experiment_required((2, 14)));
        assert!(!for_loop_experiment_required((3, 0)));
    }

    #[test]
    fn only_nonempty_skript_definition_ids_are_owned() {
        assert!(super::is_skript_definition("section:skript:definition"));
        assert!(!super::is_skript_definition("section:skript:"));
        assert!(!super::is_skript_definition(
            "section:skriptdummyaddon:definition"
        ));
    }

    #[test]
    fn loop_version_knowledge_is_conservative_at_the_supported_boundary() {
        assert_eq!(
            version_knowledge_for(Some("2.9.5")),
            super::VersionKnowledge::Known((2, 9))
        );
        assert_eq!(
            version_knowledge_for(Some("2.17.0")),
            super::VersionKnowledge::Future(2, 17)
        );
        assert_eq!(
            version_knowledge_for(None),
            super::VersionKnowledge::Unknown
        );
    }
}

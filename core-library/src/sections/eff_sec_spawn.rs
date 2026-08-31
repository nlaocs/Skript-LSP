use super::{
    context_value_update, continue_with_section_context, register_handler, reject_section, warning,
};
use crate::nlaocs::skript_parser_addon::types::{
    HookDecision, HookEffects, HookOutput, HookPayload, InvocationContext, MetadataEntry,
    SectionBodyMode, SectionPayload, SectionTiming,
};

const CLASS_SUFFIX: &str = ".EffSecSpawn";
const HANDLER_ID: &str = "core.section.eff-sec-spawn";
const EVENT_CLASS: &str = "ch.njol.skript.sections.EffSecSpawn$SpawnEvent";
const ENTITY_SNAPSHOT_CLASS: &str = "org.bukkit.entity.EntitySnapshot";
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
    let version = match version_knowledge() {
        VersionKnowledge::Unknown => {
            return unresolved_version(
                payload,
                "core.eff-sec-spawn.unresolved-version",
                "the Skript version is unavailable, so spawn Section semantics were not selected",
            );
        }
        VersionKnowledge::Future(major, minor) => {
            return unresolved_version(
                payload,
                "core.eff-sec-spawn.future-version",
                format!(
                    "Skript {major}.{minor} is newer than the supported semantic model, so spawn Section semantics were not selected"
                ),
            );
        }
        VersionKnowledge::Known(version) => version,
    };
    if version < ParsedVersion(2, 6, 1) {
        return reject_section("spawn Sections are not available before Skript 2.6.1");
    }
    payload.candidate.body_mode = SectionBodyMode::Trigger;
    let entering = matches!(payload.timing, SectionTiming::EnterChildren);
    let dropped_items = dropped_items_available(version);
    let Some(entity_snapshots) = entity_snapshots_available(version) else {
        return unresolved_section(
            payload,
            "core.eff-sec-spawn.unresolved-entity-snapshots",
            "the host class catalog cannot determine whether Bukkit EntitySnapshot is available",
            None,
        );
    };
    let metadata = [
        ("semantic-mode", "effect-section-spawn".to_owned()),
        ("section-event-class", EVENT_CLASS.to_owned()),
        (
            "section-registration-version",
            registration_version().to_owned(),
        ),
        ("section-dropped-items", dropped_items.to_string()),
        ("section-entity-snapshots", entity_snapshots.to_string()),
    ];
    let updates = if entering {
        vec![
            context_value_update(&context, "parser.event-classes", EVENT_CLASS),
            context_value_update(&context, "parser.event-name", "spawn"),
            context_value_update(&context, "parser.delay-state", "false"),
            context_value_update(&context, "core.section.effect-section", "true"),
            context_value_update(&context, "core.section.event-class", EVENT_CLASS),
            context_value_update(
                &context,
                "core.section.spawn.dropped-items",
                &dropped_items.to_string(),
            ),
            context_value_update(
                &context,
                "core.section.spawn.entity-snapshots",
                &entity_snapshots.to_string(),
            ),
        ]
    } else {
        Vec::new()
    };
    continue_with_section_context(&context, payload, metadata, updates)
}

fn registration_version() -> &'static str {
    "2.6.1"
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ParsedVersion(u64, u64, u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VersionKnowledge {
    Known(ParsedVersion),
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
    let patch = version
        .split(|character: char| !character.is_ascii_digit())
        .filter(|component| !component.is_empty())
        .nth(2)
        .and_then(|component| component.parse::<u64>().ok())
        .unwrap_or(0);
    if (major, minor) > LATEST_SUPPORTED_VERSION {
        VersionKnowledge::Future(major, minor)
    } else {
        VersionKnowledge::Known(ParsedVersion(major, minor, patch))
    }
}

fn entity_snapshots_available(version: ParsedVersion) -> Option<bool> {
    if version < ParsedVersion(2, 10, 0) {
        return Some(false);
    }
    crate::catalog::class_known(ENTITY_SNAPSHOT_CLASS).ok()
}

fn dropped_items_available(version: ParsedVersion) -> bool {
    version >= ParsedVersion(2, 8, 6)
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
    let patch = version
        .split(|character: char| !character.is_ascii_digit())
        .filter(|component| !component.is_empty())
        .nth(2)
        .and_then(|component| component.parse::<u64>().ok())
        .unwrap_or(0);
    if (major, minor) > LATEST_SUPPORTED_VERSION {
        VersionKnowledge::Future(major, minor)
    } else {
        VersionKnowledge::Known(ParsedVersion(major, minor, patch))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ParsedVersion, VersionKnowledge, dropped_items_available, entity_snapshots_available,
        registration_version, version_knowledge_for,
    };

    #[test]
    fn spawn_section_preserves_each_upstream_feature_boundary() {
        assert_eq!(registration_version(), "2.6.1");
        assert_eq!(
            version_knowledge_for(Some("2.6.0")),
            VersionKnowledge::Known(ParsedVersion(2, 6, 0))
        );
        assert_eq!(
            version_knowledge_for(Some("2.8.6")),
            VersionKnowledge::Known(ParsedVersion(2, 8, 6))
        );
        assert_eq!(
            version_knowledge_for(Some("2.17.0")),
            VersionKnowledge::Future(2, 17)
        );
        assert_eq!(version_knowledge_for(None), VersionKnowledge::Unknown);
        assert!(!dropped_items_available(ParsedVersion(2, 8, 5)));
        assert!(dropped_items_available(ParsedVersion(2, 8, 6)));
        assert!(!entity_snapshots_available(ParsedVersion(2, 9, 9)).unwrap());
    }
}

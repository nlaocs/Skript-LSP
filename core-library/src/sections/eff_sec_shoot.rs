use super::{context_value_update, continue_with_section_context, register_handler, warning};
use crate::nlaocs::skript_parser_addon::types::{
    HookDecision, HookEffects, HookOutput, HookPayload, InvocationContext, MetadataEntry,
    SectionBodyMode, SectionPayload, SectionTiming,
};

const CLASS_SUFFIX: &str = ".EffSecShoot";
const HANDLER_ID: &str = "core.section.eff-sec-shoot";
const EVENT_CLASS: &str = "ch.njol.skript.sections.EffSecShoot$ShootEvent";
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
                "core.eff-sec-shoot.unresolved-version",
                "the Skript version is unavailable, so shoot Section semantics were not selected",
            );
        }
        VersionKnowledge::Future(major, minor) => {
            return unresolved_version(
                payload,
                "core.eff-sec-shoot.future-version",
                format!(
                    "Skript {major}.{minor} is newer than the supported semantic model, so shoot Section semantics were not selected"
                ),
            );
        }
        VersionKnowledge::Known(version) if version < (2, 10) => {
            return super::reject_section("shoot sections are not available before Skript 2.10");
        }
        VersionKnowledge::Known(_) => {}
    }

    payload.candidate.body_mode = SectionBodyMode::Trigger;
    let entering = matches!(payload.timing, SectionTiming::EnterChildren);
    let metadata = [
        ("semantic-mode", "effect-section-shoot".to_owned()),
        ("section-event-class", EVENT_CLASS.to_owned()),
        (
            "section-registration-version",
            registration_version().to_owned(),
        ),
    ];
    let updates = if entering {
        vec![
            context_value_update(&context, "parser.event-classes", EVENT_CLASS),
            context_value_update(&context, "parser.event-name", "shoot"),
            context_value_update(&context, "parser.delay-state", "false"),
            context_value_update(&context, "core.section.effect-section", "true"),
            context_value_update(&context, "core.section.event-class", EVENT_CLASS),
        ]
    } else {
        Vec::new()
    };
    continue_with_section_context(&context, payload, metadata, updates)
}

fn registration_version() -> &'static str {
    "2.10"
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
    let mut payload = payload;
    payload.candidate.metadata.push(MetadataEntry {
        key: "semantic-state".to_owned(),
        value: "unresolved".to_owned(),
        owner_component_id: None,
    });
    let span = payload.candidate.span.clone();
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Section(payload)),
        effects: HookEffects {
            diagnostics: vec![warning(code, message, span)],
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
mod tests {
    use super::{VersionKnowledge, registration_version, version_knowledge_for};

    #[test]
    fn shoot_section_has_the_upstream_introduction_boundary() {
        assert_eq!(registration_version(), "2.10");
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

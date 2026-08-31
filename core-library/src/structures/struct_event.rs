use super::register_handler;
use crate::nlaocs::skript_parser_addon::types::{
    CaptureParserBinding, ContextUpdate, HookDecision, HookEffects, HookOutput, HookPayload,
    InvocationContext, MetadataEntry, ParseResultStatus, RegisteredSyntaxHandler,
    StructureBodyMode, StructurePayload, StructureTiming,
};

const CLASS_SUFFIX: &str = ".StructEvent";
const HANDLER_ID: &str = "core.structure.struct-event";
const EVENT_PRIORITIES: [&str; 6] = ["lowest", "low", "normal", "high", "highest", "monitor"];
const INTRODUCED_IN: (u64, u64) = (2, 8);
const FIRST_UNSUPPORTED_MINOR: (u64, u64) = (2, 17);

pub(super) fn register(handlers: &mut Vec<RegisteredSyntaxHandler>) {
    register_handler(
        handlers,
        HANDLER_ID,
        CLASS_SUFFIX,
        vec![CaptureParserBinding {
            capture_index: 0,
            parser_id: "host.event".to_owned(),
            required: true,
            options: Vec::new(),
        }],
    );
}

pub(super) fn matches(payload: &StructurePayload) -> bool {
    payload.candidate.handler.as_deref() == Some(HANDLER_ID)
        || crate::runtime::handler_matches(HANDLER_ID, &payload.candidate.registration_id)
}

pub(super) fn resolve(context: InvocationContext, mut payload: StructurePayload) -> HookOutput {
    if !matches!(payload.timing, StructureTiming::EnterBody) {
        return super::continue_with_mode(
            &context,
            payload,
            StructureBodyMode::Trigger,
            "event-structure",
            "core.structure.event",
        );
    }
    match version_support(INTRODUCED_IN) {
        VersionSupport::TooOld => {
            return super::reject_structure("StructEvent is not available before Skript 2.8");
        }
        VersionSupport::Unresolved => {
            return unresolved_structure(
                payload,
                "core.struct-event.unresolved-version",
                "Skript version is missing or newer than the supported 2.16 range; StructEvent semantics are unresolved",
            );
        }
        VersionSupport::Supported => {}
    }
    let Some(event) = payload.candidate.parsed_captures.iter().find(|capture| {
        capture.capture_index == 0
            && capture.parser_id == "host.event"
            && capture.status == ParseResultStatus::Success
    }) else {
        return unresolved_structure(
            payload,
            "core.struct-event.unresolved-event-capture",
            "the Event capture is missing or incomplete; StructEvent semantics are unresolved",
        );
    };
    let reference_classes = match summary_metadata(event, "parser.event.reference-classes") {
        MetadataLookup::Value(metadata) if !metadata.value.trim().is_empty() => {
            Some(metadata.value)
        }
        MetadataLookup::Missing | MetadataLookup::Value(_) => None,
        MetadataLookup::Conflict => {
            return unresolved_structure(
                payload,
                "core.struct-event.conflicting-metadata",
                "the Event capture supplied conflicting reference-event metadata; StructEvent semantics are unresolved",
            );
        }
    };
    if reference_classes.is_none() {
        return unresolved_structure(
            payload,
            "core.struct-event.unresolved-reference-events",
            "the Event source does not expose the Event classes available in its body",
        );
    }
    let cancellable_metadata = summary_metadata(event, "parser.event.cancellable");
    let cancellable = match cancellable_metadata {
        MetadataLookup::Value(metadata) if metadata.value == "true" => Some(true),
        MetadataLookup::Value(metadata) if metadata.value == "false" => Some(false),
        MetadataLookup::Value(_) => {
            return crate::reject("StructEvent capture has an invalid cancellable state");
        }
        MetadataLookup::Missing => None,
        MetadataLookup::Conflict => {
            return unresolved_structure(
                payload,
                "core.struct-event.conflicting-metadata",
                "the Event capture supplied conflicting cancellable metadata; StructEvent semantics are unresolved",
            );
        }
    };
    let priority_metadata = summary_metadata(event, "parser.event.priority-supported");
    let priority_supported = match priority_metadata {
        MetadataLookup::Value(metadata) if metadata.value == "true" => Some(true),
        MetadataLookup::Value(metadata) if metadata.value == "false" => Some(false),
        MetadataLookup::Value(_) => {
            return crate::reject("StructEvent capture has an invalid priority-supported state");
        }
        MetadataLookup::Missing => None,
        MetadataLookup::Conflict => {
            return unresolved_structure(
                payload,
                "core.struct-event.conflicting-metadata",
                "the Event capture supplied conflicting priority metadata; StructEvent semantics are unresolved",
            );
        }
    };
    let (listening_behavior, priority) = match event_options(&payload) {
        Ok(options) => options,
        Err(reason) => return crate::reject(&reason),
    };
    if !listening_behavior.is_empty() && cancellable == Some(false) {
        return crate::reject(
            "this Event is not cancellable, so it cannot select a cancellation listening behavior",
        );
    }
    if !priority.is_empty() && priority_supported == Some(false) {
        return crate::reject("this Event does not support a custom Event priority");
    }
    if !listening_behavior.is_empty() && cancellable.is_none() {
        return unresolved_structure(
            payload,
            "core.struct-event.unresolved-cancellable-state",
            "the Event source does not expose whether cancellation filters are supported",
        );
    }
    if !priority.is_empty() && priority_supported.is_none() {
        return unresolved_structure(
            payload,
            "core.struct-event.unresolved-priority-support",
            "the Event source does not expose whether custom priorities are supported",
        );
    }
    payload.candidate.body_mode = StructureBodyMode::Trigger;
    push_metadata(&mut payload, "semantic-mode", "event-structure");
    push_metadata(&mut payload, "event-listening-behavior", listening_behavior);
    push_metadata(&mut payload, "event-priority", priority);
    let cancellable_state =
        cancellable.map_or("unresolved", |value| if value { "true" } else { "false" });
    push_metadata(&mut payload, "event-cancellable", cancellable_state);
    let mut context_updates = vec![
        ContextUpdate {
            syntax_context: context.syntax_context,
            key: "core.structure.event".to_owned(),
            value: Some(b"true".to_vec()),
        },
        ContextUpdate {
            syntax_context: context.syntax_context,
            key: "core.structure.event.listening-behavior".to_owned(),
            value: Some(listening_behavior.as_bytes().to_vec()),
        },
        ContextUpdate {
            syntax_context: context.syntax_context,
            key: "core.structure.event.priority".to_owned(),
            value: Some(priority.as_bytes().to_vec()),
        },
        ContextUpdate {
            syntax_context: context.syntax_context,
            key: "core.structure.event.cancellable".to_owned(),
            value: Some(cancellable_state.as_bytes().to_vec()),
        },
        ContextUpdate {
            syntax_context: context.syntax_context,
            key: "parser.delay-state".to_owned(),
            value: Some(b"false".to_vec()),
        },
    ];
    if let Some(reference_classes) = reference_classes.as_deref() {
        context_updates.push(ContextUpdate {
            syntax_context: context.syntax_context,
            key: "parser.event-classes".to_owned(),
            value: Some(reference_classes.as_bytes().to_vec()),
        });
    }
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Structure(payload)),
        effects: HookEffects {
            diagnostics: Vec::new(),
            context_updates,
            parse_requests: Vec::new(),
            parse_results: Vec::new(),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MetadataLookup {
    Missing,
    Value(EventMetadata),
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EventMetadata {
    value: String,
    /// Preserve every trusted declaration that agreed on the value. The host
    /// separates an addon's namespace into `owner_component_id`, so selecting
    /// the first matching entry would otherwise make the result order-dependent.
    owner_component_ids: Vec<Option<String>>,
}

fn summary_metadata(
    capture: &crate::nlaocs::skript_parser_addon::types::ParsedCapture,
    key: &str,
) -> MetadataLookup {
    let Some(summary) = capture.summary.as_ref() else {
        return MetadataLookup::Missing;
    };
    metadata_lookup(&summary.metadata, key)
}

fn metadata_lookup(entries: &[MetadataEntry], key: &str) -> MetadataLookup {
    let mut value = None;
    let mut owners = Vec::new();
    for entry in entries.iter().filter(|entry| entry.key == key) {
        // `parser.event.*` is a deliberately small cross-component contract.
        // Qualified addon metadata is accepted, but conflicting owners must
        // not silently choose whichever component happened to run last.
        match value.as_deref() {
            None => value = Some(entry.value.clone()),
            Some(previous) if previous == entry.value => {}
            Some(_) => return MetadataLookup::Conflict,
        }
        if !owners.contains(&entry.owner_component_id) {
            owners.push(entry.owner_component_id.clone());
        }
    }
    value.map_or(MetadataLookup::Missing, |value| {
        MetadataLookup::Value(EventMetadata {
            value,
            owner_component_ids: owners,
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VersionSupport {
    Supported,
    TooOld,
    Unresolved,
}

fn version_support(introduced_in: (u64, u64)) -> VersionSupport {
    let version = crate::runtime::current().and_then(|profile| {
        profile
            .skript_version
            .and_then(|version| crate::runtime::parse_skript_version(&version))
    });
    version_support_for(version, introduced_in)
}

fn version_support_for(version: Option<(u64, u64)>, introduced_in: (u64, u64)) -> VersionSupport {
    match version {
        Some(version) if version < introduced_in => VersionSupport::TooOld,
        Some(version) if version >= FIRST_UNSUPPORTED_MINOR => VersionSupport::Unresolved,
        Some(_) => VersionSupport::Supported,
        None => VersionSupport::Unresolved,
    }
}

fn unresolved_structure(payload: StructurePayload, code: &str, message: &str) -> HookOutput {
    let span = payload.candidate.span.clone();
    super::continue_unresolved(payload, vec![super::structure_warning(code, message, span)])
}

/// Mirrors `StructEvent.EventData`: the modifier and priority are scoped to the
/// selected Event while its body is parsed. An empty value means Skript's
/// registration default, not an invented CoreLibrary default.
fn event_options(payload: &StructurePayload) -> Result<(&'static str, &'static str), String> {
    let tags = payload
        .candidate
        .tags
        .iter()
        .map(|tag| tag.value.as_str())
        .collect::<Vec<_>>();
    event_options_from_tags(&tags)
}

fn event_options_from_tags(tags: &[&str]) -> Result<(&'static str, &'static str), String> {
    let behavior = ["uncancelled", "cancelled", "any"]
        .into_iter()
        .filter(|value| has_tag(tags, value))
        .collect::<Vec<_>>();
    if behavior.len() > 1 {
        return Err("StructEvent selected more than one listening behavior".to_owned());
    }

    let priorities = EVENT_PRIORITIES
        .into_iter()
        .filter(|value| has_tag(tags, value))
        .collect::<Vec<_>>();
    if priorities.len() > 1 {
        return Err("StructEvent selected more than one Event priority".to_owned());
    }
    let has_priority = has_tag(tags, "priority");
    if has_priority != !priorities.is_empty() {
        return Err("StructEvent has an incomplete Event priority selection".to_owned());
    }

    Ok((
        behavior.first().copied().unwrap_or(""),
        priorities.first().copied().unwrap_or(""),
    ))
}

fn has_tag(tags: &[&str], value: &str) -> bool {
    tags.iter().any(|tag| tag.eq_ignore_ascii_case(value))
}

fn push_metadata(payload: &mut StructurePayload, key: &str, value: &str) {
    payload
        .candidate
        .metadata
        .push(crate::nlaocs::skript_parser_addon::types::MetadataEntry {
            key: key.to_owned(),
            value: value.to_owned(),
            owner_component_id: None,
        });
}

#[cfg(test)]
mod tests {
    use super::{
        EventMetadata, FIRST_UNSUPPORTED_MINOR, MetadataLookup, VersionSupport,
        event_options_from_tags, metadata_lookup, version_support_for,
    };
    use crate::nlaocs::skript_parser_addon::types::MetadataEntry;

    #[test]
    fn event_options_preserve_skript_defaults_and_explicit_values() {
        assert_eq!(event_options_from_tags(&[]), Ok(("", "")));
        assert_eq!(
            event_options_from_tags(&["cancelled", "priority", "highest"]),
            Ok(("cancelled", "highest"))
        );
    }

    #[test]
    fn event_options_reject_inconsistent_parse_data() {
        assert!(event_options_from_tags(&["cancelled", "any"]).is_err());
        assert!(event_options_from_tags(&["priority"]).is_err());
        assert!(event_options_from_tags(&["monitor"]).is_err());
    }

    #[test]
    fn event_metadata_accepts_and_preserves_qualified_addon_values() {
        let entries = vec![
            MetadataEntry {
                key: "parser.event.cancellable".to_owned(),
                value: "true".to_owned(),
                owner_component_id: None,
            },
            MetadataEntry {
                key: "parser.event.cancellable".to_owned(),
                value: "true".to_owned(),
                owner_component_id: Some("fixture.addon".to_owned()),
            },
        ];
        assert_eq!(
            metadata_lookup(&entries, "parser.event.cancellable"),
            MetadataLookup::Value(EventMetadata {
                value: "true".to_owned(),
                owner_component_ids: vec![None, Some("fixture.addon".to_owned())],
            })
        );
    }

    #[test]
    fn event_metadata_does_not_choose_between_conflicting_owners() {
        let entries = vec![
            MetadataEntry {
                key: "parser.event.cancellable".to_owned(),
                value: "true".to_owned(),
                owner_component_id: Some("first.addon".to_owned()),
            },
            MetadataEntry {
                key: "parser.event.cancellable".to_owned(),
                value: "false".to_owned(),
                owner_component_id: Some("second.addon".to_owned()),
            },
        ];
        assert_eq!(
            metadata_lookup(&entries, "parser.event.cancellable"),
            MetadataLookup::Conflict
        );
    }

    #[test]
    fn event_uses_the_known_2_8_boundary() {
        assert_eq!(
            version_support_for(Some((2, 7)), (2, 8)),
            VersionSupport::TooOld
        );
        assert_eq!(
            version_support_for(Some((2, 8)), (2, 8)),
            VersionSupport::Supported
        );
        assert_eq!(
            version_support_for(Some((2, 16)), (2, 8)),
            VersionSupport::Supported
        );
        assert_eq!(
            version_support_for(Some(FIRST_UNSUPPORTED_MINOR), (2, 8)),
            VersionSupport::Unresolved
        );
        assert_eq!(
            version_support_for(None, (2, 8)),
            VersionSupport::Unresolved
        );
    }
}

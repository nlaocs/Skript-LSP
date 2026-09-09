//! Synthetic Structure registrations for releases that predate StructureInfo.

use crate::nlaocs::skript_parser_addon::{
    dynamic_syntax_registry,
    types::{
        DynamicSyntaxDefinition, DynamicSyntaxId, DynamicSyntaxReference, RegisteredHandlerBinding,
        RuntimeSnapshotCapabilities, StructureBodyMode, StructureEntryData, StructureEntryKind,
        StructureEntryValidator, StructureNodeType, SyntaxKind,
    },
};

const EVENT_ID: &str = "legacy-struct-event";
const COMMAND_ID: &str = "legacy-struct-command";
const FUNCTION_ID: &str = "legacy-struct-function";

pub(super) fn register_missing(
    skript_version: &str,
    snapshot_capabilities: Option<&RuntimeSnapshotCapabilities>,
    bindings: &[RegisteredHandlerBinding],
) -> Result<(), String> {
    let version = parse_version(skript_version)
        .ok_or_else(|| "CoreLibrary could not parse the Skript version".to_owned())?;

    for definition in definitions(version, snapshot_capabilities, bindings)? {
        dynamic_syntax_registry::register(&definition).map_err(|error| error.message)?;
    }
    Ok(())
}

fn definitions(
    version: (u64, u64, u64),
    snapshot_capabilities: Option<&RuntimeSnapshotCapabilities>,
    bindings: &[RegisteredHandlerBinding],
) -> Result<Vec<DynamicSyntaxDefinition>, String> {
    let mut definitions = Vec::new();
    let command_missing = structure_is_missing(
        super::struct_command::HANDLER_ID,
        snapshot_capabilities,
        bindings,
    );
    let function_missing = structure_is_missing(
        super::struct_function::HANDLER_ID,
        snapshot_capabilities,
        bindings,
    );
    let event_missing = structure_is_missing(
        super::struct_event::HANDLER_ID,
        snapshot_capabilities,
        bindings,
    );
    let event_reference = (command_missing || function_missing)
        .then(|| event_reference(event_missing, bindings))
        .transpose()?
        .into_iter()
        .collect::<Vec<_>>();

    if command_missing {
        definitions.push(command_definition(event_reference.clone()));
    }
    if function_missing {
        definitions.push(function_definition(version, event_reference));
    }
    if event_missing {
        definitions.push(event_definition(version));
    }
    Ok(definitions)
}

fn structure_is_missing(
    handler_id: &str,
    snapshot_capabilities: Option<&RuntimeSnapshotCapabilities>,
    bindings: &[RegisteredHandlerBinding],
) -> bool {
    let Some(snapshot_capabilities) = snapshot_capabilities else {
        return false;
    };
    if !snapshot_capabilities.syntax_kinds.structures {
        return true;
    }
    !bindings.iter().any(|binding| {
        binding.handler_id == handler_id
            && (!binding.definition_ids.is_empty() || !binding.registration_ids.is_empty())
    })
}

fn event_definition(version: (u64, u64, u64)) -> DynamicSyntaxDefinition {
    let pattern = if version < (2, 9, 0) {
        "[on] <.+> [priority:with priority (:(lowest|low|normal|high|highest|monitor))]"
    } else {
        "[on] [:uncancelled|:cancelled|any:(any|all)] <.+> [priority:with priority (:(lowest|low|normal|high|highest|monitor))]"
    };
    DynamicSyntaxDefinition {
        local_id: EVENT_ID.to_owned(),
        kind: SyntaxKind::Structure,
        patterns: vec![pattern.to_owned()],
        // ScriptLoader checks custom Structures before the catch-all Event
        // path. StructEvent later published that same slot as priority 600.
        priority: 600,
        before: Vec::new(),
        after: Vec::new(),
        return_type: None,
        return_multiplicity: None,
        structure_node_type: Some(StructureNodeType::Section),
        structure_body_mode: Some(StructureBodyMode::Trigger),
        entry_validator: None,
        handler: "core.structure.struct-event".to_owned(),
        metadata: Vec::new(),
    }
}

pub(super) fn is_event_registration(registration_id: &str) -> bool {
    let Some(dynamic_id) = registration_id.strip_prefix("dynamic:") else {
        return false;
    };
    let Some((component_id, local_id)) = dynamic_id.split_once('/') else {
        return false;
    };
    component_id == crate::COMPONENT_ID && local_id == EVENT_ID
}

fn command_definition(event_reference: Vec<DynamicSyntaxReference>) -> DynamicSyntaxDefinition {
    DynamicSyntaxDefinition {
        local_id: COMMAND_ID.to_owned(),
        kind: SyntaxKind::Structure,
        patterns: vec!["command <.+>".to_owned()],
        priority: 500,
        before: event_reference,
        after: Vec::new(),
        return_type: None,
        return_multiplicity: None,
        structure_node_type: Some(StructureNodeType::Section),
        structure_body_mode: Some(StructureBodyMode::Entries),
        entry_validator: Some(legacy_command_validator()),
        handler: "core.structure.struct-command".to_owned(),
        metadata: Vec::new(),
    }
}

fn function_definition(
    version: (u64, u64, u64),
    event_reference: Vec<DynamicSyntaxReference>,
) -> DynamicSyntaxDefinition {
    let pattern = if version < (2, 7, 0) {
        "function <.+>"
    } else {
        "[:local] function <.+>"
    };
    DynamicSyntaxDefinition {
        local_id: FUNCTION_ID.to_owned(),
        kind: SyntaxKind::Structure,
        patterns: vec![pattern.to_owned()],
        priority: 400,
        before: event_reference,
        after: Vec::new(),
        return_type: None,
        return_multiplicity: None,
        structure_node_type: Some(StructureNodeType::Section),
        structure_body_mode: Some(StructureBodyMode::Trigger),
        entry_validator: None,
        handler: "core.structure.struct-function".to_owned(),
        metadata: Vec::new(),
    }
}

fn event_reference(
    event_missing: bool,
    bindings: &[RegisteredHandlerBinding],
) -> Result<DynamicSyntaxReference, String> {
    if event_missing {
        return Ok(DynamicSyntaxReference::Dynamic(DynamicSyntaxId {
            component_id: None,
            local_id: EVENT_ID.to_owned(),
        }));
    }
    let binding = bindings
        .iter()
        .find(|binding| binding.handler_id == super::struct_event::HANDLER_ID)
        .ok_or_else(|| {
            "CoreLibrary could not order a legacy Structure before StructEvent".to_owned()
        })?;
    if let Some(registration_id) = binding.registration_ids.first() {
        return Ok(DynamicSyntaxReference::RegistrationId(
            registration_id.clone(),
        ));
    }
    binding
        .definition_ids
        .first()
        .cloned()
        .map(DynamicSyntaxReference::DefinitionId)
        .ok_or_else(|| "CoreLibrary received an empty StructEvent handler binding".to_owned())
}

fn legacy_command_validator() -> StructureEntryValidator {
    // Skript 2.6.4 validates these keys with SectionValidator. EntryData and
    // its typed subclasses did not exist yet, so the synthetic registration
    // deliberately keeps ordinary entries raw instead of inventing modern
    // implementation classes or parser behavior.
    const VALIDATOR_CLASS: &str = "ch.njol.skript.config.validate.SectionValidator";
    StructureEntryValidator {
        entry_data: vec![
            entry(
                "usage",
                None,
                true,
                VALIDATOR_CLASS,
                StructureEntryKind::KeyValue,
            ),
            entry(
                "description",
                Some(r#""""#),
                true,
                VALIDATOR_CLASS,
                StructureEntryKind::KeyValue,
            ),
            entry(
                "permission",
                Some(r#""""#),
                true,
                VALIDATOR_CLASS,
                StructureEntryKind::KeyValue,
            ),
            entry(
                "permission message",
                None,
                true,
                VALIDATOR_CLASS,
                StructureEntryKind::KeyValue,
            ),
            entry(
                "cooldown",
                None,
                true,
                VALIDATOR_CLASS,
                StructureEntryKind::KeyValue,
            ),
            entry(
                "cooldown message",
                None,
                true,
                VALIDATOR_CLASS,
                StructureEntryKind::KeyValue,
            ),
            entry(
                "cooldown bypass",
                Some(r#""""#),
                true,
                VALIDATOR_CLASS,
                StructureEntryKind::KeyValue,
            ),
            entry(
                "cooldown storage",
                None,
                true,
                VALIDATOR_CLASS,
                StructureEntryKind::KeyValue,
            ),
            entry(
                "aliases",
                Some("[]"),
                true,
                VALIDATOR_CLASS,
                StructureEntryKind::KeyValue,
            ),
            entry(
                "executable by",
                Some(r#""console,players""#),
                true,
                VALIDATOR_CLASS,
                StructureEntryKind::KeyValue,
            ),
            entry(
                "trigger",
                None,
                false,
                VALIDATOR_CLASS,
                StructureEntryKind::Trigger,
            ),
        ],
    }
}

fn entry(
    key: &str,
    default_value: Option<&str>,
    optional: bool,
    entry_data_class: &str,
    kind: StructureEntryKind,
) -> StructureEntryData {
    StructureEntryData {
        parent_entry_index: None,
        key: key.to_owned(),
        default_value: default_value.map(str::to_owned),
        optional,
        multiple: false,
        entry_data_class: entry_data_class.to_owned(),
        kind,
        separator: Some(": ".to_owned()),
        value_type: None,
        string_mode: None,
        return_types: Vec::new(),
        flags: None,
        nested_validator_present: false,
    }
}

fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let mut components = version
        .split(|character: char| !character.is_ascii_digit())
        .filter(|component| !component.is_empty())
        .filter_map(|component| component.parse::<u64>().ok());
    Some((
        components.next()?,
        components.next()?,
        components.next().unwrap_or(0),
    ))
}

#[cfg(test)]
mod tests {
    use super::{COMMAND_ID, EVENT_ID, FUNCTION_ID, definitions, is_event_registration};
    use crate::nlaocs::skript_parser_addon::types::{
        DynamicSyntaxReference, RegisteredHandlerBinding, RuntimeAliasCapabilities,
        RuntimeSnapshotCapabilities, RuntimeSyntaxKindCapabilities, StructureEntryKind,
    };

    fn capabilities(structures: bool) -> RuntimeSnapshotCapabilities {
        RuntimeSnapshotCapabilities {
            syntax_api: if structures {
                "registry"
            } else {
                "legacy-static"
            }
            .to_owned(),
            event_value_api: "legacy".to_owned(),
            syntax_kinds: RuntimeSyntaxKindCapabilities {
                conditions: true,
                effects: true,
                events: true,
                expressions: true,
                types: true,
                functions: true,
                sections: true,
                structures,
                properties: false,
                arithmetic: false,
                converters: true,
                comparators: true,
                event_values: true,
            },
            aliases: RuntimeAliasCapabilities {
                supported: true,
                collected: true,
            },
        }
    }

    fn binding(handler_id: &str) -> RegisteredHandlerBinding {
        RegisteredHandlerBinding {
            handler_id: handler_id.to_owned(),
            definition_ids: vec![format!("definition:{handler_id}")],
            registration_ids: vec![format!("registration:{handler_id}")],
        }
    }

    #[test]
    fn snapshots_without_structure_collection_receive_all_legacy_entries() {
        let capabilities = capabilities(false);
        let legacy = definitions((2, 6, 4), Some(&capabilities), &[]).unwrap();
        assert_eq!(
            legacy
                .iter()
                .map(|definition| definition.local_id.as_str())
                .collect::<Vec<_>>(),
            [COMMAND_ID, FUNCTION_ID, EVENT_ID]
        );
        assert_eq!(legacy[0].priority, 500);
        assert_eq!(legacy[1].priority, 400);
        assert_eq!(legacy[2].priority, 600);
    }

    #[test]
    fn unknown_snapshot_capabilities_do_not_guess_legacy_registrations() {
        assert!(definitions((2, 6, 4), None, &[]).unwrap().is_empty());
    }

    #[test]
    fn registry_snapshots_receive_only_actually_missing_structures() {
        let capabilities = capabilities(true);
        let bindings = [
            binding(super::super::struct_command::HANDLER_ID),
            binding(super::super::struct_function::HANDLER_ID),
        ];
        let missing_event = definitions((2, 7, 3), Some(&capabilities), &bindings).unwrap();
        assert_eq!(
            missing_event
                .iter()
                .map(|definition| definition.local_id.as_str())
                .collect::<Vec<_>>(),
            [EVENT_ID],
            "2.7 has StructCommand and StructFunction but not StructEvent"
        );

        let missing_modern_event = definitions((2, 16, 0), Some(&capabilities), &bindings).unwrap();
        assert_eq!(missing_modern_event.len(), 1);
        assert_eq!(missing_modern_event[0].local_id, EVENT_ID);
        assert_eq!(
            missing_modern_event[0].patterns,
            [
                "[on] [:uncancelled|:cancelled|any:(any|all)] <.+> [priority:with priority (:(lowest|low|normal|high|highest|monitor))]"
            ]
        );

        let all_bindings = [
            binding(super::super::struct_command::HANDLER_ID),
            binding(super::super::struct_function::HANDLER_ID),
            binding(super::super::struct_event::HANDLER_ID),
        ];
        assert!(
            definitions((2, 16, 0), Some(&capabilities), &all_bindings)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn partial_fallbacks_order_before_the_static_event_registration() {
        let capabilities = capabilities(true);
        let event = binding(super::super::struct_event::HANDLER_ID);
        let expected_registration = event.registration_ids[0].clone();
        let definitions = definitions((2, 7, 3), Some(&capabilities), &[event]).unwrap();
        assert_eq!(
            definitions
                .iter()
                .map(|definition| definition.local_id.as_str())
                .collect::<Vec<_>>(),
            [COMMAND_ID, FUNCTION_ID]
        );
        assert!(definitions.iter().all(|definition| {
            matches!(
                definition.before.as_slice(),
                [DynamicSyntaxReference::RegistrationId(registration_id)]
                    if registration_id == &expected_registration
            )
        }));
    }

    #[test]
    fn event_header_grammar_changes_at_skript_2_9() {
        let capabilities = capabilities(false);
        let event_pattern = |version| {
            definitions(version, Some(&capabilities), &[])
                .unwrap()
                .into_iter()
                .find(|definition| definition.local_id == EVENT_ID)
                .unwrap()
                .patterns
        };
        assert_eq!(
            event_pattern((2, 8, 7)),
            ["[on] <.+> [priority:with priority (:(lowest|low|normal|high|highest|monitor))]"]
        );
        assert_eq!(
            event_pattern((2, 9, 0)),
            [
                "[on] [:uncancelled|:cancelled|any:(any|all)] <.+> [priority:with priority (:(lowest|low|normal|high|highest|monitor))]"
            ]
        );
    }

    #[test]
    fn fallback_function_supports_local_from_skript_2_7() {
        let capabilities = capabilities(false);
        let function_pattern = |version| {
            definitions(version, Some(&capabilities), &[])
                .unwrap()
                .into_iter()
                .find(|definition| definition.local_id == FUNCTION_ID)
                .unwrap()
                .patterns
        };
        assert_eq!(function_pattern((2, 6, 4)), ["function <.+>"]);
        assert_eq!(function_pattern((2, 7, 0)), ["[:local] function <.+>"]);
    }

    #[test]
    fn legacy_event_pattern_keeps_the_loader_priority_suffix() {
        let capabilities = capabilities(false);
        let event = definitions((2, 6, 4), Some(&capabilities), &[])
            .unwrap()
            .into_iter()
            .find(|definition| definition.local_id == EVENT_ID)
            .unwrap();
        assert_eq!(
            event.patterns,
            ["[on] <.+> [priority:with priority (:(lowest|low|normal|high|highest|monitor))]"]
        );
    }

    #[test]
    fn legacy_event_identity_requires_the_core_component_and_local_id() {
        assert!(is_event_registration(
            "dynamic:nlaocs.core-library/legacy-struct-event"
        ));
        assert!(!is_event_registration(
            "dynamic:addon.example/legacy-struct-event"
        ));
        assert!(!is_event_registration(
            "dynamic:nlaocs.core-library/other-event"
        ));
    }

    #[test]
    fn legacy_command_keeps_the_required_trigger_and_no_modern_prefix() {
        let capabilities = capabilities(false);
        let command = definitions((2, 6, 4), Some(&capabilities), &[])
            .unwrap()
            .into_iter()
            .find(|definition| definition.local_id == COMMAND_ID)
            .unwrap();
        let entries = command.entry_validator.unwrap().entry_data;
        assert!(entries.iter().all(|entry| entry.key != "prefix"));
        let trigger = entries.iter().find(|entry| entry.key == "trigger").unwrap();
        assert!(!trigger.optional);
        assert_eq!(trigger.kind, StructureEntryKind::Trigger);
        assert!(entries.iter().all(|entry| {
            entry.entry_data_class == "ch.njol.skript.config.validate.SectionValidator"
        }));
    }
}

//! Synthetic Structure registrations for releases that predate StructureInfo.

use crate::nlaocs::skript_parser_addon::{
    dynamic_syntax_registry,
    types::{
        DynamicSyntaxDefinition, DynamicSyntaxId, DynamicSyntaxReference, StructureBodyMode,
        StructureEntryData, StructureEntryKind, StructureEntryValidator, StructureNodeType,
        SyntaxKind,
    },
};

const EVENT_ID: &str = "legacy-struct-event";
const COMMAND_ID: &str = "legacy-struct-command";
const FUNCTION_ID: &str = "legacy-struct-function";

pub(super) fn register_missing(skript_version: &str) -> Result<(), String> {
    let version = parse_version(skript_version)
        .ok_or_else(|| "CoreLibrary could not parse the Skript version".to_owned())?;

    for definition in definitions(version) {
        dynamic_syntax_registry::register(&definition).map_err(|error| error.message)?;
    }
    Ok(())
}

fn definitions(version: (u64, u64, u64)) -> Vec<DynamicSyntaxDefinition> {
    let mut definitions = Vec::new();
    if version < (2, 7, 0) {
        definitions.push(command_definition());
        definitions.push(function_definition());
    }
    if version < (2, 8, 0) {
        definitions.push(event_definition());
    }
    definitions
}

fn event_definition() -> DynamicSyntaxDefinition {
    DynamicSyntaxDefinition {
        local_id: EVENT_ID.to_owned(),
        kind: SyntaxKind::Structure,
        patterns: vec!["[on] <.+>".to_owned()],
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

fn command_definition() -> DynamicSyntaxDefinition {
    DynamicSyntaxDefinition {
        local_id: COMMAND_ID.to_owned(),
        kind: SyntaxKind::Structure,
        patterns: vec!["command <.+>".to_owned()],
        priority: 500,
        before: vec![event_reference()],
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

fn function_definition() -> DynamicSyntaxDefinition {
    DynamicSyntaxDefinition {
        local_id: FUNCTION_ID.to_owned(),
        kind: SyntaxKind::Structure,
        patterns: vec!["function <.+>".to_owned()],
        priority: 400,
        before: vec![event_reference()],
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

fn event_reference() -> DynamicSyntaxReference {
    DynamicSyntaxReference::Dynamic(DynamicSyntaxId {
        component_id: None,
        local_id: EVENT_ID.to_owned(),
    })
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
    use crate::nlaocs::skript_parser_addon::types::StructureEntryKind;

    #[test]
    fn legacy_versions_receive_only_the_structures_missing_from_ssg() {
        let legacy = definitions((2, 6, 4));
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
        assert_eq!(
            definitions((2, 7, 3))[0].local_id,
            EVENT_ID,
            "2.7 has StructCommand and StructFunction but not StructEvent"
        );
        assert!(definitions((2, 8, 0)).is_empty());
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
        let command = definitions((2, 6, 4))
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

use std::{collections::BTreeMap, sync::Arc};

use syntax_pattern_parser::syntax::{self, PluralRules};
use syntaxes::{
    Addon, AliasRegistry, Catalog, CatalogParts, ClassName, CommonSyntax, DefinitionId,
    Documentation, DynamicRegistryError, DynamicStructureBodyMode, DynamicSyntaxId,
    DynamicSyntaxInput, DynamicSyntaxOverrideInput, DynamicSyntaxRegistry, Effect, EntryData,
    EntryKind, EntryValidator, NodeType, Pattern, RegistrationId, Syntax, SyntaxCandidateSource,
    SyntaxKind, SyntaxOverrideTarget, SyntaxReference,
};

fn plural_rules() -> PluralRules {
    PluralRules::from_json(
        r#"{
            "algorithm":"singular-aware",
            "pluralOverrideSupported":false,
            "rules":[{
                "ruleOrder":0,
                "singular":"",
                "plural":"s",
                "completeWord":false,
                "origin":"built-in",
                "addon":{"name":"Skript","version":"test"}
            }]
        }"#,
    )
    .unwrap()
}

fn common_effect(
    registration_order: usize,
    definition_id: &str,
    registration_id: &str,
    pattern: &str,
) -> CommonSyntax {
    let rules = plural_rules();
    CommonSyntax {
        registration_order,
        documentation: Documentation::default(),
        id: None,
        element_class: ClassName("test.StaticEffect".to_owned()),
        super_class: None,
        no_doc: false,
        events: Vec::new(),
        deprecated: None,
        priority_name: None,
        priority: None,
        patterns: vec![Pattern {
            source: pattern.to_owned(),
            parsed: syntax::parse(pattern, &rules).unwrap(),
        }],
        addon: Addon {
            name: "Skript".to_owned(),
            version: "test".to_owned(),
        },
        definition_id: DefinitionId(definition_id.to_owned()),
        registration_id: RegistrationId(registration_id.to_owned()),
        related_property: None,
        supported_events: None,
        supported_events_state: None,
        experimental_syntax: None,
        experimental_syntax_state: None,
        return_handler: None,
        return_handler_state: None,
    }
}

fn catalog() -> Arc<Catalog> {
    Arc::new(Catalog::new(CatalogParts {
        syntaxes: vec![
            Syntax::Effect(Effect {
                common: common_effect(0, "definition:first", "registration:first", "static first"),
            }),
            Syntax::Effect(Effect {
                common: common_effect(
                    1,
                    "definition:second",
                    "registration:second",
                    "static second",
                ),
            }),
        ],
        converters: Vec::new(),
        comparators: Vec::new(),
        event_values: Vec::new(),
        properties: Vec::new(),
        operators: Vec::new(),
        operations: BTreeMap::new(),
        differences: Vec::new(),
        classes: Vec::new(),
        aliases: AliasRegistry {
            aliases: BTreeMap::new(),
            targets: Vec::new(),
        },
        plural_rules: plural_rules(),
        language: BTreeMap::new(),
    }))
}

fn dynamic_effect(local_id: &str, pattern: &str) -> DynamicSyntaxInput {
    DynamicSyntaxInput {
        local_id: local_id.to_owned(),
        kind: SyntaxKind::Effect,
        patterns: vec![pattern.to_owned()],
        priority: 0,
        before: Vec::new(),
        after: Vec::new(),
        return_type: None,
        return_multiplicity: None,
        structure_node_type: None,
        structure_body_mode: None,
        entry_validator: None,
        handler: format!("handle-{local_id}"),
        metadata: BTreeMap::new(),
    }
}

fn dynamic_candidates(snapshot: &syntaxes::DynamicSyntaxSnapshot) -> Vec<String> {
    snapshot
        .candidates
        .iter()
        .filter_map(|candidate| match &candidate.source {
            SyntaxCandidateSource::Dynamic(id) => Some(id.qualified()),
            SyntaxCandidateSource::Static(_) => None,
        })
        .collect()
}

fn string_entry(key: &str) -> EntryData {
    EntryData {
        key: key.to_owned(),
        default_value: Some(syntaxes::parse_json_value(r#""default""#).unwrap()),
        optional: false,
        multiple: false,
        entry_data_class: ClassName("test.StringEntryData".to_owned()),
        kind: EntryKind::Literal,
        separator: Some(": ".to_owned()),
        value_type: Some(ClassName("java.lang.String".to_owned())),
        string_mode: None,
        return_types: Vec::new(),
        flags: None,
        nested_validator: None,
    }
}

#[test]
fn empty_overlay_preserves_static_registration_order() {
    let registry = DynamicSyntaxRegistry::new(catalog());
    registry.begin_document("file:///test.sk", 1).unwrap();

    let snapshot = registry.freeze("file:///test.sk", 1).unwrap();
    let sources = snapshot
        .candidates
        .iter()
        .map(|candidate| candidate.source.clone())
        .collect::<Vec<_>>();

    assert!(snapshot.definitions.is_empty());
    assert!(snapshot.overrides.is_empty());
    assert_eq!(
        sources,
        [
            SyntaxCandidateSource::Static(0),
            SyntaxCandidateSource::Static(1)
        ]
    );
}

#[test]
fn registers_dynamic_effect_and_freezes_mixed_catalog() {
    let registry = DynamicSyntaxRegistry::new(catalog());
    let mut update = registry.begin_initial_update("test.addon", 0).unwrap();
    update
        .register(dynamic_effect("hello", "send dynamic %string%"))
        .unwrap();
    update.commit().unwrap();

    registry.begin_document("file:///test.sk", 1).unwrap();
    let snapshot = registry.freeze("file:///test.sk", 1).unwrap();

    assert_eq!(snapshot.definitions.len(), 1);
    assert_eq!(snapshot.candidates.len(), 3);
    assert_eq!(dynamic_candidates(&snapshot), ["dynamic:test.addon/hello"]);
    let definition = snapshot
        .definitions
        .get(&DynamicSyntaxId::new("test.addon", "hello"))
        .unwrap();
    assert_eq!(definition.patterns[0].source, "send dynamic %string%");
    assert_eq!(definition.handler, "handle-hello");
}

#[test]
fn resolves_registration_and_definition_overrides_deterministically() {
    let registry = DynamicSyntaxRegistry::new(catalog());
    let mut first = registry.begin_initial_update("test.first", 0).unwrap();
    first
        .register_override(DynamicSyntaxOverrideInput {
            local_id: "by-registration".to_owned(),
            target: SyntaxOverrideTarget::Registration(RegistrationId(
                "registration:first".to_owned(),
            )),
            priority: 10,
            handler: "registration-handler".to_owned(),
            metadata: BTreeMap::new(),
        })
        .unwrap();
    first.commit().unwrap();

    let mut second = registry.begin_initial_update("test.second", 1).unwrap();
    second
        .register_override(DynamicSyntaxOverrideInput {
            local_id: "by-definition".to_owned(),
            target: SyntaxOverrideTarget::Definition(DefinitionId("definition:first".to_owned())),
            priority: -10,
            handler: "definition-handler".to_owned(),
            metadata: BTreeMap::new(),
        })
        .unwrap();
    second.commit().unwrap();

    registry.begin_document("file:///test.sk", 1).unwrap();
    let snapshot = registry.freeze("file:///test.sk", 1).unwrap();
    let candidate = snapshot
        .candidates
        .iter()
        .find(|candidate| candidate.source == SyntaxCandidateSource::Static(0))
        .unwrap();
    let handlers = candidate
        .overrides
        .iter()
        .map(|handler| handler.handler.as_str())
        .collect::<Vec<_>>();

    assert_eq!(handlers, ["definition-handler", "registration-handler"]);
}

#[test]
fn before_after_constraints_produce_a_deterministic_order() {
    let registry = DynamicSyntaxRegistry::new(catalog());
    let mut update = registry.begin_initial_update("test.addon", 0).unwrap();
    let mut before = dynamic_effect("before", "dynamic before");
    before.before = vec![SyntaxReference::Registration(RegistrationId(
        "registration:first".to_owned(),
    ))];
    let mut after = dynamic_effect("after", "dynamic after");
    after.after = vec![SyntaxReference::Dynamic(DynamicSyntaxId::new(
        "test.addon",
        "before",
    ))];
    update.register(after).unwrap();
    update.register(before).unwrap();
    update.commit().unwrap();

    registry.begin_document("file:///test.sk", 1).unwrap();
    let snapshot = registry.freeze("file:///test.sk", 1).unwrap();
    let order = snapshot
        .candidates
        .iter()
        .map(|candidate| match &candidate.source {
            SyntaxCandidateSource::Static(index) => format!("static:{index}"),
            SyntaxCandidateSource::Dynamic(id) => id.qualified(),
        })
        .collect::<Vec<_>>();

    let before_position = order
        .iter()
        .position(|value| value == "dynamic:test.addon/before")
        .unwrap();
    let after_position = order
        .iter()
        .position(|value| value == "dynamic:test.addon/after")
        .unwrap();
    let static_position = order.iter().position(|value| value == "static:0").unwrap();
    assert!(before_position < after_position);
    assert!(before_position < static_position);
}

#[test]
fn reports_invalid_patterns_duplicate_ids_and_priority_cycles() {
    let registry = DynamicSyntaxRegistry::new(catalog());
    let mut update = registry.begin_initial_update("test.addon", 0).unwrap();
    let invalid = update
        .register(dynamic_effect("retryable", "[(unclosed"))
        .unwrap_err();
    assert!(matches!(
        invalid,
        DynamicRegistryError::InvalidPattern { .. }
    ));
    update
        .register(dynamic_effect("retryable", "valid after failure"))
        .unwrap();
    let duplicate = update
        .register(dynamic_effect("retryable", "duplicate"))
        .unwrap_err();
    assert!(matches!(
        duplicate,
        DynamicRegistryError::DuplicateId { .. }
    ));

    let mut left = dynamic_effect("left", "left");
    left.before = vec![SyntaxReference::Dynamic(DynamicSyntaxId::new(
        "test.addon",
        "right",
    ))];
    let mut right = dynamic_effect("right", "right");
    right.before = vec![SyntaxReference::Dynamic(DynamicSyntaxId::new(
        "test.addon",
        "left",
    ))];
    update.register(left).unwrap();
    update.register(right).unwrap();
    update.commit().unwrap();

    registry.begin_document("file:///test.sk", 1).unwrap();
    let error = registry.freeze("file:///test.sk", 1).unwrap_err();
    assert!(matches!(error, DynamicRegistryError::PriorityCycle { .. }));
}

#[test]
fn freeze_rejects_updates_and_savepoint_restores_prepass_state() {
    let registry = DynamicSyntaxRegistry::new(catalog());
    registry.begin_document("file:///test.sk", 1).unwrap();
    let savepoint = registry.savepoint("file:///test.sk", 1).unwrap();

    let mut update = registry
        .begin_document_update("test.addon", 0, "file:///test.sk", 1)
        .unwrap();
    update
        .register(dynamic_effect("temporary", "temporary"))
        .unwrap();
    update.commit().unwrap();
    registry.rollback_to(&savepoint).unwrap();

    let snapshot = registry.freeze("file:///test.sk", 1).unwrap();
    assert!(snapshot.definitions.is_empty());
    let error = registry
        .begin_document_update("test.addon", 0, "file:///test.sk", 1)
        .err()
        .expect("frozen registry must reject updates");
    assert!(matches!(error, DynamicRegistryError::Frozen { .. }));
}

#[test]
fn unloading_a_component_preserves_frozen_snapshots_but_removes_future_entries() {
    let registry = DynamicSyntaxRegistry::new(catalog());
    let mut update = registry.begin_initial_update("test.addon", 0).unwrap();
    update
        .register(dynamic_effect("initial", "initial"))
        .unwrap();
    update.commit().unwrap();

    registry.begin_document("file:///first.sk", 1).unwrap();
    let frozen = registry.freeze("file:///first.sk", 1).unwrap();
    registry.remove_component("test.addon").unwrap();
    registry.begin_document("file:///second.sk", 1).unwrap();
    let future = registry.freeze("file:///second.sk", 1).unwrap();

    assert_eq!(frozen.definitions.len(), 1);
    assert!(future.definitions.is_empty());
}

#[test]
fn stale_document_revisions_are_rejected() {
    let registry = DynamicSyntaxRegistry::new(catalog());
    registry.begin_document("file:///test.sk", 2).unwrap();
    let error = registry.begin_document("file:///test.sk", 1).unwrap_err();
    assert!(matches!(
        error,
        DynamicRegistryError::StaleDocumentRevision {
            actual: 1,
            latest: 2,
            ..
        }
    ));
}

#[test]
fn dynamic_structure_retains_node_body_and_entry_validator_metadata() {
    let registry = DynamicSyntaxRegistry::new(catalog());
    let mut update = registry.begin_initial_update("test.structure", 0).unwrap();
    update
        .register(DynamicSyntaxInput {
            local_id: "root".to_owned(),
            kind: SyntaxKind::Structure,
            patterns: vec!["root".to_owned()],
            priority: 0,
            before: Vec::new(),
            after: Vec::new(),
            return_type: None,
            return_multiplicity: None,
            structure_node_type: Some(NodeType::Section),
            structure_body_mode: Some(DynamicStructureBodyMode::Entries),
            entry_validator: Some(EntryValidator {
                entry_data: vec![string_entry("name")],
            }),
            handler: "test.structure.root".to_owned(),
            metadata: BTreeMap::new(),
        })
        .unwrap();
    update.commit().unwrap();
    registry.begin_document("file:///structure.sk", 1).unwrap();
    let snapshot = registry.freeze("file:///structure.sk", 1).unwrap();
    let definition = snapshot
        .definitions
        .get(&DynamicSyntaxId::new("test.structure", "root"))
        .unwrap();

    assert_eq!(definition.structure_node_type, Some(NodeType::Section));
    assert_eq!(
        definition.structure_body_mode,
        Some(DynamicStructureBodyMode::Entries)
    );
    assert_eq!(
        definition.entry_validator.as_ref().unwrap().entry_data[0].default_value,
        Some(syntaxes::parse_json_value(r#""default""#).unwrap())
    );
}

#[test]
fn dynamic_structure_metadata_is_rejected_for_other_kinds_and_invalid_combinations() {
    let registry = DynamicSyntaxRegistry::new(catalog());
    let mut update = registry.begin_initial_update("test.validation", 0).unwrap();

    let mut non_structure = dynamic_effect("non-structure", "not a structure");
    non_structure.structure_node_type = Some(NodeType::Section);
    assert!(matches!(
        update.register(non_structure),
        Err(DynamicRegistryError::InvalidInput { .. })
    ));

    let mut missing_validator = dynamic_effect("missing-validator", "structure");
    missing_validator.kind = SyntaxKind::Structure;
    missing_validator.structure_body_mode = Some(DynamicStructureBodyMode::Entries);
    assert!(matches!(
        update.register(missing_validator),
        Err(DynamicRegistryError::InvalidInput { .. })
    ));

    let mut simple_trigger = dynamic_effect("simple-trigger", "simple");
    simple_trigger.kind = SyntaxKind::Structure;
    simple_trigger.structure_node_type = Some(NodeType::Simple);
    simple_trigger.structure_body_mode = Some(DynamicStructureBodyMode::Trigger);
    assert!(matches!(
        update.register(simple_trigger),
        Err(DynamicRegistryError::InvalidInput { .. })
    ));

    let mut container_without_validator = dynamic_effect("bad-container", "container");
    container_without_validator.kind = SyntaxKind::Structure;
    container_without_validator.structure_node_type = Some(NodeType::Section);
    container_without_validator.entry_validator = Some(EntryValidator {
        entry_data: vec![EntryData {
            kind: EntryKind::Container,
            ..string_entry("container")
        }],
    });
    assert!(matches!(
        update.register(container_without_validator),
        Err(DynamicRegistryError::InvalidInput { .. })
    ));
}

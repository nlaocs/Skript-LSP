use std::{collections::BTreeMap, sync::Arc};

use skript_parser::{catalog_pattern_candidates, snapshot_pattern_candidates};
use syntax_pattern_parser::syntax::{self, PluralRules};
use syntaxes::{
    Addon, AliasRegistry, Catalog, CatalogParts, ClassName, CommonSyntax, DefinitionId,
    Documentation, DynamicSyntaxInput, DynamicSyntaxRegistry, Effect, Pattern, RegistrationId,
    Syntax, SyntaxCandidateSource, SyntaxKind, SyntaxReference,
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

fn effect(
    registration_order: usize,
    definition_id: &str,
    registration_id: &str,
    source: &str,
) -> Syntax {
    let rules = plural_rules();
    Syntax::Effect(Effect {
        common: CommonSyntax {
            registration_order,
            documentation: Documentation::default(),
            id: None,
            element_class: ClassName("test.Effect".to_owned()),
            super_class: None,
            no_doc: false,
            events: Vec::new(),
            deprecated: None,
            priority_name: None,
            priority: None,
            patterns: vec![Pattern {
                source: source.to_owned(),
                parsed: syntax::parse(source, &rules).unwrap(),
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
        },
    })
}

fn catalog() -> Arc<Catalog> {
    Arc::new(Catalog::new(CatalogParts {
        syntaxes: vec![
            effect(0, "definition:first", "registration:first", "static first"),
            effect(
                1,
                "definition:second",
                "registration:second",
                "static second",
            ),
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
    }))
}

#[test]
fn catalog_adapter_filters_kinds_and_borrows_registered_patterns() {
    let catalog = catalog();
    let candidates = catalog_pattern_candidates(&catalog, SyntaxKind::Effect);

    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.definition_id.as_str())
            .collect::<Vec<_>>(),
        ["definition:first", "definition:second"]
    );
    assert_eq!(candidates[0].patterns[0].source, "static first");
    assert!(catalog_pattern_candidates(&catalog, SyntaxKind::Condition).is_empty());
}

#[test]
fn snapshot_adapter_preserves_resolved_static_and_dynamic_order() {
    let catalog = catalog();
    let registry = DynamicSyntaxRegistry::new(Arc::clone(&catalog));
    let mut update = registry.begin_initial_update("test.addon", 0).unwrap();
    update
        .register(DynamicSyntaxInput {
            local_id: "before".to_owned(),
            kind: SyntaxKind::Effect,
            patterns: vec!["dynamic before".to_owned()],
            priority: 0,
            before: vec![SyntaxReference::Registration(RegistrationId(
                "registration:first".to_owned(),
            ))],
            after: Vec::new(),
            return_type: None,
            return_multiplicity: None,
            handler: "handle-before".to_owned(),
            metadata: BTreeMap::new(),
        })
        .unwrap();
    update.commit().unwrap();
    registry.begin_document("file:///test.sk", 1).unwrap();
    let snapshot = registry.freeze("file:///test.sk", 1).unwrap();

    let candidates = snapshot_pattern_candidates(&catalog, &snapshot, SyntaxKind::Effect);
    assert_eq!(candidates.len(), snapshot.candidates.len());
    for (index, (candidate, resolved)) in candidates.iter().zip(&snapshot.candidates).enumerate() {
        assert_eq!(candidate.resolved_order, Some(index));
        let expected_definition = match &resolved.source {
            SyntaxCandidateSource::Static(index) => catalog
                .syntax_at(*index)
                .unwrap()
                .definition_id()
                .as_str()
                .to_owned(),
            SyntaxCandidateSource::Dynamic(id) => id.qualified(),
        };
        assert_eq!(candidate.definition_id, expected_definition);
    }

    let dynamic_position = candidates
        .iter()
        .position(|candidate| candidate.definition_id == "dynamic:test.addon/before")
        .unwrap();
    let first_position = candidates
        .iter()
        .position(|candidate| candidate.definition_id == "definition:first")
        .unwrap();
    assert!(dynamic_position < first_position);
    assert_eq!(
        candidates[dynamic_position].patterns[0].source,
        "dynamic before"
    );
}

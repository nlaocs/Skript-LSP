use std::collections::BTreeMap;

use serde_json::json;
use skript_parser::{
    ExpressionLeafParse, ExpressionLeafRequest, ExpressionParseContext, ExpressionParseEnvironment,
    MappedSource, NoopExpressionEnvironment, PatternHookControl, PatternHookEvent,
    PatternMatchEnvironment, RawTreeOptions, StructureBody, StructureDiagnosticKind,
    StructureDocument, StructureDocumentNode, StructureEntryValue, StructureExitDecision,
    StructureHookRequest, StructureParseRequest, StructureParserConfig, TextRange,
    TypeExpressionOutcome, TypeExpressionRequest, parse_raw_tree, parse_structures,
    parse_structures_with_snapshot,
};
use syntax_pattern_parser::syntax::{self, PluralRules};
use syntaxes::{
    Addon, AliasRegistry, Catalog, CatalogParts, ClassName, CommonSyntax, DefinitionId,
    Documentation, DynamicPattern, DynamicStructureBodyMode, DynamicSyntaxDefinition,
    DynamicSyntaxId, DynamicSyntaxSnapshot, EntryData, EntryKind, EntryValidator, NodeType,
    Pattern, RankedSyntaxCandidate, RegistrationId, Structure, Syntax, SyntaxCandidateSource,
    SyntaxKind,
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
    .expect("test plural rules must be valid")
}

fn common_structure(
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
        element_class: ClassName("test.Structure".to_owned()),
        super_class: None,
        no_doc: false,
        events: Vec::new(),
        deprecated: None,
        priority_name: None,
        priority: None,
        patterns: vec![Pattern {
            source: pattern.to_owned(),
            parsed: syntax::parse(pattern, &rules).expect("literal test pattern must parse"),
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

fn structure(
    order: usize,
    name: &str,
    node_type: NodeType,
    entry_validator: Option<EntryValidator>,
) -> Syntax {
    Syntax::Structure(Structure {
        common: common_structure(
            order,
            &format!("definition:{name}"),
            &format!("registration:{name}"),
            name,
        ),
        entry_validator,
        node_type: Some(node_type),
    })
}

fn catalog(syntaxes: Vec<Syntax>) -> Catalog {
    Catalog::new(CatalogParts {
        syntaxes,
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
    })
}

fn parse(catalog: &Catalog, text: &str) -> StructureDocument {
    let source = MappedSource::identity(text);
    let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
    parse_structures(
        catalog,
        StructureParseRequest {
            source: &source,
            tree: &tree,
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        StructureParserConfig::default(),
    )
    .expect("synthetic Structure document must parse")
}

struct RejectStructureExit {
    reject_registration: Option<&'static str>,
}

impl PatternMatchEnvironment for RejectStructureExit {
    fn resolve_type(
        &mut self,
        _request: TypeExpressionRequest<'_>,
    ) -> Result<TypeExpressionOutcome, String> {
        Ok(TypeExpressionOutcome::default())
    }

    fn dispatch_hook(
        &mut self,
        _event: PatternHookEvent<'_>,
    ) -> Result<PatternHookControl, String> {
        Ok(PatternHookControl::Continue)
    }
}

impl ExpressionParseEnvironment for RejectStructureExit {
    fn parse_expression_leaf(
        &mut self,
        _request: ExpressionLeafRequest<'_>,
    ) -> Result<ExpressionLeafParse, String> {
        Ok(ExpressionLeafParse::default())
    }

    fn exit_structure(
        &mut self,
        request: StructureHookRequest<'_>,
    ) -> Result<StructureExitDecision, String> {
        if let Some(registration_id) = self.reject_registration
            && request.candidate.matched.registration_id.as_str() != registration_id
        {
            return Ok(StructureExitDecision::Accept);
        }
        Ok(StructureExitDecision::Reject {
            reason: "rejected after parsing the Structure body".to_owned(),
            diagnostics: Vec::new(),
        })
    }

    fn state_revision(&self) -> Result<u64, String> {
        Ok(0)
    }
}

#[test]
fn structure_exit_rejection_is_recoverable_and_keeps_identity() {
    let catalog = catalog(vec![structure(0, "root", NodeType::Section, None)]);
    let source = MappedSource::identity("root:\n    unknown child\n");
    let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
    let document = parse_structures(
        &catalog,
        StructureParseRequest {
            source: &source,
            tree: &tree,
            context: ExpressionParseContext::default(),
        },
        &mut RejectStructureExit {
            reject_registration: None,
        },
        StructureParserConfig::default(),
    )
    .expect("Structure exit rejection must not abort the document");

    let StructureDocumentNode::Structure(matches) = &document.roots[0] else {
        panic!("Structure result expected");
    };
    assert!(matches.selected.is_none());
    let trace = matches
        .unknown
        .as_ref()
        .expect("rejected Structure remains source preserving")
        .failure
        .as_ref()
        .expect("exit rejection is ranked");
    assert!(matches!(
        trace.root_cause().failure.reasons.as_slice(),
        [skript_parser::PatternFailureReason::HookRejected { reason }]
            if reason == "rejected after parsing the Structure body"
    ));
    assert_eq!(
        trace
            .frame
            .as_ref()
            .expect("semantic rejection keeps syntax identity")
            .registration_id,
        "registration:root"
    );
}

fn alternative_structure_catalog() -> Catalog {
    let mut first_syntax = structure(0, "lifecycle structure", NodeType::Section, None);
    let Syntax::Structure(first) = &mut first_syntax else {
        unreachable!("structure helper must create a Structure syntax");
    };
    first.common.definition_id = DefinitionId("definition:first".to_owned());
    first.common.registration_id = RegistrationId("registration:first".to_owned());

    let mut fallback = first.clone();
    fallback.common.definition_id = DefinitionId("definition:fallback".to_owned());
    fallback.common.registration_id = RegistrationId("registration:fallback".to_owned());
    catalog(vec![first_syntax, Syntax::Structure(fallback)])
}

#[test]
fn structure_exit_rejection_retries_a_later_candidate() {
    let catalog = alternative_structure_catalog();
    let source = MappedSource::identity("lifecycle structure:\n    body\n");
    let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
    let mut environment = RejectStructureExit {
        reject_registration: Some("registration:first"),
    };
    let document = parse_structures(
        &catalog,
        StructureParseRequest {
            source: &source,
            tree: &tree,
            context: ExpressionParseContext::default(),
        },
        &mut environment,
        StructureParserConfig::default(),
    )
    .expect("a rejected Structure candidate must remain recoverable");

    let StructureDocumentNode::Structure(matches) = &document.roots[0] else {
        panic!("Structure result expected");
    };
    let selected = matches
        .selected
        .as_ref()
        .expect("the fallback Structure must be selected");
    assert_eq!(selected.matched.registration_id, "registration:fallback");
    assert!(matches.unknown.is_none());
}

fn selected(document: &StructureDocument) -> &skript_parser::StructureCandidate {
    match &document.roots[0] {
        StructureDocumentNode::Structure(matches) => matches
            .selected
            .as_ref()
            .expect("the synthetic Structure must claim its root"),
        other => panic!("expected a Structure root, got {other:?}"),
    }
}

fn partial(document: &StructureDocument) -> &skript_parser::StructureCandidate {
    match &document.roots[0] {
        StructureDocumentNode::Structure(matches) => matches
            .unknown
            .as_ref()
            .and_then(|unknown| unknown.partial.as_deref())
            .expect("invalid Structure must retain a partial candidate"),
        other => panic!("expected a Structure root, got {other:?}"),
    }
}

fn key_value(key: &str, optional: bool, multiple: bool) -> EntryData {
    EntryData {
        key: key.to_owned(),
        default_value: None,
        optional,
        multiple,
        entry_data_class: ClassName("test.KeyValueEntryData".to_owned()),
        kind: EntryKind::KeyValue,
        separator: Some(": ".to_owned()),
        value_type: None,
        string_mode: None,
        return_types: Vec::new(),
        flags: None,
        nested_validator: None,
    }
}

fn container(key: &str, nested_validator: EntryValidator) -> EntryData {
    EntryData {
        key: key.to_owned(),
        default_value: None,
        optional: false,
        multiple: false,
        entry_data_class: ClassName("test.ContainerEntryData".to_owned()),
        kind: EntryKind::Container,
        separator: None,
        value_type: None,
        string_mode: None,
        return_types: Vec::new(),
        flags: None,
        nested_validator: Some(nested_validator),
    }
}

#[test]
fn respects_simple_section_and_both_node_types() {
    let catalog = catalog(vec![
        structure(0, "simple", NodeType::Simple, None),
        structure(1, "section", NodeType::Section, None),
        structure(2, "both", NodeType::Both, None),
    ]);
    let document = parse(&catalog, "simple\nsection:\n    body\nboth\n");

    assert_eq!(document.roots.len(), 3);
    let candidates = document
        .roots
        .iter()
        .map(|root| match root {
            StructureDocumentNode::Structure(matches) => matches
                .selected
                .as_ref()
                .expect("all three roots should be claimed"),
            other => panic!("expected a Structure root, got {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(candidates[0].declared_node_type, NodeType::Simple);
    assert_eq!(candidates[0].actual_node_type, NodeType::Simple);
    assert_eq!(candidates[1].declared_node_type, NodeType::Section);
    assert_eq!(candidates[1].actual_node_type, NodeType::Section);
    assert_eq!(candidates[2].declared_node_type, NodeType::Both);
    assert_eq!(candidates[2].actual_node_type, NodeType::Simple);
}

#[test]
fn validates_nested_container_and_required_key_value_entry() {
    let nested = EntryValidator {
        entry_data: vec![key_value("name", false, false)],
    };
    let root_validator = EntryValidator {
        entry_data: vec![container("settings", nested)],
    };
    let catalog = catalog(vec![structure(
        0,
        "config",
        NodeType::Section,
        Some(root_validator),
    )]);
    let document = parse(&catalog, "config:\n    settings:\n        name: alice\n");
    let candidate = selected(&document);

    let StructureBody::Entries(entries) = &candidate.body else {
        panic!("validator-backed Section must expose parsed entries");
    };
    assert_eq!(entries.len(), 1);
    let StructureEntryValue::Container(nested_entries) = &entries[0].value else {
        panic!("settings must retain its nested validator entries");
    };
    assert_eq!(nested_entries.len(), 1);
    assert_eq!(nested_entries[0].key, "name");
    assert_eq!(nested_entries[0].source, "name: alice");
    assert!(
        matches!(nested_entries[0].value, StructureEntryValue::Raw(ref value) if value == "alice")
    );
    assert!(document.diagnostics.is_empty());
}

#[test]
fn reports_duplicate_non_multiple_and_missing_required_entries() {
    let validator = EntryValidator {
        entry_data: vec![key_value("name", false, false)],
    };
    let catalog = catalog(vec![structure(
        0,
        "config",
        NodeType::Section,
        Some(validator),
    )]);

    let duplicate = parse(&catalog, "config:\n    name: first\n    name: second\n");
    assert!(
        duplicate
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == StructureDiagnosticKind::DuplicateEntry)
    );
    let duplicate_candidate = partial(&duplicate);
    let StructureBody::Entries(entries) = &duplicate_candidate.body else {
        panic!("validator-backed Section must expose entries");
    };
    assert_eq!(
        entries.len(),
        1,
        "non-multiple entries keep only the first value"
    );

    let missing = parse(&catalog, "config:\n");
    assert!(
        missing
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == StructureDiagnosticKind::MissingRequiredEntry)
    );
}

#[test]
fn required_entry_with_default_still_reports_missing_and_keeps_unquoted_fallback() {
    let mut entry = key_value("name", false, false);
    entry.default_value = Some(json!("fallback"));
    let catalog = catalog(vec![structure(
        0,
        "config",
        NodeType::Section,
        Some(EntryValidator {
            entry_data: vec![entry],
        }),
    )]);
    let document = parse(&catalog, "config:\n");
    let candidate = partial(&document);

    assert!(
        document
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == StructureDiagnosticKind::MissingRequiredEntry)
    );
    let StructureBody::Entries(entries) = &candidate.body else {
        panic!("validator-backed Section must expose defaulted entries");
    };
    assert_eq!(entries.len(), 1);
    assert!(entries[0].defaulted);
    assert_eq!(entries[0].source, "fallback");
    assert!(matches!(
        entries[0].value,
        StructureEntryValue::Raw(ref value) if value == "fallback"
    ));
}

#[test]
fn multiple_entries_retain_every_occurrence() {
    let catalog = catalog(vec![structure(
        0,
        "config",
        NodeType::Section,
        Some(EntryValidator {
            entry_data: vec![key_value("tag", true, true)],
        }),
    )]);
    let document = parse(&catalog, "config:\n    tag: first\n    tag: second\n");
    let candidate = selected(&document);

    let StructureBody::Entries(entries) = &candidate.body else {
        panic!("validator-backed Section must expose repeated entries");
    };
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].source, "tag: first");
    assert_eq!(entries[1].source, "tag: second");
    assert!(
        !document
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == StructureDiagnosticKind::DuplicateEntry)
    );
}

#[test]
fn wrong_node_type_is_not_selected() {
    let section_only = parse(
        &catalog(vec![structure(0, "section-only", NodeType::Section, None)]),
        "section-only\n",
    );
    let StructureDocumentNode::Structure(section_matches) = &section_only.roots[0] else {
        panic!("simple root must still produce Structure match results");
    };
    assert!(section_matches.selected.is_none());
    assert!(section_matches.unknown.is_some());

    let simple_only = parse(
        &catalog(vec![structure(0, "simple-only", NodeType::Simple, None)]),
        "simple-only:\n",
    );
    let StructureDocumentNode::Structure(simple_matches) = &simple_only.roots[0] else {
        panic!("section root must still produce Structure match results");
    };
    assert!(simple_matches.selected.is_none());
    assert!(simple_matches.unknown.is_some());
}

#[test]
fn custom_separator_matches_case_insensitive_key() {
    let mut entry = key_value("Name", false, false);
    entry.separator = Some(" = ".to_owned());
    let catalog = catalog(vec![structure(
        0,
        "config",
        NodeType::Section,
        Some(EntryValidator {
            entry_data: vec![entry],
        }),
    )]);
    let document = parse(&catalog, "config:\n    nAmE = value\n");
    let candidate = selected(&document);

    let StructureBody::Entries(entries) = &candidate.body else {
        panic!("validator-backed Section must expose custom-separator entries");
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].source, "nAmE = value");
    assert!(matches!(
        entries[0].value,
        StructureEntryValue::Raw(ref value) if value == "value"
    ));
    assert!(document.diagnostics.is_empty());
}

#[test]
fn unknown_entry_data_on_a_section_preserves_the_unknown_entry() {
    let catalog = catalog(vec![structure(
        0,
        "root",
        NodeType::Section,
        Some(EntryValidator {
            entry_data: vec![EntryData {
                key: "custom".to_owned(),
                default_value: None,
                optional: true,
                multiple: false,
                entry_data_class: ClassName("addon.CustomSectionEntryData".to_owned()),
                kind: EntryKind::Unknown,
                separator: None,
                value_type: None,
                string_mode: None,
                return_types: Vec::new(),
                flags: None,
                nested_validator: None,
            }],
        }),
    )]);
    let document = parse(&catalog, "root:\n    custom:\n        body\n");
    let candidate = selected(&document);

    let StructureBody::Entries(entries) = &candidate.body else {
        panic!("validator-backed Section must expose unknown entries");
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].source, "custom");
    assert!(matches!(entries[0].value, StructureEntryValue::Unknown(_)));
    assert!(
        document
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == StructureDiagnosticKind::UnknownEntryData)
    );
}

#[test]
fn missing_required_entry_points_to_body_end_zero_width_span() {
    let text = "config:\n";
    let catalog = catalog(vec![structure(
        0,
        "config",
        NodeType::Section,
        Some(EntryValidator {
            entry_data: vec![key_value("required", false, false)],
        }),
    )]);
    let document = parse(&catalog, text);
    let diagnostic = document
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.kind == StructureDiagnosticKind::MissingRequiredEntry)
        .expect("required entry diagnostic must be present");

    assert_eq!(diagnostic.span.local_range, TextRange::empty(text.len()));
}

#[test]
fn only_raw_tree_roots_are_structures_not_nested_nodes() {
    let catalog = catalog(vec![
        structure(0, "root", NodeType::Section, None),
        structure(1, "child", NodeType::Section, None),
    ]);
    let document = parse(&catalog, "root:\n    child:\n        body\n");

    assert_eq!(document.roots.len(), 1);
    let candidate = selected(&document);
    let StructureBody::Raw(children) = &candidate.body else {
        panic!("a Structure without an EntryValidator retains its raw body");
    };
    assert_eq!(children.len(), 1);
    assert!(
        document
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.kind != StructureDiagnosticKind::Unclaimed)
    );
}

#[test]
fn dynamic_structure_candidate_defaults_to_both_and_retains_handler_metadata() {
    let catalog = catalog(Vec::new());
    let id = DynamicSyntaxId::new("test.addon", "dynamic-structure");
    let pattern = "dynamic root";
    let definition = DynamicSyntaxDefinition {
        id: id.clone(),
        kind: SyntaxKind::Structure,
        patterns: vec![DynamicPattern {
            source: pattern.to_owned(),
            parsed: syntax::parse(pattern, catalog.plural_rules()).expect("pattern must parse"),
        }],
        priority: 0,
        before: Vec::new(),
        after: Vec::new(),
        return_type: None,
        return_multiplicity: None,
        structure_node_type: None,
        structure_body_mode: None,
        entry_validator: None,
        handler: "test.addon.structure".to_owned(),
        metadata: BTreeMap::from([("mode".to_owned(), "wasm".to_owned())]),
        component_load_order: 0,
        declaration_order: 0,
    };
    let snapshot = DynamicSyntaxSnapshot {
        document_id: "file:///structure.sk".to_owned(),
        document_revision: 1,
        registry_revision: 1,
        definitions: BTreeMap::from([(id.clone(), definition)]),
        overrides: BTreeMap::new(),
        candidates: vec![RankedSyntaxCandidate {
            source: SyntaxCandidateSource::Dynamic(id),
            kind: SyntaxKind::Structure,
            overrides: Vec::new(),
        }],
    };
    let source = MappedSource::identity("dynamic root:\n    body\n");
    let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
    let document = parse_structures_with_snapshot(
        &catalog,
        Some(&snapshot),
        StructureParseRequest {
            source: &source,
            tree: &tree,
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        StructureParserConfig::default(),
    )
    .expect("dynamic Structure must parse");
    let candidate = selected(&document);

    assert_eq!(candidate.declared_node_type, NodeType::Both);
    assert_eq!(candidate.actual_node_type, NodeType::Section);
    assert_eq!(candidate.handler.as_deref(), Some("test.addon.structure"));
    assert_eq!(
        candidate.metadata.get("mode").map(String::as_str),
        Some("wasm")
    );
}

#[test]
fn dynamic_structure_uses_explicit_node_type_body_mode_and_entry_validator() {
    let catalog = catalog(Vec::new());
    let id = DynamicSyntaxId::new("test.addon", "validated-structure");
    let pattern = "validated root";
    let definition = DynamicSyntaxDefinition {
        id: id.clone(),
        kind: SyntaxKind::Structure,
        patterns: vec![DynamicPattern {
            source: pattern.to_owned(),
            parsed: syntax::parse(pattern, catalog.plural_rules()).expect("pattern must parse"),
        }],
        priority: 0,
        before: Vec::new(),
        after: Vec::new(),
        return_type: None,
        return_multiplicity: None,
        structure_node_type: Some(NodeType::Section),
        structure_body_mode: Some(DynamicStructureBodyMode::Entries),
        entry_validator: Some(EntryValidator {
            entry_data: vec![key_value("name", false, false)],
        }),
        handler: "test.addon.validated-structure".to_owned(),
        metadata: BTreeMap::new(),
        component_load_order: 0,
        declaration_order: 0,
    };
    let snapshot = DynamicSyntaxSnapshot {
        document_id: "file:///validated.sk".to_owned(),
        document_revision: 1,
        registry_revision: 1,
        definitions: BTreeMap::from([(id.clone(), definition)]),
        overrides: BTreeMap::new(),
        candidates: vec![RankedSyntaxCandidate {
            source: SyntaxCandidateSource::Dynamic(id),
            kind: SyntaxKind::Structure,
            overrides: Vec::new(),
        }],
    };
    let source = MappedSource::identity("validated root:\n    name: value\n");
    let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
    let document = parse_structures_with_snapshot(
        &catalog,
        Some(&snapshot),
        StructureParseRequest {
            source: &source,
            tree: &tree,
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        StructureParserConfig::default(),
    )
    .expect("dynamic Structure must parse");
    let candidate = selected(&document);

    assert_eq!(candidate.declared_node_type, NodeType::Section);
    assert_eq!(candidate.actual_node_type, NodeType::Section);
    let StructureBody::Entries(entries) = &candidate.body else {
        panic!("explicit Entries body mode must use the supplied validator");
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].key, "name");
    assert_eq!(entries[0].source, "name: value");
}

#[test]
fn dynamic_structure_infers_entries_body_from_an_omitted_body_mode() {
    let catalog = catalog(Vec::new());
    let id = DynamicSyntaxId::new("test.addon", "inferred-entries");
    let pattern = "inferred entries";
    let definition = DynamicSyntaxDefinition {
        id: id.clone(),
        kind: SyntaxKind::Structure,
        patterns: vec![DynamicPattern {
            source: pattern.to_owned(),
            parsed: syntax::parse(pattern, catalog.plural_rules()).expect("pattern must parse"),
        }],
        priority: 0,
        before: Vec::new(),
        after: Vec::new(),
        return_type: None,
        return_multiplicity: None,
        structure_node_type: Some(NodeType::Section),
        structure_body_mode: None,
        entry_validator: Some(EntryValidator {
            entry_data: vec![key_value("name", false, false)],
        }),
        handler: "test.addon.inferred-entries".to_owned(),
        metadata: BTreeMap::new(),
        component_load_order: 0,
        declaration_order: 0,
    };
    let snapshot = DynamicSyntaxSnapshot {
        document_id: "file:///inferred-entries.sk".to_owned(),
        document_revision: 1,
        registry_revision: 1,
        definitions: BTreeMap::from([(id.clone(), definition)]),
        overrides: BTreeMap::new(),
        candidates: vec![RankedSyntaxCandidate {
            source: SyntaxCandidateSource::Dynamic(id),
            kind: SyntaxKind::Structure,
            overrides: Vec::new(),
        }],
    };
    let source = MappedSource::identity("inferred entries:\n    name: value\n");
    let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
    let document = parse_structures_with_snapshot(
        &catalog,
        Some(&snapshot),
        StructureParseRequest {
            source: &source,
            tree: &tree,
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        StructureParserConfig::default(),
    )
    .expect("dynamic Structure must parse");

    let candidate = selected(&document);
    let StructureBody::Entries(entries) = &candidate.body else {
        panic!("an omitted body mode must infer Entries from the validator");
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].key, "name");
}

#[test]
fn dynamic_structure_node_type_filters_before_header_matching() {
    let catalog = catalog(Vec::new());
    let id = DynamicSyntaxId::new("test.addon", "section-only");
    let pattern = "section-only";
    let definition = DynamicSyntaxDefinition {
        id: id.clone(),
        kind: SyntaxKind::Structure,
        patterns: vec![DynamicPattern {
            source: pattern.to_owned(),
            parsed: syntax::parse(pattern, catalog.plural_rules()).expect("pattern must parse"),
        }],
        priority: 0,
        before: Vec::new(),
        after: Vec::new(),
        return_type: None,
        return_multiplicity: None,
        structure_node_type: Some(NodeType::Section),
        structure_body_mode: Some(DynamicStructureBodyMode::Raw),
        entry_validator: None,
        handler: "test.addon.section-only".to_owned(),
        metadata: BTreeMap::new(),
        component_load_order: 0,
        declaration_order: 0,
    };
    let snapshot = DynamicSyntaxSnapshot {
        document_id: "file:///section-only.sk".to_owned(),
        document_revision: 1,
        registry_revision: 1,
        definitions: BTreeMap::from([(id.clone(), definition)]),
        overrides: BTreeMap::new(),
        candidates: vec![RankedSyntaxCandidate {
            source: SyntaxCandidateSource::Dynamic(id),
            kind: SyntaxKind::Structure,
            overrides: Vec::new(),
        }],
    };

    let simple_source = MappedSource::identity("section-only\n");
    let simple_tree = parse_raw_tree(&simple_source, RawTreeOptions::for_skript_version(2, 15));
    let simple = parse_structures_with_snapshot(
        &catalog,
        Some(&snapshot),
        StructureParseRequest {
            source: &simple_source,
            tree: &simple_tree,
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        StructureParserConfig::default(),
    )
    .expect("simple Structure parse must recover");
    assert!(matches!(
        &simple.roots[0],
        StructureDocumentNode::Structure(matches) if matches.selected.is_none()
    ));

    let section_source = MappedSource::identity("section-only:\n");
    let section_tree = parse_raw_tree(&section_source, RawTreeOptions::for_skript_version(2, 15));
    let section = parse_structures_with_snapshot(
        &catalog,
        Some(&snapshot),
        StructureParseRequest {
            source: &section_source,
            tree: &section_tree,
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        StructureParserConfig::default(),
    )
    .expect("section Structure parse must succeed");
    assert_eq!(selected(&section).actual_node_type, NodeType::Section);
}

#[test]
fn unknown_addon_entry_data_keeps_raw_value_without_rejecting_structure() {
    let unknown = EntryData {
        key: "custom".to_owned(),
        default_value: None,
        optional: true,
        multiple: false,
        entry_data_class: ClassName("addon.CustomEntryData".to_owned()),
        kind: EntryKind::Unknown,
        separator: Some(": ".to_owned()),
        value_type: None,
        string_mode: None,
        return_types: Vec::new(),
        flags: None,
        nested_validator: None,
    };
    let catalog = catalog(vec![structure(
        0,
        "root",
        NodeType::Section,
        Some(EntryValidator {
            entry_data: vec![unknown],
        }),
    )]);
    let document = parse(&catalog, "root:\n    custom: preserved by addon\n");
    let candidate = selected(&document);

    let StructureBody::Entries(entries) = &candidate.body else {
        panic!("unknown EntryData must not change the selected Structure body");
    };
    assert_eq!(entries.len(), 1);
    assert!(matches!(
        entries[0].value,
        StructureEntryValue::Unknown(ref value) if value == "preserved by addon"
    ));
    assert!(
        document
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == StructureDiagnosticKind::UnknownEntryData)
    );
}

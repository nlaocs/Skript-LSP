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

#[path = "structure/body_reuse.rs"]
mod body_reuse;

#[test]
fn entry_declaration_identity_and_key_ranges_survive_duplicates_nesting_and_defaults() {
    let mut first = key_value("na\u{ef}ve", false, false);
    first.entry_data_class = ClassName("test.First".into());
    let mut second = first.clone();
    second.entry_data_class = ClassName("test.Second".into());
    let nested = EntryValidator {
        entry_data: vec![key_value("na\u{ef}ve", false, false)],
    };
    let mut default = key_value("implicit", true, false);
    default.default_value = Some(json!("fallback"));
    let validator = EntryValidator {
        entry_data: vec![first, second, container("settings", nested), default],
    };
    let catalog = catalog(vec![structure(
        0,
        "root",
        NodeType::Section,
        Some(validator),
    )]);
    let source = "root:\r\n\tNA\u{ef}VE: first # note\r\n\tna\u{ef}ve: second\r\n\tsettings:\r\n\t\tna\u{ef}ve: nested\r\n";
    let document = parse(&catalog, source);
    assert!(
        document.diagnostics.is_empty(),
        "{:?}",
        document.diagnostics
    );
    let StructureBody::Entries(entries) = &selected(&document).body else {
        panic!("entries")
    };
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.declaration_index)
            .collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    for (entry, expected) in entries[..3]
        .iter()
        .zip(["NA\u{ef}VE", "na\u{ef}ve", "settings"])
    {
        let span = entry.key_span.as_ref().unwrap();
        assert_eq!(span.mapped.origins.len(), 1);
        assert_eq!(
            span.mapped.origins[0].kind,
            skript_parser::OriginKind::Exact
        );
        assert_eq!(
            span.mapped.origins[0].original_range.slice(source),
            Some(expected)
        );
    }
    assert_ne!(entries[0].entry_data_class, entries[1].entry_data_class);
    let StructureEntryValue::Container(children) = &entries[2].value else {
        panic!("container")
    };
    assert_eq!(children[0].declaration_index, 0);
    assert_eq!(
        children[0].key_span.as_ref().unwrap().mapped.origins[0]
            .original_range
            .slice(source),
        Some("na\u{ef}ve")
    );
    assert!(entries[3].defaulted);
    assert!(entries[3].raw_node_id.is_none());
    assert!(entries[3].key_span.is_none());
}

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
            let mut metadata=request.candidate.metadata.clone();
            metadata.insert("accepted-exit".into(),request.candidate.matched.registration_id.to_string());
            return Ok(StructureExitDecision::Accept { metadata });
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

#[test]
fn headers_only_stops_after_the_enter_hook() {
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
        StructureParserConfig {
            headers_only: true,
            ..StructureParserConfig::default()
        },
    )
    .expect("header-only parsing must not invoke the rejecting exit hook");

    let StructureDocumentNode::Structure(matches) = &document.roots[0] else {
        panic!("Structure result expected");
    };
    let selected = matches
        .selected
        .as_ref()
        .expect("the accepted header must remain selected");
    assert!(matches!(selected.body, skript_parser::StructureBody::None));
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
    assert_eq!(selected.metadata.get("accepted-exit").map(String::as_str),Some("registration:fallback"));
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

mod extension_fault_recovery {
    use super::*;
    use skript_parser::*;

    #[derive(Default)]
    struct FaultEnvironment {
        fault_at: &'static str,
        capture_owner: Option<SyntaxKind>,
        capture_parser: &'static str,
        pending: Option<ExtensionFault>,
        frames: Vec<(bool, Vec<String>, Vec<String>)>,
        writes: Vec<String>,
        declarations: Vec<String>,
        failed_capture: Option<String>,
        capture_id: usize,
        faulted: bool,
        observed: Vec<ExtensionFault>,
    }

    impl FaultEnvironment {
        fn fault(&mut self, phase: &str) -> String {
            self.faulted = true;
            self.failed_capture = self.writes.last().cloned();
            self.pending = Some(ExtensionFault {
                component_id: "test.injected".into(),
                subscription_id: "test.fault".into(),
                phase: phase.into(),
                target: "candidate".into(),
                code: "parser.extension-trap".into(),
                message: "controlled fault".into(),
            });
            "controlled fault".into()
        }
    }

    impl PatternMatchEnvironment for FaultEnvironment {
        fn take_extension_fault(&mut self) -> Option<ExtensionFault> {
            self.pending.take()
        }
        fn resolve_type(
            &mut self,
            _: TypeExpressionRequest<'_>,
        ) -> Result<TypeExpressionOutcome, String> {
            Ok(TypeExpressionOutcome::default())
        }
        fn dispatch_hook(&mut self, _: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
            Ok(PatternHookControl::Continue)
        }
        fn allows_regex_pattern(
            &mut self,
            _: MatchSyntaxKind,
            _: &str,
            _: usize,
        ) -> Result<bool, String> {
            Ok(true)
        }
        fn take_pattern_candidate_state(&mut self) -> Option<ExpressionEffects> {
            self.capture_id += 1;
            Some(ExpressionEffects::new(format!(
                "capture-{}",
                self.capture_id
            )))
        }
        fn restore_pattern_candidate_state(
            &mut self,
            state: &ExpressionEffects,
        ) -> Result<(), String> {
            let id = state.downcast_ref::<String>().unwrap().clone();
            self.writes.push(id.clone());
            self.declarations.push(id);
            if !self.faulted
                && ((self.fault_at == "activate-header" && self.capture_id == 1)
                    || (self.fault_at == "activate-child" && self.capture_id == 2))
            {
                return Err(self.fault("Matching"));
            }
            Ok(())
        }
    }

    impl ExpressionParseEnvironment for FaultEnvironment {
        fn parse_expression_leaf(
            &mut self,
            _: ExpressionLeafRequest<'_>,
        ) -> Result<ExpressionLeafParse, String> {
            Ok(ExpressionLeafParse::default())
        }
        fn state_revision(&self) -> Result<u64, String> {
            Ok(0)
        }
        fn registered_capture_bindings(
            &self,
            request: RegisteredSyntaxIdentity<'_>,
        ) -> Result<Vec<RegisteredCaptureBinding>, String> {
            Ok(
                if self.capture_owner == Some(request.kind)
                    && request
                        .pattern_source
                        .is_some_and(|pattern| pattern.contains("<.+>"))
                {
                    vec![RegisteredCaptureBinding {
                        capture_index: 0,
                        parser_id: self.capture_parser.into(),
                        required: true,
                        options: BTreeMap::new(),
                    }]
                } else {
                    Vec::new()
                },
            )
        }
        fn begin_semantic_candidate(&mut self) -> Result<(), String> {
            self.frames
                .push((false, self.writes.clone(), self.declarations.clone()));
            Ok(())
        }
        fn finish_semantic_candidate(&mut self, accepted: bool) -> Result<(), String> {
            let (expression, writes, declarations) =
                self.frames.pop().ok_or("unbalanced candidate")?;
            assert!(!expression, "semantic frame closed out of order");
            if !accepted {
                self.writes = writes;
                self.declarations = declarations;
            }
            Ok(())
        }
        fn begin_expression_candidate(&mut self) -> Result<(), String> {
            self.frames
                .push((true, self.writes.clone(), self.declarations.clone()));
            Ok(())
        }
        fn defer_expression_candidate(
            &mut self,
            accepted: bool,
        ) -> Result<Option<ExpressionEffects>, String> {
            let (expression, writes, declarations) =
                self.frames.pop().ok_or("unbalanced expression")?;
            assert!(expression, "expression frame closed out of order");
            if !accepted {
                self.writes = writes;
                self.declarations = declarations;
            }
            Ok(None)
        }
        fn enter_structure(
            &mut self,
            request: StructureHookRequest<'_>,
        ) -> Result<StructureHookDecision, String> {
            if self.fault_at == "header" && !self.faulted {
                return Err(self.fault("Structure"));
            }
            Ok(StructureHookDecision::Accept {
                context: request.context.clone(),
                body_mode: StructureBodyMode::Trigger,
                entry_body_modes: request.candidate.entry_body_modes.clone(),
                metadata: request.candidate.metadata.clone(),
            })
        }
        fn resolve_condition_candidate(
            &mut self,
            _: ConditionSemanticRequest<'_>,
        ) -> Result<ConditionSemanticDecision, String> {
            if self.fault_at == "condition" && !self.faulted {
                return Err(self.fault("Condition"));
            }
            if self.fault_at == "fatal-condition" {
                return Err("ordinary internal failure".into());
            }
            Ok(ConditionSemanticDecision::UseCandidate)
        }
        fn resolve_effect_candidate(
            &mut self,
            _: EffectSemanticRequest<'_>,
        ) -> Result<EffectSemanticDecision, String> {
            if self.fault_at == "effect" && !self.faulted {
                return Err(self.fault("Effect"));
            }
            Ok(EffectSemanticDecision::UseCandidate)
        }
        fn observe_extension_fault(&mut self, fault: &ExtensionFault, _: MappedSpan) {
            self.observed.push(fault.clone());
        }
    }

    fn fixture_syntaxes() -> Vec<Syntax> {
        vec![
            structure(0, "root", NodeType::Section, None),
            structure(1, "probe <.+>", NodeType::Section, None),
            Syntax::Condition(syntaxes::Condition {
                common: common_structure(2, "condition", "condition", "condition"),
            }),
            Syntax::Effect(syntaxes::Effect {
                common: common_structure(3, "effect", "effect", "effect"),
            }),
            Syntax::Section(syntaxes::Section {
                common: common_structure(4, "section", "section", "section <.+>"),
                loop_section: false,
                effect_section: false,
            }),
        ]
    }
    fn fixture() -> Catalog {
        catalog(fixture_syntaxes())
    }

    fn run(
        source: &str,
        environment: &mut FaultEnvironment,
    ) -> Result<StructureDocument, StructureParseError> {
        let source = MappedSource::identity(source);
        let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
        parse_structures(
            &fixture(),
            StructureParseRequest {
                source: &source,
                tree: &tree,
                context: ExpressionParseContext::default(),
            },
            environment,
            StructureParserConfig::default(),
        )
    }

    fn assert_recovered(environment: &FaultEnvironment, expected_writes: usize) {
        assert!(
            environment.frames.is_empty(),
            "candidate frames remain open"
        );
        assert_eq!(
            environment.writes.len(),
            expected_writes,
            "{:?}",
            environment.writes
        );
        assert_eq!(environment.declarations, environment.writes);
        assert!(
            !environment
                .writes
                .contains(environment.failed_capture.as_ref().expect("fault ran"))
        );
        assert_eq!(environment.observed.len(), 1);
        assert_eq!(environment.observed[0].code, "parser.extension-trap");
    }

    #[test]
    fn condition_fault_discards_applied_capture_before_independent_sibling() {
        let mut env = FaultEnvironment {
            fault_at: "condition",
            ..Default::default()
        };
        let document = run("root:\n    condition\n    condition\n", &mut env).unwrap();
        assert_recovered(&env, 2);
        let StructureDocumentNode::Structure(root) = &document.roots[0] else {
            panic!()
        };
        let StructureBody::Trigger(body) = &root.selected.as_ref().unwrap().body else {
            panic!()
        };
        assert!(matches!(body[0], SectionBodyNode::Unclaimed(_)));
        assert!(
            matches!(&body[1], SectionBodyNode::Condition { matches, .. } if matches.selected.is_some())
        );
    }

    #[test]
    fn effect_fault_and_activation_fault_discard_capture_before_independent_sibling() {
        for fault_at in ["effect", "activate-child"] {
            let mut env = FaultEnvironment {
                fault_at,
                ..Default::default()
            };
            let document = run("root:\n    effect\n    effect\n", &mut env).unwrap();
            assert_recovered(&env, 2);
            let StructureDocumentNode::Structure(root) = &document.roots[0] else {
                panic!()
            };
            let StructureBody::Trigger(body) = &root.selected.as_ref().unwrap().body else {
                panic!()
            };
            assert!(matches!(body[0], SectionBodyNode::Unclaimed(_)));
            assert!(
                matches!(&body[1], SectionBodyNode::Effect(matches) if matches.selected.is_some())
            );
        }
    }

    #[test]
    fn header_fault_and_activation_fault_discard_capture_before_independent_root() {
        for fault_at in ["header", "activate-header"] {
            let mut env = FaultEnvironment {
                fault_at,
                ..Default::default()
            };
            let document = run("root:\n    # first\nroot:\n    # later\n", &mut env).unwrap();
            assert_recovered(&env, 1);
            assert!(matches!(
                document.roots[0],
                StructureDocumentNode::Unclaimed(_)
            ));
            assert!(
                matches!(&document.roots[1], StructureDocumentNode::Structure(matches) if matches.selected.is_some())
            );
        }
    }

    #[test]
    fn structure_condition_and_effect_capture_faults_keep_cause_and_later_root() {
        for (capture_parser, fault_at, text) in [
            (HOST_CONDITION_PARSER_ID, "condition", "condition"),
            (HOST_EFFECT_PARSER_ID, "effect", "effect"),
        ] {
            let mut env = FaultEnvironment {
                fault_at,
                capture_owner: Some(SyntaxKind::Structure),
                capture_parser,
                ..Default::default()
            };
            let document = run(
                &format!("probe {text}:\n    # first\nroot:\n    # later\n"),
                &mut env,
            )
            .unwrap();
            assert_recovered(&env, 1);
            assert!(matches!(
                document.roots[0],
                StructureDocumentNode::Unclaimed(_)
            ));
            assert!(
                matches!(&document.roots[1], StructureDocumentNode::Structure(matches) if matches.selected.is_some())
            );
            assert!(
                document
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.kind == StructureDiagnosticKind::ExtensionFault)
            );
        }
    }

    #[test]
    fn section_condition_and_effect_capture_faults_discard_nested_state() {
        for (capture_parser, fault_at, text) in [
            (HOST_CONDITION_PARSER_ID, "condition", "condition"),
            (HOST_EFFECT_PARSER_ID, "effect", "effect"),
        ] {
            let mut env = FaultEnvironment {
                fault_at,
                capture_owner: Some(SyntaxKind::Section),
                capture_parser,
                ..Default::default()
            };
            let document = run(
                &format!("root:\n    section {text}:\n        # first\n    effect\n"),
                &mut env,
            )
            .unwrap();
            assert_recovered(&env, 2);
            let StructureDocumentNode::Structure(root) = &document.roots[0] else {
                panic!()
            };
            let StructureBody::Trigger(body) = &root.selected.as_ref().unwrap().body else {
                panic!()
            };
            assert!(matches!(body[0], SectionBodyNode::Unclaimed(_)));
            assert!(
                matches!(&body[1], SectionBodyNode::Effect(matches) if matches.selected.is_some())
            );
        }
    }

    #[test]
    fn ordinary_internal_capture_error_remains_fatal_and_closes_scopes() {
        let mut env = FaultEnvironment {
            fault_at: "fatal-condition",
            capture_owner: Some(SyntaxKind::Structure),
            capture_parser: HOST_CONDITION_PARSER_ID,
            ..Default::default()
        };
        let error = run(
            "probe condition:\n    # first\nroot:\n    # later\n",
            &mut env,
        )
        .unwrap_err();
        assert!(extension_fault(&error).is_none());
        assert!(matches!(error, StructureParseError::Body { .. }));
        assert!(env.frames.is_empty());
        assert!(env.writes.is_empty());
        assert!(env.observed.is_empty());
    }

    #[test]
    fn registered_expression_capture_fault_unwinds_expression_and_semantic_frames() {
        let source_catalog = ssg::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4"),
        )
        .unwrap()
        .into_catalog();
        let mut expression = source_catalog.expressions().next().unwrap().clone();
        expression.common = common_structure(
            5,
            "nested-expression",
            "nested-expression",
            "expression <.+>",
        );
        expression.return_type = Some(ClassName("java.lang.Object".into()));
        expression.return_type_state = syntaxes::ReturnTypeState::Static;
        expression.possible_return_types = vec![ClassName("java.lang.Object".into())];
        expression.possible_return_types_state = syntaxes::PossibleReturnTypesState::Complete;
        expression.return_type_multiplicity = Some(syntaxes::Multiplicity::Single);
        expression.return_type_multiplicity_state = syntaxes::ResolutionState::Resolved;
        let mut syntaxes = fixture_syntaxes();
        syntaxes.extend(source_catalog.types().cloned().map(Syntax::Type));
        syntaxes.push(Syntax::Expression(expression));
        syntaxes.push(Syntax::Condition(syntaxes::Condition {
            common: common_structure(6, "check", "check", "check %object%"),
        }));
        let catalog = Catalog::new(CatalogParts {
            syntaxes,
            converters: Vec::new(),
            comparators: Vec::new(),
            event_values: Vec::new(),
            properties: Vec::new(),
            operators: Vec::new(),
            operations: BTreeMap::new(),
            differences: Vec::new(),
            classes: source_catalog.classes().to_vec(),
            aliases: AliasRegistry {
                aliases: BTreeMap::new(),
                targets: Vec::new(),
            },
            plural_rules: plural_rules(),
            language: BTreeMap::new(),
        });
        let source = MappedSource::identity("root:\n    check expression condition\n    effect\n");
        let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
        let mut env = FaultEnvironment {
            fault_at: "condition",
            capture_owner: Some(SyntaxKind::Expression),
            capture_parser: HOST_CONDITION_PARSER_ID,
            ..Default::default()
        };
        let document = parse_structures(
            &catalog,
            StructureParseRequest {
                source: &source,
                tree: &tree,
                context: ExpressionParseContext::default(),
            },
            &mut env,
            StructureParserConfig::default(),
        )
        .unwrap();
        assert_recovered(&env, 2);
        let StructureDocumentNode::Structure(root) = &document.roots[0] else {
            panic!()
        };
        let StructureBody::Trigger(body) = &root.selected.as_ref().unwrap().body else {
            panic!()
        };
        assert!(matches!(body[0], SectionBodyNode::Unclaimed(_)));
        assert!(matches!(&body[1], SectionBodyNode::Effect(matches) if matches.selected.is_some()));
    }
}

struct EntryBodyEnvironment {
    path: Vec<String>,
}
impl PatternMatchEnvironment for EntryBodyEnvironment {
    fn resolve_type(
        &mut self,
        _: TypeExpressionRequest<'_>,
    ) -> Result<TypeExpressionOutcome, String> {
        Ok(TypeExpressionOutcome::default())
    }
    fn dispatch_hook(&mut self, _: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
        Ok(PatternHookControl::Continue)
    }
}
impl ExpressionParseEnvironment for EntryBodyEnvironment {
    fn state_revision(&self) -> Result<u64, String> {
        Ok(0)
    }
    fn parse_expression_leaf(
        &mut self,
        _: ExpressionLeafRequest<'_>,
    ) -> Result<ExpressionLeafParse, String> {
        Ok(ExpressionLeafParse::default())
    }
    fn enter_structure(
        &mut self,
        request: StructureHookRequest<'_>,
    ) -> Result<skript_parser::StructureHookDecision, String> {
        Ok(skript_parser::StructureHookDecision::Accept {
            context: request.context.clone(),
            body_mode: request.default_body_mode,
            entry_body_modes: BTreeMap::from([(
                self.path.clone(),
                skript_parser::SectionBodyMode::Trigger,
            )]),
            metadata: request.candidate.metadata.clone(),
        })
    }
}

#[test]
fn semantic_entry_body_modes_follow_declarations_and_preserve_source_identity() {
    let mut code = key_value("code", true, false);
    code.kind = EntryKind::Section;
    let mut raw = code.clone();
    raw.key = "data".into();
    let validator = EntryValidator {
        entry_data: vec![container(
            "settings",
            EntryValidator {
                entry_data: vec![code, raw],
            },
        )],
    };
    let catalog = catalog(vec![structure(
        0,
        "config",
        NodeType::Section,
        Some(validator.clone()),
    )]);
    let source = MappedSource::identity(
        "config:\n    settings:\n        code:\n            invalid statement\n        data:\n            retained value\n",
    );
    let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
    let document = parse_structures(
        &catalog,
        StructureParseRequest {
            source: &source,
            tree: &tree,
            context: ExpressionParseContext::default(),
        },
        &mut EntryBodyEnvironment {
            path: vec!["settings".into(), "code".into()],
        },
        StructureParserConfig::default(),
    )
    .unwrap();
    let candidate = selected(&document);
    assert_eq!(candidate.entry_validator.as_ref(), Some(&validator));
    let StructureBody::Entries(entries) = &candidate.body else {
        panic!("entries required")
    };
    let StructureEntryValue::Container(children) = &entries[0].value else {
        panic!("container required")
    };
    let StructureEntryValue::Trigger(body) = &children[0].value else {
        panic!("owner selected executable body")
    };
    assert!(
        matches!(&body[0], skript_parser::SectionBodyNode::Effect(matches) if matches.unknown.is_some())
    );
    assert!(matches!(&children[1].value, StructureEntryValue::Section(ids) if ids.len() == 1));
    assert_eq!(children[0].kind, EntryKind::Section);
    assert_eq!(
        children[0]
            .span
            .mapped
            .virtual_range
            .slice(source.virtual_source()),
        Some("code:")
    );
    let diagnostic = document
        .diagnostics
        .iter()
        .find(|d| d.kind == StructureDiagnosticKind::Unclaimed)
        .unwrap();
    assert_eq!(
        diagnostic
            .span
            .mapped
            .virtual_range
            .slice(source.virtual_source()),
        Some("invalid statement")
    );
    assert_eq!(
        diagnostic.span.mapped.origins[0].original_range,
        diagnostic.span.mapped.virtual_range
    );
    let mut unreachable_validator = validator.clone();
    unreachable_validator.entry_data[0].kind = EntryKind::Section;
    let unreachable_catalog = self::catalog(vec![structure(
        0,
        "config",
        NodeType::Section,
        Some(unreachable_validator),
    )]);
    assert!(
        parse_structures(
            &unreachable_catalog,
            StructureParseRequest {
                source: &source,
                tree: &tree,
                context: ExpressionParseContext::default()
            },
            &mut EntryBodyEnvironment {
                path: vec!["settings".into(), "code".into()]
            },
            StructureParserConfig::default()
        )
        .is_err()
    );
    let mut legacy_validator = validator.clone();
    legacy_validator.entry_data[0]
        .nested_validator
        .as_mut()
        .unwrap()
        .entry_data[0]
        .kind = EntryKind::Trigger;
    let legacy_catalog = self::catalog(vec![structure(
        0,
        "config",
        NodeType::Section,
        Some(legacy_validator),
    )]);
    let legacy = parse_structures(
        &legacy_catalog,
        StructureParseRequest {
            source: &source,
            tree: &tree,
            context: ExpressionParseContext::default(),
        },
        &mut EntryBodyEnvironment {
            path: vec!["settings".into(), "code".into()],
        },
        StructureParserConfig::default(),
    )
    .unwrap();
    selected(&legacy);
    for path in [vec!["missing".into()], vec!["settings".into()], Vec::new()] {
        assert!(
            parse_structures(
                &catalog,
                StructureParseRequest {
                    source: &source,
                    tree: &tree,
                    context: ExpressionParseContext::default()
                },
                &mut EntryBodyEnvironment { path },
                StructureParserConfig::default()
            )
            .is_err(),
            "undeclared/non-Section paths must fail"
        );
    }
}

#[test]
fn later_structure_fault_does_not_revive_an_entry_rejection() {
    use skript_parser::*;
    struct Environment {
        pending: Option<ExtensionFault>,
    }
    impl PatternMatchEnvironment for Environment {
        fn resolve_type(
            &mut self,
            _: TypeExpressionRequest<'_>,
        ) -> Result<TypeExpressionOutcome, String> {
            Ok(TypeExpressionOutcome::default())
        }
        fn dispatch_hook(&mut self, _: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
            Ok(PatternHookControl::Continue)
        }
        fn take_extension_fault(&mut self) -> Option<ExtensionFault> {
            self.pending.take()
        }
    }
    impl ExpressionParseEnvironment for Environment {
        fn state_revision(&self) -> Result<u64, String> {
            Ok(0)
        }
        fn parse_expression_leaf(
            &mut self,
            _: ExpressionLeafRequest<'_>,
        ) -> Result<ExpressionLeafParse, String> {
            Ok(ExpressionLeafParse::default())
        }
        fn enter_structure(
            &mut self,
            request: StructureHookRequest<'_>,
        ) -> Result<StructureHookDecision, String> {
            Ok(StructureHookDecision::Accept {
                context: request.context.clone(),
                body_mode: request.default_body_mode,
                entry_body_modes: BTreeMap::new(),
                metadata: request.candidate.metadata.clone(),
            })
        }
        fn exit_structure(
            &mut self,
            _: StructureHookRequest<'_>,
        ) -> Result<StructureExitDecision, String> {
            self.pending = Some(ExtensionFault {
                component_id: "fixture.owner".into(),
                subscription_id: "exit".into(),
                phase: "Structure".into(),
                target: "candidate".into(),
                code: "parser.extension-trap".into(),
                message: "injected".into(),
            });
            Err("injected".into())
        }
    }
    let first = structure(
        0,
        "config",
        NodeType::Section,
        Some(EntryValidator {
            entry_data: vec![key_value("required", false, false)],
        }),
    );
    let mut second = structure(1, "config", NodeType::Section, None);
    let Syntax::Structure(other) = &mut second else {
        panic!()
    };
    other.common.registration_id = RegistrationId("registration:fallback".into());
    other.common.definition_id = DefinitionId("definition:fallback".into());
    let catalog = catalog(vec![first, second]);
    let source = MappedSource::identity("config:\n    arbitrary: value\n");
    let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
    let document = parse_structures(
        &catalog,
        StructureParseRequest {
            source: &source,
            tree: &tree,
            context: ExpressionParseContext::default(),
        },
        &mut Environment { pending: None },
        StructureParserConfig::default(),
    )
    .unwrap();
    assert!(
        document
            .diagnostics
            .iter()
            .any(|d| d.kind == StructureDiagnosticKind::ExtensionFault)
    );
    let StructureDocumentNode::Structure(matches) = &document.roots[0] else {
        panic!()
    };
    assert!(matches.selected.is_none());
    assert!(
        matches
            .unknown
            .as_ref()
            .and_then(|node| node.partial.as_ref())
            .is_none()
    );
}

mod editing_preparation {
    use super::*;
    use skript_parser::{
        RawNodeId, SectionBodyMode, SectionChildrenDecision, SectionChildrenRequest,
        SectionExitDecision, StructureBodyMode, StructureHookDecision, StructurePreparationEvent,
    };

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Notice {
        Root(u64, RawNodeId, bool),
        Section {
            attempt: u64,
            parent: Option<u64>,
            node: RawNodeId,
            registration: String,
            mode: SectionBodyMode,
        },
        Invalidated(u64),
        Effect,
        Condition,
    }

    #[derive(Default)]
    struct Environment {
        notices: Vec<Notice>,
        conditions: bool,
        reject_root: bool,
        reject_outer_section: bool,
        cancel_after_section: bool,
        cancelled: bool,
        fail_finish_after_section: bool,
        finish_error_pending: bool,
        mixed_section_modes: bool,
        first_section_conditions: bool,
        reject_first_section: bool,
        entered_modes: Vec<(String, SectionBodyMode)>,
    }

    impl PatternMatchEnvironment for Environment {
        fn resolve_type(
            &mut self,
            _: TypeExpressionRequest<'_>,
        ) -> Result<TypeExpressionOutcome, String> {
            Ok(TypeExpressionOutcome::default())
        }
        fn dispatch_hook(&mut self, _: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
            Ok(PatternHookControl::Continue)
        }
        fn cancellation_requested(&mut self) -> bool {
            self.cancelled
        }
    }

    impl ExpressionParseEnvironment for Environment {
        fn state_revision(&self) -> Result<u64, String> {
            Ok(0)
        }
        fn parse_expression_leaf(
            &mut self,
            _: ExpressionLeafRequest<'_>,
        ) -> Result<ExpressionLeafParse, String> {
            Ok(ExpressionLeafParse::default())
        }
        fn enter_structure(
            &mut self,
            request: StructureHookRequest<'_>,
        ) -> Result<StructureHookDecision, String> {
            Ok(StructureHookDecision::Accept {
                context: request.context.clone(),
                body_mode: StructureBodyMode::Trigger,
                entry_body_modes: BTreeMap::new(),
                metadata: request.candidate.metadata.clone(),
            })
        }
        fn exit_structure(
            &mut self,
            request: StructureHookRequest<'_>,
        ) -> Result<StructureExitDecision, String> {
            if self.reject_root && request.candidate.matched.registration_id == "registration:first"
            {
                return Ok(StructureExitDecision::Reject {
                    reason: "retry root".into(),
                    diagnostics: Vec::new(),
                });
            }
            Ok(StructureExitDecision::Accept { metadata: request.candidate.metadata.clone() })
        }
        fn enter_section_children(
            &mut self,
            request: SectionChildrenRequest<'_>,
        ) -> Result<SectionChildrenDecision, String> {
            let conditions =
                if self.mixed_section_modes && request.registration_id.starts_with("gate:") {
                    (request.registration_id == "gate:first") == self.first_section_conditions
                } else {
                    self.conditions
                };
            let body_mode = if conditions {
                SectionBodyMode::Conditions
            } else {
                SectionBodyMode::Trigger
            };
            self.entered_modes
                .push((request.registration_id.to_owned(), body_mode));
            Ok(SectionChildrenDecision::Accept {
                context: request.context.clone(),
                body_mode,
                metadata: request.metadata.clone(),
            })
        }
        fn exit_section_children(
            &mut self,
            request: SectionChildrenRequest<'_>,
        ) -> Result<SectionExitDecision, String> {
            if (self.reject_outer_section && request.context.section_stack.len() == 1)
                || (self.reject_first_section && request.registration_id == "gate:first")
            {
                return Ok(SectionExitDecision::Reject {
                    reason: "reject outer section".into(),
                    diagnostics: Vec::new(),
                });
            }
            Ok(SectionExitDecision::Accept {
                context: request.context.clone(),
                metadata: request.metadata.clone(),
            })
        }
        fn finish_semantic_candidate(&mut self, _: bool) -> Result<(), String> {
            if std::mem::take(&mut self.finish_error_pending) {
                return Err("injected semantic finish failure".into());
            }
            Ok(())
        }
        fn resolve_effect_candidate(
            &mut self,
            _: skript_parser::EffectSemanticRequest<'_>,
        ) -> Result<skript_parser::EffectSemanticDecision, String> {
            self.notices.push(Notice::Effect);
            Ok(skript_parser::EffectSemanticDecision::UseCandidate)
        }
        fn resolve_condition_candidate(
            &mut self,
            _: skript_parser::ConditionSemanticRequest<'_>,
        ) -> Result<skript_parser::ConditionSemanticDecision, String> {
            self.notices.push(Notice::Condition);
            Ok(skript_parser::ConditionSemanticDecision::UseCandidate)
        }
        fn structure_preparation(&mut self, event: StructurePreparationEvent<'_>) {
            match event {
                StructurePreparationEvent::Headers { headers, .. } => {
                    self.notices.extend(headers.into_iter().map(|header| {
                        Notice::Root(
                            header.attempt_id,
                            header.candidate.raw_node_id,
                            header.ambiguous,
                        )
                    }));
                }
                StructurePreparationEvent::StructureHeader { header, .. } => {
                    self.notices.push(Notice::Root(
                        header.attempt_id,
                        header.candidate.raw_node_id,
                        header.ambiguous,
                    ));
                }
                StructurePreparationEvent::SectionHeader {
                    source,
                    tree,
                    candidate,
                    attempt_id,
                    parent_attempt_id,
                } => {
                    assert!(candidate.body.is_empty(), "body must still be pending");
                    let raw = tree.get(candidate.raw_node_id).unwrap();
                    assert_eq!(
                        raw.line.raw_text,
                        raw.line
                            .span
                            .virtual_range
                            .slice(source.virtual_source())
                            .unwrap()
                            .trim_end_matches('\n')
                            .trim_end_matches('\r')
                    );
                    assert_eq!(
                        candidate
                            .body_context
                            .section_stack
                            .last()
                            .unwrap()
                            .raw_node_id,
                        candidate.raw_node_id
                    );
                    self.notices.push(Notice::Section {
                        attempt: attempt_id,
                        parent: parent_attempt_id,
                        node: candidate.raw_node_id,
                        registration: candidate.matched.registration_id.clone(),
                        mode: candidate.body_mode,
                    });
                    if self.cancel_after_section {
                        self.cancelled = true;
                    }
                    if self.fail_finish_after_section {
                        self.finish_error_pending = true;
                    }
                }
                StructurePreparationEvent::Invalidated { attempt_id, .. } => {
                    self.notices.push(Notice::Invalidated(attempt_id))
                }
            }
        }
    }

    fn syntaxes() -> Vec<Syntax> {
        vec![
            structure(0, "root", NodeType::Section, None),
            Syntax::Section(syntaxes::Section {
                common: common_structure(1, "gate", "gate", "gate"),
                loop_section: false,
                effect_section: false,
            }),
            Syntax::Effect(syntaxes::Effect {
                common: common_structure(2, "effect", "effect", "effect"),
            }),
            Syntax::Condition(syntaxes::Condition {
                common: common_structure(3, "condition", "condition", "condition"),
            }),
        ]
    }

    fn run(
        catalog: &Catalog,
        input: &str,
        environment: &mut Environment,
    ) -> Result<StructureDocument, skript_parser::StructureParseError> {
        let source = MappedSource::identity(input);
        let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
        parse_structures(
            catalog,
            StructureParseRequest {
                source: &source,
                tree: &tree,
                context: ExpressionParseContext::default(),
            },
            environment,
            StructureParserConfig::default(),
        )
    }

    #[test]
    fn nested_preparation_uses_accepted_body_semantics_before_parsing_children() {
        for conditions in [false, true] {
            let mut environment = Environment {
                conditions,
                ..Default::default()
            };
            let child = if conditions { "condition" } else { "effect" };
            run(
                &catalog(syntaxes()),
                &format!("# 日本語🙂\r\nroot:\r\n    gate:\r\n        {child}\r\n"),
                &mut environment,
            )
            .unwrap();
            let Notice::Root(root_attempt, _, false) = environment.notices[0] else {
                panic!("unique root owner")
            };
            let Notice::Section {
                attempt,
                parent: Some(parent),
                mode,
                ..
            } = environment.notices[1]
            else {
                panic!("section must precede body hooks")
            };
            assert_eq!(parent, root_attempt);
            assert_ne!(attempt, root_attempt);
            assert_eq!(
                mode,
                if conditions {
                    SectionBodyMode::Conditions
                } else {
                    SectionBodyMode::Trigger
                }
            );
            assert_eq!(
                environment.notices[2],
                if conditions {
                    Notice::Condition
                } else {
                    Notice::Effect
                }
            );
            assert!(
                !environment
                    .notices
                    .iter()
                    .any(|notice| matches!(notice, Notice::Invalidated(_)))
            );
        }
    }

    #[test]
    fn parent_rejection_and_same_pattern_retry_keep_distinct_attempts() {
        let mut syntaxes = syntaxes();
        let mut roots = alternative_structure_catalog().syntaxes().to_vec();
        syntaxes.remove(0);
        roots.extend(syntaxes);
        let mut environment = Environment {
            reject_root: true,
            ..Default::default()
        };
        run(
            &catalog(roots),
            "lifecycle structure:\n    gate:\n        effect\n",
            &mut environment,
        )
        .unwrap();
        let sections = environment
            .notices
            .iter()
            .filter_map(|notice| match notice {
                Notice::Section {
                    attempt,
                    parent,
                    node,
                    registration,
                    ..
                } => Some((*attempt, *parent, *node, registration)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].2, sections[1].2);
        assert_eq!(sections[0].3, sections[1].3);
        assert_ne!(sections[0].0, sections[1].0);
        assert_ne!(sections[0].1, sections[1].1);
        assert!(
            environment
                .notices
                .contains(&Notice::Invalidated(sections[0].1.unwrap()))
        );
        let roots = environment
            .notices
            .iter()
            .filter_map(|notice| match notice {
                Notice::Root(attempt, _, ambiguous) => Some((*attempt, *ambiguous)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(roots.len(), 2);
        assert!(roots[0].1, "initial competing roots must remain ambiguous");
        assert!(
            !roots[1].1,
            "after rejection the final root attempt is unique"
        );
    }

    #[test]
    fn rejected_parent_revokes_the_attempt_linking_accepted_descendants() {
        let mut environment = Environment {
            reject_outer_section: true,
            ..Default::default()
        };
        run(
            &catalog(syntaxes()),
            "root:\n    gate:\n        gate:\n            effect\n",
            &mut environment,
        )
        .unwrap();
        let sections = environment
            .notices
            .iter()
            .filter_map(|notice| match notice {
                Notice::Section {
                    attempt, parent, ..
                } => Some((*attempt, *parent)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            sections.len(),
            2,
            "fallback recovery must not republish descendants"
        );
        assert_eq!(sections[1].1, Some(sections[0].0));
        assert!(
            environment
                .notices
                .contains(&Notice::Invalidated(sections[0].0))
        );
    }

    #[test]
    fn unknown_parent_recovery_cannot_certify_an_accepted_descendant() {
        let mut environment = Environment::default();
        run(
            &catalog(syntaxes()),
            "root:\n    unknown parent:\n        gate:\n            effect\n",
            &mut environment,
        )
        .unwrap();
        assert!(
            !environment
                .notices
                .iter()
                .any(|notice| matches!(notice, Notice::Section { .. }))
        );
        assert!(
            environment.notices.contains(&Notice::Effect),
            "editor recovery still parses child effects"
        );
    }

    #[test]
    fn cancellation_and_semantic_finish_failure_revoke_all_published_owners() {
        for cancel in [false, true] {
            let mut environment = Environment {
                cancel_after_section: cancel,
                fail_finish_after_section: !cancel,
                ..Default::default()
            };
            let input = if cancel {
                "root:\n    gate:\n        effect\n"
            } else {
                "root:\n    gate:\n"
            };
            let result = run(&catalog(syntaxes()), input, &mut environment);
            assert!(result.is_err(), "{result:?}");
            let published = environment
                .notices
                .iter()
                .filter_map(|notice| match notice {
                    Notice::Root(id, ..) | Notice::Section { attempt: id, .. } => Some(*id),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(published.len(), 2);
            for attempt in published {
                assert!(
                    environment.notices.contains(&Notice::Invalidated(attempt)),
                    "orphaned attempt {attempt}: {:?}",
                    environment.notices
                );
            }
        }
    }

    fn competing_section_catalog() -> Catalog {
        let mut syntaxes = syntaxes();
        let Syntax::Section(first) = &mut syntaxes[1] else {
            unreachable!()
        };
        first.common.registration_id = RegistrationId("gate:first".into());
        first.common.definition_id = DefinitionId("gate:first".into());
        let mut fallback = first.clone();
        fallback.common.registration_id = RegistrationId("gate:last".into());
        fallback.common.definition_id = DefinitionId("gate:last".into());
        fallback.common.registration_order = 4;
        syntaxes.push(Syntax::Section(fallback));
        syntaxes.push(Syntax::Section(syntaxes::Section {
            common: common_structure(5, "leaf", "leaf", "leaf"),
            loop_section: false,
            effect_section: false,
        }));
        catalog(syntaxes)
    }

    #[test]
    fn competing_section_body_modes_do_not_certify_a_parent_or_its_descendants() {
        let mut environment = Environment {
            mixed_section_modes: true,
            ..Default::default()
        };
        let result = run(
            &competing_section_catalog(),
            "root:\n    gate:\n        leaf:\n            effect\n",
            &mut environment,
        )
        .unwrap();
        assert!(selected(&result).body != StructureBody::None);
        assert!(
            environment
                .entered_modes
                .contains(&("gate:first".into(), SectionBodyMode::Trigger))
        );
        assert!(
            environment
                .entered_modes
                .contains(&("leaf".into(), SectionBodyMode::Trigger)),
            "child semantics still run normally"
        );
        assert!(
            !environment
                .notices
                .iter()
                .any(|notice| matches!(notice, Notice::Section { .. })),
            "ambiguous parent must block its descendants too: {:?}",
            environment.notices
        );
        assert!(environment.notices.contains(&Notice::Effect));
    }

    #[test]
    fn unique_section_retry_publishes_its_actual_body_mode_and_descendant_chain() {
        let mut environment = Environment {
            mixed_section_modes: true,
            first_section_conditions: true,
            reject_first_section: true,
            ..Default::default()
        };
        run(
            &competing_section_catalog(),
            "root:\n    gate:\n        leaf:\n            effect\n",
            &mut environment,
        )
        .unwrap();
        assert_eq!(
            environment.entered_modes[0],
            ("gate:first".into(), SectionBodyMode::Conditions)
        );
        assert_eq!(
            environment.entered_modes[1],
            ("gate:last".into(), SectionBodyMode::Trigger)
        );
        let sections = environment
            .notices
            .iter()
            .filter_map(|notice| match notice {
                Notice::Section {
                    attempt,
                    parent,
                    registration,
                    mode,
                    ..
                } => Some((*attempt, *parent, registration.as_str(), *mode)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            sections.len(),
            2,
            "only the unique accepted retry and its child are public"
        );
        assert_eq!(
            (sections[0].2, sections[0].3),
            ("gate:last", SectionBodyMode::Trigger)
        );
        assert_eq!(sections[1].2, "leaf");
        assert_eq!(sections[1].1, Some(sections[0].0));
        let Notice::Root(root_attempt, _, false) = environment.notices[0] else {
            panic!("unique root")
        };
        assert_eq!(sections[0].1, Some(root_attempt));
        assert!(
            sections[0].0 > root_attempt + 1,
            "the unpublished failed attempt still consumes its distinct ID"
        );
    }
}

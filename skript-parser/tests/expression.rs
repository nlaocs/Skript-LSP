use skript_parser::{
    ExpressionExpectedType, ExpressionFailureKind, ExpressionLeafCandidate, ExpressionLeafKind,
    ExpressionLeafParse, ExpressionLeafRequest, ExpressionNodeKind, ExpressionParseContext,
    ExpressionParseEnvironment, ExpressionParseRequest, ExpressionParserConfig, ExpressionRootMode,
    MappedSource, NoopExpressionEnvironment, PatternHookControl, PatternHookEvent,
    PatternMatchEnvironment, PatternMatchError, RegisteredExpressionDecision,
    RegisteredExpressionRequest, TextRange, TypeExpressionOutcome, TypeExpressionRequest,
    parse_expression, parse_expression_with_snapshot,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use syntaxes::{
    Catalog, CatalogParts, ClassName, DynamicMultiplicity, DynamicPattern, DynamicSyntaxDefinition,
    DynamicSyntaxId, DynamicSyntaxSnapshot, Multiplicity, PossibleReturnTypesState,
    RankedSyntaxCandidate, ResolutionState, ReturnTypeState, Syntax, SyntaxCandidateSource,
    SyntaxKind,
};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn expected(class_name: &str) -> ExpressionExpectedType {
    ExpressionExpectedType {
        class_name: ClassName(class_name.to_owned()),
        plural: false,
    }
}

fn expected_plural(class_name: &str) -> ExpressionExpectedType {
    ExpressionExpectedType {
        class_name: ClassName(class_name.to_owned()),
        plural: true,
    }
}

fn expression_fixture() -> Catalog {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let source = snapshot.catalog();
    let syntaxes = source
        .syntaxes()
        .iter()
        .filter(|syntax| match syntax {
            Syntax::Type(value) => matches!(value.code_name.as_str(), "string" | "object"),
            Syntax::Expression(value) => value.common.patterns.iter().any(|pattern| {
                matches!(
                    pattern.source.as_str(),
                    "dummy direct registry expression"
                        | "dummy supplier-backed expression"
                        | "dummy dynamically registered expression"
                        | "dummy dynamically registered expression using %string%"
                )
            }),
            _ => false,
        })
        .cloned()
        .collect();
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
        aliases: source.aliases().clone(),
        plural_rules: source.plural_rules().clone(),
        language: source
            .language_entries()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect(),
    })
}

#[test]
fn parses_registered_expression_without_placeholders() {
    let catalog = expression_fixture();
    let source = MappedSource::identity("dummy direct registry expression");
    let result = parse_expression(
        &catalog,
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, source.virtual_source().len()),
            expected_types: vec![expected("java.lang.String")],
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("registered expression must parse");

    let selected = result.selected.expect("one expression must be selected");
    let ExpressionNodeKind::Registered {
        pattern_index,
        registration_id,
        ..
    } = selected.node.kind
    else {
        panic!("registered node expected");
    };
    assert_eq!(pattern_index, 0);
    assert!(registration_id.starts_with("expression:skriptdummyaddon:"));
    assert_eq!(selected.node.span.local_range, TextRange::new(0, 32));
    assert!(selected.node.children.is_empty());
}

fn event_context(event_classes: &[&str]) -> ExpressionParseContext {
    ExpressionParseContext {
        event_classes: event_classes
            .iter()
            .map(|event| ClassName((*event).to_owned()))
            .collect(),
        ..ExpressionParseContext::default()
    }
}

fn catalog_with_syntaxes(source: &Catalog, syntaxes: Vec<Syntax>) -> Catalog {
    Catalog::new(CatalogParts {
        syntaxes,
        converters: source.converters().to_vec(),
        comparators: source.comparators().to_vec(),
        event_values: source.event_values().to_vec(),
        properties: source.properties().to_vec(),
        operators: source.operators().to_vec(),
        operations: source.operations().clone(),
        differences: source.differences().to_vec(),
        classes: source.classes().to_vec(),
        aliases: source.aliases().clone(),
        plural_rules: source.plural_rules().clone(),
        language: source
            .language_entries()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect(),
    })
}

#[test]
fn filtered_expression_patterns_keep_their_registration_index() {
    let catalog = expression_fixture();
    let id = DynamicSyntaxId::new("test.dynamic", "filtered-patterns");
    let patterns = ["first", "second", "third"]
        .into_iter()
        .map(|source| DynamicPattern {
            source: source.to_owned(),
            parsed: syntax_pattern_parser::syntax::parse(source, catalog.plural_rules())
                .expect("test pattern must parse"),
        })
        .collect();
    let definition = DynamicSyntaxDefinition {
        id: id.clone(),
        kind: SyntaxKind::Expression,
        patterns,
        priority: -100,
        before: Vec::new(),
        after: Vec::new(),
        return_type: Some("java.lang.String".to_owned()),
        return_multiplicity: Some(DynamicMultiplicity::Single),
        structure_node_type: None,
        structure_body_mode: None,
        entry_validator: None,
        handler: "test.dynamic.filtered-patterns".to_owned(),
        metadata: BTreeMap::new(),
        component_load_order: 1,
        declaration_order: 0,
    };
    let snapshot = DynamicSyntaxSnapshot {
        document_id: "file:///dynamic.sk".to_owned(),
        document_revision: 1,
        registry_revision: 1,
        definitions: BTreeMap::from([(id.clone(), definition)]),
        overrides: BTreeMap::new(),
        candidates: vec![RankedSyntaxCandidate {
            source: SyntaxCandidateSource::Dynamic(id),
            kind: SyntaxKind::Expression,
            overrides: Vec::new(),
        }],
    };
    let text = "third";
    let source = MappedSource::identity(text);
    let result = parse_expression_with_snapshot(
        &catalog,
        Some(&snapshot),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![expected("java.lang.String")],
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("the third pattern must match");

    let selected = result.selected.expect("one expression must be selected");
    let ExpressionNodeKind::Registered { pattern_index, .. } = selected.node.kind else {
        panic!("registered node expected");
    };
    // `matcher_candidates` keeps only the matching pattern in its temporary
    // Vec, but the public result must retain the original registration index.
    assert_eq!(pattern_index, 2);
}

#[test]
fn expression_candidate_order_is_deterministic() {
    let catalog = expression_fixture();
    let source = MappedSource::identity("dummy supplier-backed expression");
    let parse = || {
        parse_expression(
            &catalog,
            ExpressionParseRequest {
                source: &source,
                range: TextRange::new(0, source.virtual_source().len()),
                expected_types: vec![expected("java.lang.String")],
                context: ExpressionParseContext::default(),
            },
            &mut NoopExpressionEnvironment,
            ExpressionParserConfig::default(),
        )
        .unwrap()
    };

    assert_eq!(parse(), parse());
}

#[test]
fn recursively_parses_typed_expression_capture() {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let catalog = snapshot.catalog();
    let text = concat!(
        "dummy dynamically registered expression using ",
        "dummy direct registry expression"
    );
    let source = MappedSource::identity(text);
    let result = parse_expression(
        catalog,
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![expected("java.lang.String")],
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("recursive expression must parse");

    let selected = result.selected.expect("outer expression must be selected");
    assert_eq!(selected.node.children.len(), 1);
    assert_eq!(
        selected.node.children[0].span.local_range.slice(text),
        Some("dummy direct registry expression")
    );
    assert!(matches!(
        selected.node.children[0].kind,
        ExpressionNodeKind::Registered { .. }
    ));
}

#[test]
fn nested_parentheses_are_transparent_nodes_with_exact_spans() {
    let catalog = expression_fixture();
    let text = "((\"hello\"))";
    let source = MappedSource::identity(text);
    let result = parse_expression(
        &catalog,
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![expected("java.lang.String")],
            context: ExpressionParseContext::default(),
        },
        &mut LiteralEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("nested parenthesized literal must parse");

    assert!(result.alternatives.is_empty());
    let outer = result.selected.expect("outer group must be selected").node;
    assert!(matches!(outer.kind, ExpressionNodeKind::Grouped));
    assert_eq!(outer.span.local_range, TextRange::new(0, text.len()));
    assert_eq!(
        outer.return_type,
        Some(ClassName("java.lang.String".to_owned()))
    );

    let inner = &outer.children[0];
    assert!(matches!(inner.kind, ExpressionNodeKind::Grouped));
    assert_eq!(inner.span.local_range, TextRange::new(1, text.len() - 1));
    assert!(matches!(
        inner.children[0].kind,
        ExpressionNodeKind::Literal { .. }
    ));
    assert_eq!(
        inner.children[0].span.local_range,
        TextRange::new(2, text.len() - 2)
    );
}

#[test]
fn root_parse_mode_distinguishes_literals_from_registered_expressions() {
    let catalog = expression_fixture();

    let literal = MappedSource::identity("\"hello\"");
    let literal_result = parse_expression(
        &catalog,
        ExpressionParseRequest {
            source: &literal,
            range: TextRange::new(0, literal.virtual_source().len()),
            expected_types: vec![expected("java.lang.String")],
            context: ExpressionParseContext::default(),
        },
        &mut LiteralEnvironment,
        ExpressionParserConfig {
            root_mode: ExpressionRootMode::ExpressionsOnly,
            ..ExpressionParserConfig::default()
        },
    )
    .expect("mode filtering is a normal parse result");
    assert!(literal_result.selected.is_none());

    let registered = MappedSource::identity("dummy direct registry expression");
    let registered_result = parse_expression(
        &catalog,
        ExpressionParseRequest {
            source: &registered,
            range: TextRange::new(0, registered.virtual_source().len()),
            expected_types: vec![expected("java.lang.String")],
            context: ExpressionParseContext::default(),
        },
        &mut LiteralEnvironment,
        ExpressionParserConfig {
            root_mode: ExpressionRootMode::LiteralsOnly,
            ..ExpressionParserConfig::default()
        },
    )
    .expect("mode filtering is a normal parse result");
    assert!(registered_result.selected.is_none());
}

#[test]
fn parenthesized_expression_trims_inner_ascii_whitespace_like_skript() {
    let catalog = expression_fixture();
    let text = "( \t\"hello\"\r\n )";
    let source = MappedSource::identity(text);
    let result = parse_expression(
        &catalog,
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![expected("java.lang.String")],
            context: ExpressionParseContext::default(),
        },
        &mut LiteralEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("parenthesized literal with inner whitespace must parse");

    let group = result.selected.expect("group must be selected").node;
    assert!(matches!(group.kind, ExpressionNodeKind::Grouped));
    assert_eq!(group.span.local_range, TextRange::new(0, text.len()));
    assert_eq!(group.children[0].span.local_range, TextRange::new(3, 10));
    assert_eq!(
        group.children[0].span.local_range.slice(text),
        Some("\"hello\"")
    );
}

#[test]
fn typed_capture_preserves_its_parenthesized_child() {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let inner = "dummy direct registry expression";
    let text = format!("dummy dynamically registered expression using ({inner})");
    let source = MappedSource::identity(text.as_str());
    let result = parse_expression(
        snapshot.catalog(),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![expected("java.lang.String")],
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("parenthesized typed capture must parse");

    let outer = result.selected.expect("outer expression must parse").node;
    let grouped = &outer.children[0];
    assert!(matches!(grouped.kind, ExpressionNodeKind::Grouped));
    assert_eq!(
        grouped.span.local_range.slice(&text),
        Some(format!("({inner})").as_str())
    );
    assert!(matches!(
        grouped.children[0].kind,
        ExpressionNodeKind::Registered { .. }
    ));
    assert_eq!(
        grouped.children[0].span.local_range.slice(&text),
        Some(inner)
    );
}

#[test]
fn parentheses_owned_by_a_registered_pattern_are_not_unwrapped() {
    let catalog = expression_fixture();
    let id = DynamicSyntaxId::new("test.dynamic", "parenthesized-pattern");
    let source_pattern = r"\(%string%\) suffix";
    let definition = DynamicSyntaxDefinition {
        id: id.clone(),
        kind: SyntaxKind::Expression,
        patterns: vec![DynamicPattern {
            source: source_pattern.to_owned(),
            parsed: syntax_pattern_parser::syntax::parse(source_pattern, catalog.plural_rules())
                .unwrap(),
        }],
        priority: -50,
        before: Vec::new(),
        after: Vec::new(),
        return_type: Some("java.lang.String".to_owned()),
        return_multiplicity: Some(DynamicMultiplicity::Single),
        structure_node_type: None,
        structure_body_mode: None,
        entry_validator: None,
        handler: "test.dynamic.parenthesized-pattern".to_owned(),
        metadata: BTreeMap::new(),
        component_load_order: 1,
        declaration_order: 0,
    };
    let snapshot = DynamicSyntaxSnapshot {
        document_id: "file:///dynamic.sk".to_owned(),
        document_revision: 1,
        registry_revision: 10,
        definitions: BTreeMap::from([(id.clone(), definition)]),
        overrides: BTreeMap::new(),
        candidates: vec![RankedSyntaxCandidate {
            source: SyntaxCandidateSource::Dynamic(id),
            kind: SyntaxKind::Expression,
            overrides: Vec::new(),
        }],
    };
    let text = "(\"hello\") suffix";
    let source = MappedSource::identity(text);
    let result = parse_expression_with_snapshot(
        &catalog,
        Some(&snapshot),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![expected("java.lang.String")],
            context: ExpressionParseContext::default(),
        },
        &mut LiteralEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("registered parenthesis pattern must parse");

    let selected = result.selected.expect("registered candidate must win").node;
    assert!(matches!(
        selected.kind,
        ExpressionNodeKind::Registered { .. }
    ));
    assert!(matches!(
        selected.children[0].kind,
        ExpressionNodeKind::Literal { .. }
    ));
}

#[test]
fn parenthesis_failures_keep_primary_and_related_spans() {
    let catalog = expression_fixture();
    for (text, kind, primary, related) in [
        (
            "(unknown",
            ExpressionFailureKind::UnclosedParenthesis,
            TextRange::empty(8),
            Some(TextRange::new(0, 1)),
        ),
        (
            "unknown)",
            ExpressionFailureKind::UnexpectedClosingParenthesis,
            TextRange::new(7, 8),
            None,
        ),
        (
            "( )",
            ExpressionFailureKind::EmptyGroup,
            TextRange::empty(2),
            Some(TextRange::new(0, 1)),
        ),
    ] {
        let source = MappedSource::identity(text);
        let result = parse_expression(
            &catalog,
            ExpressionParseRequest {
                source: &source,
                range: TextRange::new(0, text.len()),
                expected_types: vec![expected("java.lang.String")],
                context: ExpressionParseContext::default(),
            },
            &mut NoopExpressionEnvironment,
            ExpressionParserConfig::default(),
        )
        .expect("invalid parentheses are a recoverable no-match");
        let failure = result.failure.expect("failure must be retained");
        assert_eq!(failure.kind, kind);
        assert_eq!(failure.span.local_range, primary);
        assert_eq!(failure.related_span.map(|span| span.local_range), related);
    }

    let quoted = MappedSource::identity("\"(\"");
    let result = parse_expression(
        &catalog,
        ExpressionParseRequest {
            source: &quoted,
            range: TextRange::new(0, quoted.virtual_source().len()),
            expected_types: vec![expected("java.lang.String")],
            context: ExpressionParseContext::default(),
        },
        &mut LiteralEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("parenthesis inside a quote must remain literal text");
    assert!(result.selected.is_some());
}

#[test]
fn parenthesized_recursion_uses_the_expression_depth_limit() {
    let catalog = expression_fixture();
    let text = "(((\"hello\")))";
    let source = MappedSource::identity(text);
    let error = parse_expression(
        &catalog,
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![expected("java.lang.String")],
            context: ExpressionParseContext::default(),
        },
        &mut LiteralEnvironment,
        ExpressionParserConfig {
            max_depth: 1,
            ..ExpressionParserConfig::default()
        },
    )
    .expect_err("deeply grouped input must hit the configured depth bound");
    assert!(matches!(
        error,
        skript_parser::ExpressionParseError::DepthLimit { limit: 1 }
    ));
}

#[test]
fn parses_dynamic_expression_from_frozen_registry_order() {
    let catalog = expression_fixture();
    let id = DynamicSyntaxId::new("test.dynamic", "expression");
    let source_pattern = "dynamic expression";
    let definition = DynamicSyntaxDefinition {
        id: id.clone(),
        kind: SyntaxKind::Expression,
        patterns: vec![DynamicPattern {
            source: source_pattern.to_owned(),
            parsed: syntax_pattern_parser::syntax::parse(source_pattern, catalog.plural_rules())
                .unwrap(),
        }],
        priority: -50,
        before: Vec::new(),
        after: Vec::new(),
        return_type: Some("java.lang.String".to_owned()),
        return_multiplicity: Some(DynamicMultiplicity::Single),
        structure_node_type: None,
        structure_body_mode: None,
        entry_validator: None,
        handler: "test.dynamic.expression".to_owned(),
        metadata: BTreeMap::from([("source".to_owned(), "test".to_owned())]),
        component_load_order: 1,
        declaration_order: 0,
    };
    let snapshot = DynamicSyntaxSnapshot {
        document_id: "file:///dynamic.sk".to_owned(),
        document_revision: 1,
        registry_revision: 9,
        definitions: BTreeMap::from([(id.clone(), definition)]),
        overrides: BTreeMap::new(),
        candidates: vec![RankedSyntaxCandidate {
            source: SyntaxCandidateSource::Dynamic(id),
            kind: SyntaxKind::Expression,
            overrides: Vec::new(),
        }],
    };
    let source = MappedSource::identity(source_pattern);
    let result = parse_expression_with_snapshot(
        &catalog,
        Some(&snapshot),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, source_pattern.len()),
            expected_types: vec![expected("java.lang.String")],
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("dynamic Expression must parse");

    let selected = result.selected.expect("dynamic candidate must be selected");
    assert!(matches!(
        selected.node.kind,
        ExpressionNodeKind::Registered { ref registration_id, .. }
            if registration_id == "dynamic:test.dynamic/expression"
    ));
    assert_eq!(
        selected.node.metadata.get("source"),
        Some(&"test".to_owned())
    );
}

#[test]
fn excludes_incompatible_expected_return_type() {
    let catalog = expression_fixture();
    let text = "dummy direct registry expression";
    let source = MappedSource::identity(text);
    let result = parse_expression(
        &catalog,
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![expected("java.lang.Number")],
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("type mismatch is a normal no-match");

    assert!(result.selected.is_none());
    assert!(result.alternatives.is_empty());
    assert_eq!(
        result
            .failure
            .expect("failure must be retained")
            .expected_types,
        vec![expected("java.lang.Number")]
    );
}

#[test]
fn dynamic_return_type_is_finalized_after_the_pattern_matches() {
    let source_catalog = expression_fixture();
    let mut syntaxes = source_catalog.syntaxes().to_vec();
    let expression = syntaxes
        .iter_mut()
        .find_map(|syntax| match syntax {
            Syntax::Expression(value)
                if value
                    .common
                    .patterns
                    .iter()
                    .any(|pattern| pattern.source == "dummy direct registry expression") =>
            {
                Some(value)
            }
            _ => None,
        })
        .expect("fixture Expression must exist");
    expression.return_type = Some(ClassName("java.lang.Object".to_owned()));
    expression.return_type_state = ReturnTypeState::Dynamic;
    expression.possible_return_types = vec![ClassName("java.lang.Long".to_owned())];
    expression.possible_return_types_state = PossibleReturnTypesState::Partial;
    let catalog = Catalog::new(CatalogParts {
        syntaxes,
        converters: Vec::new(),
        comparators: Vec::new(),
        event_values: Vec::new(),
        properties: Vec::new(),
        operators: Vec::new(),
        operations: BTreeMap::new(),
        differences: Vec::new(),
        classes: Vec::new(),
        aliases: source_catalog.aliases().clone(),
        plural_rules: source_catalog.plural_rules().clone(),
        language: source_catalog
            .language_entries()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect(),
    });
    let text = "dummy direct registry expression";
    let source = MappedSource::identity(text);
    let mut environment = DynamicReturnEnvironment::default();
    let result = parse_expression(
        &catalog,
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![expected("java.lang.Long")],
            context: ExpressionParseContext::default(),
        },
        &mut environment,
        ExpressionParserConfig::default(),
    )
    .expect("dynamic return type must parse");

    let selected = result.selected.expect("resolved candidate must survive");
    assert_eq!(
        selected.node.return_type,
        Some(ClassName("java.lang.Long".to_owned()))
    );
    assert_eq!(environment.resolutions, 1);
    assert_eq!(environment.finalizations, [true]);
}

#[test]
fn parses_leaf_candidates_from_the_extension_environment() {
    let catalog = expression_fixture();
    let source = MappedSource::identity("\"hello\"");
    let mut environment = LiteralEnvironment;
    let result = parse_expression(
        &catalog,
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, source.virtual_source().len()),
            expected_types: vec![expected("java.lang.String")],
            context: ExpressionParseContext::default(),
        },
        &mut environment,
        ExpressionParserConfig::default(),
    )
    .expect("literal extension must parse");

    let selected = result.selected.expect("literal must be selected");
    assert!(matches!(
        selected.node.kind,
        ExpressionNodeKind::Literal { ref parser_id } if parser_id == "test.string"
    ));
}

#[test]
fn top_level_plural_expectation_controls_multiple_leaf_acceptance() {
    let catalog = expression_fixture();
    let source = MappedSource::identity("values");
    let mut environment = MultipleEnvironment::default();
    let singular = parse_expression(
        &catalog,
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, source.virtual_source().len()),
            expected_types: vec![expected("java.lang.String")],
            context: ExpressionParseContext::default(),
        },
        &mut environment,
        ExpressionParserConfig::default(),
    )
    .expect("multiplicity filtering is a normal parse result");
    assert!(singular.selected.is_none());
    assert_eq!(environment.finalizations, [false]);

    environment.finalizations.clear();
    let plural = parse_expression(
        &catalog,
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, source.virtual_source().len()),
            expected_types: vec![expected_plural("java.lang.String")],
            context: ExpressionParseContext::default(),
        },
        &mut environment,
        ExpressionParserConfig::default(),
    )
    .expect("plural leaf must parse");
    let selected = plural
        .selected
        .expect("plural expectation must accept a multiple leaf");
    assert_eq!(selected.node.multiplicity, Some(Multiplicity::Multiple));
    assert_eq!(environment.finalizations, [true]);
}

#[test]
fn grows_left_recursive_expression_from_a_literal_seed() {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let text = "\"hello\" in upper case";
    let source = MappedSource::identity(text);
    let result = parse_expression(
        snapshot.catalog(),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![expected("java.lang.String")],
            context: ExpressionParseContext::default(),
        },
        &mut LiteralEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("left-recursive string expression must parse");

    let selected = result
        .selected
        .expect("transformed string must be selected");
    assert!(matches!(
        selected.node.kind,
        ExpressionNodeKind::Registered { .. }
    ));
    assert_eq!(selected.node.children.len(), 1);
    assert!(matches!(
        selected.node.children[0].kind,
        ExpressionNodeKind::Literal { .. }
    ));
}

#[test]
fn optional_leading_literal_does_not_require_its_boundary_space() {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    for text in ["absorbed blocks", "the absorbed blocks"] {
        let source = MappedSource::identity(text);
        let result = parse_expression(
            snapshot.catalog(),
            ExpressionParseRequest {
                source: &source,
                range: TextRange::new(0, text.len()),
                expected_types: vec![expected_plural("ch.njol.skript.util.BlockStateBlock")],
                context: event_context(&["org.bukkit.event.block.SpongeAbsorbEvent"]),
            },
            &mut NoopExpressionEnvironment,
            ExpressionParserConfig::default(),
        )
        .expect("optional leading literal must parse with or without its following space");

        assert!(result.selected.is_some(), "{text:?} must match");
    }
}

#[test]
fn restricted_expression_matches_an_exact_event() {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let text = "final damage";
    let source = MappedSource::identity(text);
    let result = parse_expression(
        snapshot.catalog(),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![expected("java.lang.Number")],
            context: event_context(&["org.bukkit.event.entity.EntityDamageEvent"]),
        },
        &mut NoopExpressionEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("exact Event context must allow the restricted Expression");

    let selected = result.selected.expect("restricted Expression must parse");
    assert_eq!(
        selected.node.span.local_range,
        TextRange::new(0, text.len())
    );
}

#[test]
fn restricted_expression_accepts_a_child_event() {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let text = "final damage";
    let source = MappedSource::identity(text);
    let result = parse_expression(
        snapshot.catalog(),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![expected("java.lang.Number")],
            context: event_context(&["org.bukkit.event.entity.EntityDamageByEntityEvent"]),
        },
        &mut NoopExpressionEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("a child Event must satisfy the supported parent Event");

    assert!(result.selected.is_some());
}

#[test]
fn unresolved_expression_event_restriction_does_not_drop_the_candidate() {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let source_catalog = snapshot.catalog();
    let mut syntaxes = source_catalog.syntaxes().to_vec();
    let target = syntaxes
        .iter_mut()
        .find_map(|syntax| match syntax {
            Syntax::Expression(value)
                if value.common.element_class.as_str()
                    == "ch.njol.skript.expressions.ExprFinalDamage" =>
            {
                Some(value)
            }
            _ => None,
        })
        .expect("fixture must contain final damage");
    target.common.supported_events_state = Some(ResolutionState::Unresolved);
    let catalog = catalog_with_syntaxes(source_catalog, syntaxes);
    let text = "final damage";
    let source = MappedSource::identity(text);
    let result = parse_expression(
        &catalog,
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![expected("java.lang.Number")],
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("unresolved event metadata must remain parseable");

    assert!(result.selected.is_some());
}

#[test]
fn unknown_event_hierarchy_does_not_reject_a_restricted_expression() {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let text = "final damage";
    let source = MappedSource::identity(text);
    let result = parse_expression(
        snapshot.catalog(),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![expected("java.lang.Number")],
            context: event_context(&["test.UnknownEvent"]),
        },
        &mut NoopExpressionEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("unknown Event hierarchy must not hard-reject the candidate");

    assert!(result.selected.is_some());
}

#[test]
fn interface_return_type_matches_java_object() {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    assert!(
        snapshot
            .catalog()
            .is_class_assignable("org.bukkit.OfflinePlayer", "java.lang.Object")
    );
    let text = "all offline players";
    let source = MappedSource::identity(text);
    let result = parse_expression(
        snapshot.catalog(),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![expected_plural("java.lang.Object")],
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        ExpressionParserConfig::default(),
    )
    .expect("interface return type must be assignable to Object");

    let selected = result.selected.expect("offline players must parse");
    assert_eq!(
        selected
            .node
            .return_type
            .as_ref()
            .map(|value| value.as_str()),
        Some("org.bukkit.OfflinePlayer")
    );
    assert_eq!(selected.node.multiplicity, Some(Multiplicity::Multiple));
}

#[test]
fn recursion_depth_is_bounded() {
    let catalog = expression_fixture();
    let text = format!(
        "{}dummy direct registry expression",
        "dummy dynamically registered expression using ".repeat(5)
    );
    let source = MappedSource::identity(text.as_str());
    let error = parse_expression(
        &catalog,
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![expected("java.lang.String")],
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        ExpressionParserConfig {
            max_depth: 2,
            ..ExpressionParserConfig::default()
        },
    )
    .expect_err("recursive input must hit the configured depth bound");

    assert!(matches!(
        error,
        skript_parser::ExpressionParseError::Matcher(PatternMatchError::TypeResolver {
            ref message,
            ..
        }) if message.contains("recursion depth limit of 2")
    ));
}

#[derive(Default)]
struct MultipleEnvironment {
    finalizations: Vec<bool>,
}

#[derive(Default)]
struct DynamicReturnEnvironment {
    resolutions: usize,
    finalizations: Vec<bool>,
}

impl PatternMatchEnvironment for DynamicReturnEnvironment {
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

impl ExpressionParseEnvironment for DynamicReturnEnvironment {
    fn parse_expression_leaf(
        &mut self,
        _request: ExpressionLeafRequest<'_>,
    ) -> Result<ExpressionLeafParse, String> {
        Ok(ExpressionLeafParse::default())
    }

    fn resolve_registered_expression(
        &mut self,
        _request: RegisteredExpressionRequest<'_>,
    ) -> Result<RegisteredExpressionDecision, String> {
        self.resolutions += 1;
        Ok(RegisteredExpressionDecision::Resolved {
            return_type: Some(ClassName("java.lang.Long".to_owned())),
            possible_return_types: vec![ClassName("java.lang.Long".to_owned())],
            possible_return_types_state: PossibleReturnTypesState::Complete,
            multiplicity: Some(Multiplicity::Single),
            metadata: BTreeMap::new(),
        })
    }

    fn finish_registered_expression(&mut self, accepted: bool) -> Result<(), String> {
        self.finalizations.push(accepted);
        Ok(())
    }

    fn state_revision(&self) -> Result<u64, String> {
        Ok(0)
    }
}

impl PatternMatchEnvironment for MultipleEnvironment {
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

impl ExpressionParseEnvironment for MultipleEnvironment {
    fn parse_expression_leaf(
        &mut self,
        request: ExpressionLeafRequest<'_>,
    ) -> Result<ExpressionLeafParse, String> {
        Ok(vec![ExpressionLeafCandidate {
            parser_id: "test.multiple".to_owned(),
            kind: ExpressionLeafKind::Custom,
            range: request.remaining,
            return_type: Some(ClassName("java.lang.String".to_owned())),
            multiplicity: Some(Multiplicity::Multiple),
            children: Vec::new(),
            metadata: BTreeMap::new(),
        }]
        .into())
    }

    fn finish_expression_leaf(&mut self, accepted: bool) -> Result<(), String> {
        self.finalizations.push(accepted);
        Ok(())
    }

    fn state_revision(&self) -> Result<u64, String> {
        Ok(0)
    }
}

struct LiteralEnvironment;

impl PatternMatchEnvironment for LiteralEnvironment {
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

impl ExpressionParseEnvironment for LiteralEnvironment {
    fn parse_expression_leaf(
        &mut self,
        request: ExpressionLeafRequest<'_>,
    ) -> Result<ExpressionLeafParse, String> {
        let candidate = request.candidate_ends.iter().rev().find_map(|end| {
            let range = TextRange::new(request.remaining.start, *end);
            let text = range.slice(request.input)?;
            (text.starts_with('"') && text.ends_with('"')).then_some(range)
        });
        Ok(candidate
            .map(|range| ExpressionLeafCandidate {
                parser_id: "test.string".to_owned(),
                kind: ExpressionLeafKind::Literal,
                range,
                return_type: Some(ClassName("java.lang.String".to_owned())),
                multiplicity: Some(Multiplicity::Single),
                children: Vec::new(),
                metadata: BTreeMap::new(),
            })
            .into_iter()
            .collect::<Vec<_>>()
            .into())
    }

    fn state_revision(&self) -> Result<u64, String> {
        Ok(0)
    }
}

use skript_parser::{
    ExpressionExpectedType, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionLeafRequest,
    ExpressionNodeKind, ExpressionParseContext, ExpressionParseEnvironment, ExpressionParseRequest,
    ExpressionParserConfig, MappedSource, NoopExpressionEnvironment, PatternHookControl,
    PatternHookEvent, PatternMatchEnvironment, PatternMatchError, TextRange, TypeExpressionRequest,
    TypeExpressionResolution, parse_expression, parse_expression_with_snapshot,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use syntaxes::{
    Catalog, CatalogParts, ClassName, DynamicMultiplicity, DynamicPattern, DynamicSyntaxDefinition,
    DynamicSyntaxId, DynamicSyntaxSnapshot, Multiplicity, RankedSyntaxCandidate, Syntax,
    SyntaxCandidateSource, SyntaxKind,
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

impl PatternMatchEnvironment for MultipleEnvironment {
    fn resolve_type(
        &mut self,
        _request: TypeExpressionRequest<'_>,
    ) -> Result<Vec<TypeExpressionResolution>, String> {
        Ok(Vec::new())
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
    ) -> Result<Vec<ExpressionLeafCandidate>, String> {
        Ok(vec![ExpressionLeafCandidate {
            parser_id: "test.multiple".to_owned(),
            kind: ExpressionLeafKind::Custom,
            range: request.remaining,
            return_type: Some(ClassName("java.lang.String".to_owned())),
            multiplicity: Some(Multiplicity::Multiple),
            metadata: BTreeMap::new(),
        }])
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
    ) -> Result<Vec<TypeExpressionResolution>, String> {
        Ok(Vec::new())
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
    ) -> Result<Vec<ExpressionLeafCandidate>, String> {
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
                metadata: BTreeMap::new(),
            })
            .into_iter()
            .collect())
    }

    fn state_revision(&self) -> Result<u64, String> {
        Ok(0)
    }
}

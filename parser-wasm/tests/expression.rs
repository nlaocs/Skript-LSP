use parser_wasm::host::{HostConfig, InvocationContext, ParserHost};
use skript_parser::{
    ExpressionExpectedType, ExpressionNodeKind, ExpressionParseContext, ExpressionParseRequest,
    ExpressionParserConfig, MappedSource, TextRange,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use syntaxes::{Catalog, CatalogParts, ClassName, Syntax};

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn expression_catalog() -> Arc<Catalog> {
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
    Arc::new(Catalog::new(CatalogParts {
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
    }))
}

fn context(revision: u64) -> InvocationContext {
    InvocationContext {
        invocation_id: revision,
        subscription_id: String::new(),
        document_id: "file:///workspace/expression.sk".to_owned(),
        document_revision: revision,
        expansion: None,
        syntax_context: 7,
    }
}

fn expected_type(class_name: &str) -> ExpressionExpectedType {
    ExpressionExpectedType {
        class_name: ClassName(class_name.to_owned()),
        plural: false,
    }
}

fn parser_context() -> ExpressionParseContext {
    ExpressionParseContext {
        syntax_context: 7,
        ..ExpressionParseContext::default()
    }
}

#[test]
fn core_library_and_registered_expressions_share_one_recursive_parser() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(expression_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let cases = [
        ("\"hello\"", "java.lang.String", "core.literal.string"),
        ("{message}", "java.lang.String", "core.variable"),
        ("42", "java.lang.Long", "core.literal.number"),
    ];

    for (index, (text, expected, parser_id)) in cases.into_iter().enumerate() {
        let revision = index as u64 + 1;
        let transaction = host
            .begin_parse(
                "file:///workspace",
                "file:///workspace/expression.sk",
                revision,
            )
            .unwrap();
        let source = MappedSource::identity(text);
        let result = host
            .parse_expression_in_parse(
                &transaction,
                context(revision),
                ExpressionParseRequest {
                    source: &source,
                    range: TextRange::new(0, text.len()),
                    expected_types: vec![expected_type(expected)],
                    context: parser_context(),
                },
                ExpressionParserConfig::default(),
            )
            .expect("CoreLibrary leaf must parse");
        let selected = result.matches.selected.expect("leaf must be selected");
        assert!(matches!(
            selected.node.kind,
            ExpressionNodeKind::Literal { parser_id: ref actual }
                | ExpressionNodeKind::Variable { parser_id: ref actual }
                if actual == parser_id
        ));
        assert!(result.calls.iter().any(|call| {
            call.component_id == "nlaocs.core-library"
                && call.subscription_id == "core.expression-leaves"
        }));
        transaction.cancel().unwrap();
    }

    let text = concat!(
        "dummy dynamically registered expression using ",
        "dummy direct registry expression"
    );
    let revision = 10;
    let transaction = host
        .begin_parse(
            "file:///workspace",
            "file:///workspace/expression.sk",
            revision,
        )
        .unwrap();
    let source = MappedSource::identity(text);
    let result = host
        .parse_expression_in_parse(
            &transaction,
            context(revision),
            ExpressionParseRequest {
                source: &source,
                range: TextRange::new(0, text.len()),
                expected_types: vec![expected_type("java.lang.String")],
                context: parser_context(),
            },
            ExpressionParserConfig::default(),
        )
        .expect("registered recursive Expression must parse");
    let selected = result
        .matches
        .selected
        .expect("outer Expression must exist");
    assert!(matches!(
        selected.node.kind,
        ExpressionNodeKind::Registered { .. }
    ));
    assert_eq!(selected.node.children.len(), 1);
    assert!(matches!(
        selected.node.children[0].kind,
        ExpressionNodeKind::Registered { .. }
    ));
    transaction.cancel().unwrap();
}

#[test]
fn no_match_rolls_back_expression_hook_state_revision() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(expression_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/expression.sk", 1)
        .unwrap();
    let source = MappedSource::identity("not a known expression");
    let result = host
        .parse_expression_in_parse(
            &transaction,
            context(1),
            ExpressionParseRequest {
                source: &source,
                range: TextRange::new(0, source.virtual_source().len()),
                expected_types: vec![expected_type("java.lang.String")],
                context: parser_context(),
            },
            ExpressionParserConfig::default(),
        )
        .expect("no-match is not a host error");

    assert!(result.matches.selected.is_none());
    assert_eq!(transaction.state_revision().unwrap(), 0);
    transaction.cancel().unwrap();
}

use parser_wasm::host::{HostConfig, InvocationContext, ParserHost};
use skript_parser::{
    ExpressionExpectedType, ExpressionMatches, ExpressionNodeKind, ExpressionParseContext,
    ExpressionParseRequest, ExpressionParserConfig, MappedSource, TextRange,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use syntaxes::{Catalog, ClassName};

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn context(revision: u64) -> InvocationContext {
    InvocationContext {
        invocation_id: revision,
        subscription_id: String::new(),
        document_id: "file:///workspace/function.sk".to_owned(),
        document_revision: revision,
        expansion: None,
        syntax_context: 7,
    }
}

fn parse(host: &mut ParserHost, revision: u64, text: &str) -> ExpressionMatches {
    let transaction = host
        .begin_parse(
            "file:///workspace",
            "file:///workspace/function.sk",
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
                expected_types: vec![ExpressionExpectedType {
                    class_name: ClassName("java.lang.Object".to_owned()),
                    plural: true,
                }],
                context: ExpressionParseContext {
                    syntax_context: 7,
                    ..ExpressionParseContext::default()
                },
            },
            ExpressionParserConfig::default(),
        )
        .expect("Function parsing must not fail the host");
    transaction.cancel().unwrap();
    result.matches
}

fn host() -> ParserHost {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(Arc::<Catalog>::new(snapshot.catalog().clone())),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load")
}

#[test]
fn parses_catalog_function_identity_and_nested_arguments() {
    let mut host = host();
    let selected = parse(&mut host, 1, "sin(abs(-1))")
        .selected
        .expect("nested Function must parse")
        .node;
    assert!(matches!(
        selected.kind,
        ExpressionNodeKind::Function { ref parser_id } if parser_id == "core.function"
    ));
    let call = selected.function.expect("Function must be structured");
    assert_eq!(call.name, "sin");
    assert!(call.registration_id.starts_with("function:skript:"));
    assert_eq!(call.arguments.len(), 1);
    assert_eq!(call.arguments[0].parameter_name, "n");
    assert_eq!(call.arguments[0].child_count, 1);
    assert!(matches!(
        selected.children[0].kind,
        ExpressionNodeKind::Function { .. }
    ));
    assert_eq!(selected.children[0].function.as_ref().unwrap().name, "abs");
}

#[test]
fn follows_skript_named_optional_and_list_parameter_rules() {
    let mut host = host();

    let named = parse(&mut host, 1, "log(base: 2, n: 8)")
        .selected
        .expect("named arguments must parse")
        .node;
    let call = named.function.unwrap();
    assert_eq!(call.arguments[0].parameter_name, "n");
    assert_eq!(call.arguments[0].supplied_name.as_deref(), Some("n"));
    assert_eq!(
        named.children[0]
            .span
            .local_range
            .slice("log(base: 2, n: 8)"),
        Some("8")
    );
    assert_eq!(call.arguments[1].parameter_name, "base");
    assert_eq!(call.arguments[1].supplied_name.as_deref(), Some("base"));
    assert_eq!(
        named.children[1]
            .span
            .local_range
            .slice("log(base: 2, n: 8)"),
        Some("2")
    );

    let optional = parse(&mut host, 2, "log(8)")
        .selected
        .expect("omitted optional argument must parse")
        .node;
    let call = optional.function.unwrap();
    assert!(!call.arguments[0].omitted);
    assert!(call.arguments[1].omitted);
    assert_eq!(call.arguments[1].child_count, 0);

    let list = parse(&mut host, 3, "sum(1, 2)")
        .selected
        .expect("single plural parameter must accept comma arguments")
        .node;
    let call = list.function.unwrap();
    assert_eq!(call.arguments.len(), 1);
    assert_eq!(call.arguments[0].parameter_name, "ns");
    assert_eq!(call.arguments[0].child_count, 2);
    assert_eq!(list.children.len(), 2);

    for (revision, text) in [(4, "sum(1 and 2)"), (5, "sum((1, 2))")] {
        let list = parse(&mut host, revision, text)
            .selected
            .unwrap_or_else(|| panic!("{text:?} must remain one list parameter"))
            .node;
        assert_eq!(list.function.unwrap().arguments[0].child_count, 2);
        assert_eq!(list.children.len(), 2);
    }

    let quoted_comma = parse(&mut host, 6, "concat(\"a,b\", \"c\")")
        .selected
        .expect("commas inside quoted strings must not split arguments")
        .node;
    assert_eq!(quoted_comma.function.unwrap().arguments[0].child_count, 2);
    assert_eq!(quoted_comma.children.len(), 2);
}

#[test]
fn rejects_unknown_wrong_arity_and_wrong_type_calls() {
    let mut host = host();
    for (revision, text) in [
        (1, "unknown_function(1)"),
        (2, "sin()"),
        (3, "sin(1, 2)"),
        (4, "sin(\"not a number\")"),
        (5, "sum(1 or 2)"),
        (6, "log(n: 8, n: 2)"),
    ] {
        assert!(
            parse(&mut host, revision, text).selected.is_none(),
            "{text:?} must not resolve to a Function"
        );
    }
}

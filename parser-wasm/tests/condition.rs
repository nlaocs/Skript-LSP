use parser_wasm::host::{HostConfig, InvocationContext, ParserHost};
use skript_parser::{
    ConditionNodeKind, ConditionParseRequest, ConditionParserConfig, ExpressionParseContext,
    ExpressionParseRequest, ExpressionParserConfig, MappedSource, TextRange,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use syntaxes::{Catalog, ClassName, Multiplicity};

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn catalog() -> Arc<Catalog> {
    Arc::new(
        ssg::load(fixture())
            .expect("schema 4 fixture must load")
            .catalog()
            .clone(),
    )
}

fn context(revision: u64) -> InvocationContext {
    InvocationContext {
        invocation_id: revision,
        subscription_id: String::new(),
        document_id: "file:///workspace/condition.sk".to_owned(),
        document_revision: revision,
        expansion: None,
        syntax_context: 7,
    }
}

fn parser_context() -> ExpressionParseContext {
    ExpressionParseContext {
        syntax_context: 7,
        ..ExpressionParseContext::default()
    }
}

#[test]
fn condition_pipeline_uses_core_expression_candidates() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/condition.sk", 1)
        .unwrap();
    let text = "dummy fixture condition with \"hello\"";
    let source = MappedSource::identity(text);
    let result = host
        .parse_condition_in_parse(
            &transaction,
            context(1),
            ConditionParseRequest {
                source: &source,
                range: TextRange::new(0, text.len()),
                context: parser_context(),
            },
            ConditionParserConfig::default(),
        )
        .expect("Condition must parse through CoreLibrary Expression candidates");

    let selected = result.matches.selected.expect("Condition must be selected");
    assert!(matches!(
        selected.node.kind,
        ConditionNodeKind::Registered { .. }
    ));
    assert_eq!(selected.node.expressions.len(), 1);
    assert!(result.calls.iter().any(|call| {
        call.component_id == "nlaocs.core-library"
            && call.subscription_id == "core.expression-candidates"
    }));
    transaction.cancel().unwrap();
}

#[test]
fn unknown_condition_rolls_back_the_parse_overlay() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/condition.sk", 2)
        .unwrap();
    let before = transaction.state_revision().unwrap();
    let text = "this condition does not exist";
    let source = MappedSource::identity(text);
    let result = host
        .parse_condition_in_parse(
            &transaction,
            context(2),
            ConditionParseRequest {
                source: &source,
                range: TextRange::new(0, text.len()),
                context: parser_context(),
            },
            ConditionParserConfig::default(),
        )
        .expect("unknown Condition remains recoverable");

    assert!(result.matches.selected.is_none());
    assert!(result.matches.unknown.is_some());
    assert_eq!(transaction.state_revision().unwrap(), before);
    transaction.cancel().unwrap();
}

#[test]
fn whether_and_ternary_parse_condition_captures_through_core_library() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/condition.sk", 3)
        .unwrap();

    let whether = "whether dummy fixture condition";
    let source = MappedSource::identity(whether);
    let parsed = host
        .parse_expression_in_parse(
            &transaction,
            context(3),
            ExpressionParseRequest {
                source: &source,
                range: TextRange::new(0, whether.len()),
                expected_types: vec![skript_parser::ExpressionExpectedType {
                    class_name: ClassName("java.lang.Boolean".to_owned()),
                    plural: false,
                }],
                context: parser_context(),
            },
            ExpressionParserConfig::default(),
        )
        .expect("whether Expression pipeline must run")
        .matches
        .selected
        .expect("whether Expression must parse");
    assert_eq!(parsed.node.conditions.len(), 1);
    assert_eq!(parsed.node.multiplicity, Some(Multiplicity::Single));

    let ternary = "\"yes\" if dummy fixture condition else \"no\"";
    let source = MappedSource::identity(ternary);
    let ternary_matches = host
        .parse_expression_in_parse(
            &transaction,
            context(3),
            ExpressionParseRequest {
                source: &source,
                range: TextRange::new(0, ternary.len()),
                expected_types: vec![skript_parser::ExpressionExpectedType {
                    class_name: ClassName("java.lang.String".to_owned()),
                    plural: false,
                }],
                context: parser_context(),
            },
            ExpressionParserConfig::default(),
        )
        .expect("ternary Expression pipeline must run")
        .matches;
    let parsed = ternary_matches.selected.unwrap_or_else(|| {
        panic!(
            "ternary Expression must parse: {:#?}",
            ternary_matches.failure
        )
    });
    assert_eq!(parsed.node.conditions.len(), 1);
    assert_eq!(parsed.node.children.len(), 2);
    assert_eq!(
        parsed.node.return_type.as_ref().map(ClassName::as_str),
        Some("java.lang.String")
    );
    assert_eq!(parsed.node.multiplicity, Some(Multiplicity::Single));
    transaction.cancel().unwrap();
}

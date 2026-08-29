use parser_wasm::host::{HostConfig, InvocationContext, ParserHost};
use skript_parser::{
    ExpressionExpectedType, ExpressionNode, ExpressionNodeKind, ExpressionParseContext,
    ExpressionParseRequest, ExpressionParserConfig, MappedSource, PatternCapture, TextRange,
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

fn full_catalog() -> Arc<Catalog> {
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

fn parse_fixture_expression(text: &str, revision: u64) -> ExpressionNode {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(full_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
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
                expected_types: vec![ExpressionExpectedType {
                    class_name: ClassName("java.lang.Object".to_owned()),
                    plural: true,
                }],
                context: parser_context(),
            },
            ExpressionParserConfig::default(),
        )
        .unwrap_or_else(|error| panic!("fixture Expression parse failed: {error:?}"));
    let selected = result.matches.selected.unwrap_or_else(|| {
        panic!(
            "fixture Expression was not selected: failure={:#?}, alternatives={:#?}",
            result.matches.failure, result.matches.alternatives
        )
    });
    let node = selected.node;
    transaction.cancel().unwrap();
    node
}

fn assert_registered(node: &ExpressionNode) {
    assert!(matches!(node.kind, ExpressionNodeKind::Registered { .. }));
}

fn assert_return(node: &ExpressionNode, class_name: &str, multiplicity: syntaxes::Multiplicity) {
    assert_eq!(
        node.return_type.as_ref().map(ClassName::as_str),
        Some(class_name)
    );
    assert_eq!(node.multiplicity, Some(multiplicity));
}

#[test]
fn random_integer_expression_uses_amount_for_multiplicity() {
    let single = parse_fixture_expression("1 random integer between 1 and 2", 30);
    assert_registered(&single);
    assert_return(&single, "java.lang.Long", syntaxes::Multiplicity::Single);
    assert_eq!(single.children.len(), 3);
    assert!(single.children.iter().all(|child| {
        matches!(
            child.kind,
            ExpressionNodeKind::Literal { ref parser_id }
                if parser_id == "core.literal.number"
        )
    }));

    let multiple = parse_fixture_expression("2 random integers between 1 and 2", 31);
    assert_registered(&multiple);
    assert_return(
        &multiple,
        "java.lang.Long",
        syntaxes::Multiplicity::Multiple,
    );
    assert_eq!(multiple.children.len(), 3);
    assert!(multiple.children.iter().all(|child| {
        matches!(
            child.kind,
            ExpressionNodeKind::Literal { ref parser_id }
                if parser_id == "core.literal.number"
        )
    }));
}

#[test]
fn any_of_all_players_collapses_a_list_to_one_player() {
    let node = parse_fixture_expression("any of all players", 32);
    assert_registered(&node);
    assert_return(
        &node,
        "org.bukkit.entity.Player",
        syntaxes::Multiplicity::Single,
    );
    assert_eq!(node.children.len(), 1);
    let source = &node.children[0];
    assert_registered(source);
    assert_return(
        source,
        "org.bukkit.entity.Player",
        syntaxes::Multiplicity::Multiple,
    );
}

#[test]
fn all_banned_ips_resolves_to_multiple_strings() {
    let node = parse_fixture_expression("all banned ips", 33);
    assert_registered(&node);
    assert_return(&node, "java.lang.String", syntaxes::Multiplicity::Multiple);
    assert!(node.children.is_empty());
}

#[test]
fn join_expression_keeps_grouped_list_and_delimiter_children() {
    let node = parse_fixture_expression("join (\"a\", \"b\") with \",\"", 34);
    assert_registered(&node);
    assert_return(&node, "java.lang.String", syntaxes::Multiplicity::Single);
    assert_eq!(node.children.len(), 2);
    assert!(matches!(node.children[0].kind, ExpressionNodeKind::Grouped));
    assert!(matches!(
        node.children[1].kind,
        ExpressionNodeKind::Literal { ref parser_id }
            if parser_id == "core.literal.string"
    ));
    let grouped = &node.children[0];
    assert_eq!(grouped.children.len(), 1);
    assert!(matches!(
        grouped.children[0].kind,
        ExpressionNodeKind::List { .. }
    ));
    assert_eq!(grouped.children[0].children.len(), 2);
    assert!(grouped.children[0].children.iter().all(|child| {
        matches!(
            child.kind,
            ExpressionNodeKind::Literal { ref parser_id }
                if parser_id == "core.literal.string"
        )
    }));
}

#[test]
fn default_value_expression_resolves_to_one_object() {
    let node = parse_fixture_expression("{_x} otherwise \"fallback\"", 35);
    assert_registered(&node);
    assert_return(&node, "java.lang.Object", syntaxes::Multiplicity::Single);
    assert_eq!(node.children.len(), 2);
    assert!(matches!(
        node.children[0].kind,
        ExpressionNodeKind::Variable { ref parser_id }
            if parser_id == "core.variable"
    ));
    assert!(matches!(
        node.children[1].kind,
        ExpressionNodeKind::Literal { ref parser_id }
            if parser_id == "core.literal.string"
    ));
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
                && call.subscription_id == "core.expression-candidates"
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
fn variable_strings_and_variable_names_parse_embedded_expressions() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(expression_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    for (revision, text, expected, parser_id) in [
        (
            20,
            "\"value: %42%\"",
            "java.lang.String",
            "core.literal.variable-string",
        ),
        (21, "{data::%42%}", "java.lang.Object", "core.variable"),
    ] {
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
                    expected_types: vec![ExpressionExpectedType {
                        class_name: ClassName(expected.to_owned()),
                        plural: true,
                    }],
                    context: parser_context(),
                },
                ExpressionParserConfig::default(),
            )
            .expect("interpolated leaf must parse");
        let selected = result
            .matches
            .selected
            .unwrap_or_else(|| {
                panic!(
                    "leaf must be selected: failure={:#?}, effects={:#?}, calls={:#?}, component_failures={:#?}",
                    result.matches.failure, result.effects, result.calls, result.failures
                )
            });
        assert!(matches!(
            selected.node.kind,
            ExpressionNodeKind::Literal { parser_id: ref actual }
                | ExpressionNodeKind::Variable { parser_id: ref actual }
                if actual == parser_id
        ));
        assert_eq!(selected.node.children.len(), 1);
        let embedded = &selected.node.children[0];
        assert!(matches!(
            embedded.kind,
            ExpressionNodeKind::Literal { ref parser_id }
                if parser_id == "core.literal.number"
        ));
        let embedded_start = text.find("42").unwrap();
        assert_eq!(
            embedded.span.mapped.virtual_range,
            TextRange::new(embedded_start, embedded_start + 2)
        );
        assert_eq!(result.effects.parse_results.len(), 1);
        assert!(
            result
                .calls
                .iter()
                .filter(|call| {
                    call.component_id == "nlaocs.core-library"
                        && call.subscription_id == "core.expression-candidates"
                })
                .count()
                >= 2
        );
        transaction.cancel().unwrap();
    }
}

#[test]
fn variable_string_rebases_registered_expression_from_nonzero_parse_result_root() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(Arc::new(
                ssg::load(fixture())
                    .expect("schema 3 fixture must load")
                    .catalog()
                    .clone(),
            )),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/expression.sk", 22)
        .unwrap();
    let text = "\"players: %size of all players%\"";
    let source = MappedSource::identity(text);
    let result = host
        .parse_expression_in_parse(
            &transaction,
            context(22),
            ExpressionParseRequest {
                source: &source,
                range: TextRange::new(0, text.len()),
                expected_types: vec![expected_type("java.lang.String")],
                context: parser_context(),
            },
            ExpressionParserConfig::default(),
        )
        .expect("interpolated string must parse");
    let selected = result
        .matches
        .selected
        .unwrap_or_else(|| {
            panic!(
                "outer variable-string Expression must be selected: failure={:#?}, effects={:#?}, calls={:#?}, component_failures={:#?}",
                result.matches.failure, result.effects, result.calls, result.failures
            )
        });
    assert!(matches!(
        selected.node.kind,
        ExpressionNodeKind::Literal { ref parser_id }
            if parser_id == "core.literal.variable-string"
    ));
    assert_eq!(selected.node.children.len(), 1);

    // `size of all players` is a registered Expression nested inside the
    // string. Its parse-result arena also contains the `player` ClassInfo
    // capture first, so the registered Expression root must not be node 0.
    let embedded = &selected.node.children[0];
    assert!(matches!(
        embedded.kind,
        ExpressionNodeKind::Registered { .. }
    ));
    let embedded_start = text.find("size of all players").unwrap();
    assert_eq!(
        embedded.span.mapped.virtual_range,
        TextRange::new(embedded_start, embedded_start + "size of all players".len())
    );
    assert_eq!(result.effects.parse_results.len(), 1);
    assert_eq!(result.effects.parse_results[0].roots.len(), 1);
    assert_ne!(result.effects.parse_results[0].roots[0], 0);
    let players_start = text
        .rfind("players")
        .expect("nested players literal exists");
    let players_range = TextRange::new(players_start, players_start + "players".len());
    let entities = &embedded.children[0];
    let literal_players = &entities.children[0];
    assert_eq!(literal_players.span.mapped.virtual_range, players_range);
    let PatternCapture::TypeExpression { span, .. } = &entities.captures[0] else {
        panic!("all players must expose its typed PatternCapture");
    };
    assert_eq!(span.mapped.virtual_range, players_range);
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

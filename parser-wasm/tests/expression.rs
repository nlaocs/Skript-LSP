use parser_wasm::host::{HostConfig, InvocationContext, ParserHost, RuntimeProfile};
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
            Syntax::Type(value) => {
                matches!(value.code_name.as_str(), "string" | "number" | "object")
            }
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
        classes: source.classes().to_vec(),
        aliases: source.aliases().clone(),
        plural_rules: source.plural_rules().clone(),
        language: source
            .language_entries()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect(),
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

fn expression_host_config() -> HostConfig {
    HostConfig {
        syntax_catalog: Some(expression_catalog()),
        runtime_profile: RuntimeProfile {
            skript_version: Some("2.15.4".to_owned()),
            ..RuntimeProfile::default()
        },
        ..HostConfig::default()
    }
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
fn rejected_expression_promotes_only_its_semantic_diagnostic() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(full_catalog()),
            runtime_profile: RuntimeProfile {
                skript_version: Some("2.15.4".to_owned()),
                ..RuntimeProfile::default()
            },
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/expression.sk", 99)
        .unwrap();
    let text = "\"hello\" parsed as string";
    let source = MappedSource::identity(text);
    let result = host
        .parse_expression_in_parse(
            &transaction,
            context(99),
            ExpressionParseRequest {
                source: &source,
                range: TextRange::new(0, text.len()),
                expected_types: vec![expected_type("java.lang.Object")],
                context: parser_context(),
            },
            ExpressionParserConfig::default(),
        )
        .expect("semantic rejection remains recoverable");

    assert!(result.matches.selected.is_none());
    assert_eq!(
        result
            .effects
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "core.expression.semantic-rejected")
            .count(),
        1,
        "failure={:#?}; effects={:#?}",
        result.matches.failure,
        result.effects
    );
    let failure = format!("{:#?}", result.matches.failure);
    assert!(
        failure.contains("parsing text as text is not supported"),
        "{failure}"
    );
    transaction.cancel().unwrap();
}

fn parse_fixture_expression(text: &str, revision: u64) -> ExpressionNode {
    parse_fixture_expression_as(text, "java.lang.Object", true, revision)
}

fn parse_fixture_expression_as(
    text: &str,
    class_name: &str,
    plural: bool,
    revision: u64,
) -> ExpressionNode {
    parse_fixture_expression_as_in_events(text, class_name, plural, &[], revision)
}

fn parse_fixture_expression_as_in_events(
    text: &str,
    class_name: &str,
    plural: bool,
    event_classes: &[&str],
    revision: u64,
) -> ExpressionNode {
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
                    class_name: ClassName(class_name.to_owned()),
                    plural,
                }],
                context: ExpressionParseContext {
                    event_classes: event_classes
                        .iter()
                        .map(|class_name| ClassName((*class_name).to_owned()))
                        .collect(),
                    ..parser_context()
                },
            },
            ExpressionParserConfig::default(),
        )
        .unwrap_or_else(|error| panic!("fixture Expression parse failed: {error:?}"));
    let selected = result.matches.selected.unwrap_or_else(|| {
        panic!(
            "fixture Expression was not selected: failure={:#?}, alternatives={:#?}, effects={:#?}, calls={:#?}, component_failures={:#?}",
            result.matches.failure,
            result.matches.alternatives,
            result.effects,
            result.calls,
            result.failures
        )
    });
    let node = selected.node;
    transaction.cancel().unwrap();
    node
}

#[test]
fn superclass_handler_preserves_unknown_simple_literal_multiplicity() {
    let node = parse_fixture_expression_as("console", "java.lang.Object", false, 35);
    assert_registered(&node);
    assert_return(
        &node,
        "org.bukkit.command.ConsoleCommandSender",
        syntaxes::Multiplicity::Both,
    );
}

#[test]
fn event_value_expression_uses_the_current_event_registration() {
    let node = parse_fixture_expression_as_in_events(
        "damage cause",
        "org.bukkit.event.entity.EntityDamageEvent$DamageCause",
        false,
        &["org.bukkit.event.entity.EntityDamageEvent"],
        36,
    );
    assert_registered(&node);
    assert_return(
        &node,
        "org.bukkit.event.entity.EntityDamageEvent$DamageCause",
        syntaxes::Multiplicity::Single,
    );
}

#[test]
fn event_expression_resolves_player_from_join_event() {
    let node = parse_fixture_expression_as_in_events(
        "event-player",
        "org.bukkit.entity.Player",
        false,
        &["org.bukkit.event.player.PlayerJoinEvent"],
        136,
    );
    assert_registered(&node);
    assert_return(
        &node,
        "org.bukkit.entity.Player",
        syntaxes::Multiplicity::Single,
    );
}

#[test]
fn entity_expression_resolves_player_without_event_prefix() {
    let node = parse_fixture_expression_as_in_events(
        "player",
        "org.bukkit.entity.Player",
        false,
        &["org.bukkit.event.player.PlayerJoinEvent"],
        137,
    );
    assert_registered(&node);
    assert_return(
        &node,
        "org.bukkit.entity.Player",
        syntaxes::Multiplicity::Single,
    );
}

#[test]
fn name_property_resolves_player_from_join_event() {
    let node = parse_fixture_expression_as_in_events(
        "player's name",
        "java.lang.Object",
        true,
        &["org.bukkit.event.player.PlayerJoinEvent"],
        138,
    );
    assert_registered(&node);
}

#[test]
fn interpolated_name_property_inherits_join_event_context() {
    let node = parse_fixture_expression_as_in_events(
        "\"<green>%player's name%\"",
        "java.lang.String",
        false,
        &["org.bukkit.event.player.PlayerJoinEvent"],
        139,
    );
    assert!(matches!(
        node.kind,
        ExpressionNodeKind::Literal { ref parser_id }
            if parser_id == "core.literal.variable-string"
    ));
    assert_eq!(node.children.len(), 1);
    assert_registered(&node.children[0]);
    assert_return(
        &node.children[0],
        "net.kyori.adventure.text.Component",
        syntaxes::Multiplicity::Single,
    );
    assert_eq!(node.children[0].children.len(), 1);
    assert_return(
        &node.children[0].children[0],
        "org.bukkit.entity.Player",
        syntaxes::Multiplicity::Single,
    );
}

#[test]
fn property_source_uses_registered_itemstack_conversion() {
    let node = parse_fixture_expression_as_in_events(
        "name of event-item",
        "java.lang.Object",
        true,
        &["org.bukkit.event.player.PlayerEditBookEvent"],
        140,
    );
    assert_registered(&node);
    assert_eq!(node.multiplicity, Some(syntaxes::Multiplicity::Single));
    assert!(
        node.possible_return_types
            .iter()
            .any(|class_name| class_name.as_str() == "net.kyori.adventure.text.Component"),
        "node={node:#?}"
    );
    assert_eq!(node.children.len(), 1);
    assert_return(
        &node.children[0],
        "org.bukkit.inventory.ItemStack",
        syntaxes::Multiplicity::Single,
    );
}

#[test]
fn regex_mapping_capture_reenters_the_expression_parser() {
    let node = parse_fixture_expression_as(
        "(1, 2) transformed using [input * 2]",
        "java.lang.Number",
        true,
        37,
    );
    assert_registered(&node);
    assert_return(&node, "java.lang.Number", syntaxes::Multiplicity::Multiple);
    assert!(
        node.parsed_captures().iter().any(|capture| {
            capture.binding.parser_id == skript_parser::HOST_EXPRESSION_PARSER_ID
        })
    );
}

#[test]
fn attacked_expression_preserves_the_entity_data_subtype() {
    let node = parse_fixture_expression_as_in_events(
        "the attacked zombie",
        "org.bukkit.entity.Zombie",
        false,
        &["org.bukkit.event.entity.EntityDamageEvent"],
        38,
    );
    assert_registered(&node);
    assert_return(
        &node,
        "org.bukkit.entity.Zombie",
        syntaxes::Multiplicity::Single,
    );
}

fn assert_registered(node: &ExpressionNode) {
    assert!(matches!(node.kind, ExpressionNodeKind::Registered { .. }));
}

fn assert_return(node: &ExpressionNode, class_name: &str, multiplicity: syntaxes::Multiplicity) {
    assert_eq!(
        node.return_type.as_ref().map(ClassName::as_str),
        Some(class_name),
        "node={node:#?}"
    );
    assert_eq!(node.multiplicity, Some(multiplicity), "node={node:#?}");
}

#[test]
fn numeric_literal_is_assignable_to_the_number_parameter_type() {
    let node = parse_fixture_expression_as("1", "java.lang.Number", false, 29);
    assert_return(&node, "java.lang.Long", syntaxes::Multiplicity::Single);
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
fn all_entities_prefers_the_registered_set_expression_over_type_literals() {
    let node = parse_fixture_expression_as("all entities", "java.lang.Object", true, 140);
    assert_registered(&node);
    assert_return(
        &node,
        "org.bukkit.entity.Entity",
        syntaxes::Multiplicity::Multiple,
    );
    assert_eq!(node.children.len(), 1, "node={node:#?}");
    assert_eq!(
        node.metadata
            .get("nlaocs.core-library/semantic-mode")
            .map(String::as_str),
        Some("entities-literal-type")
    );
    assert!(
        matches!(
            &node.children[0].kind,
            ExpressionNodeKind::Literal { parser_id } if parser_id == "core.literal.entity-data"
        ),
        "node={node:#?}"
    );
}

#[test]
fn plural_type_name_parses_as_class_info_literal() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(full_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/expression.sk", 33)
        .unwrap();
    let source = MappedSource::identity("players");
    let result = host
        .parse_expression_in_parse(
            &transaction,
            context(33),
            ExpressionParseRequest {
                source: &source,
                range: TextRange::new(0, source.virtual_source().len()),
                expected_types: vec![expected_type("ch.njol.skript.classes.ClassInfo")],
                context: parser_context(),
            },
            ExpressionParserConfig::default(),
        )
        .unwrap();
    let selected = result.matches.selected.unwrap_or_else(|| {
        panic!(
            "plural ClassInfo literal was not selected: failure={:#?}, effects={:#?}, calls={:#?}, component_failures={:#?}",
            result.matches.failure, result.effects, result.calls, result.failures
        )
    });
    assert!(matches!(
        selected.node.kind,
        ExpressionNodeKind::Literal { ref parser_id }
            if parser_id == "core.literal.class-info"
    ));
    transaction.cancel().unwrap();
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
    let mut host =
        ParserHost::new(CORE_LIBRARY, expression_host_config()).expect("CoreLibrary must load");
    let cases = [
        (
            "\"hello\"",
            "java.lang.String",
            "core.literal.string",
            "core.type-candidates",
        ),
        (
            "{message}",
            "java.lang.String",
            "core.variable",
            "core.expression-candidates",
        ),
        (
            "42",
            "java.lang.Number",
            "core.literal.number",
            "core.type-candidates",
        ),
    ];

    for (index, (text, expected, parser_id, subscription_id)) in cases.into_iter().enumerate() {
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
        let selected = result.matches.selected.unwrap_or_else(|| {
            panic!(
                "{text:?} must select a leaf; failures: {:#?}; calls: {:#?}; component failures: {:#?}",
                result.matches.failure, result.calls, result.failures
            )
        });
        assert!(matches!(
            selected.node.kind,
            ExpressionNodeKind::Literal { parser_id: ref actual }
                | ExpressionNodeKind::Variable { parser_id: ref actual }
                if actual == parser_id
        ));
        assert!(result.calls.iter().any(|call| {
            call.component_id == "nlaocs.core-library" && call.subscription_id == subscription_id
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
    let mut host =
        ParserHost::new(CORE_LIBRARY, expression_host_config()).expect("CoreLibrary must load");
    for (revision, text, expected, parser_id) in [
        (
            20,
            "\"value: %dummy direct registry expression%\"",
            "java.lang.String",
            "core.literal.variable-string",
        ),
        (
            21,
            "{data::%dummy direct registry expression%}",
            "java.lang.Object",
            "core.variable",
        ),
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
            ExpressionNodeKind::Registered { .. }
        ));
        let embedded_start = text.find("dummy direct registry expression").unwrap();
        assert_eq!(
            embedded.span.mapped.virtual_range,
            TextRange::new(
                embedded_start,
                embedded_start + "dummy direct registry expression".len(),
            )
        );
        assert_eq!(result.effects.parse_results.len(), 1);
        let (subscription_id, minimum_calls) = if parser_id == "core.literal.variable-string" {
            ("core.type-candidates", 1)
        } else {
            ("core.expression-candidates", 2)
        };
        assert!(
            result
                .calls
                .iter()
                .filter(|call| {
                    call.component_id == "nlaocs.core-library"
                        && call.subscription_id == subscription_id
                })
                .count()
                >= minimum_calls
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
    let mut host =
        ParserHost::new(CORE_LIBRARY, expression_host_config()).expect("CoreLibrary must load");
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

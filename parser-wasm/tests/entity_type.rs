use parser_wasm::WasmExpressionParseResult;
use parser_wasm::host::{HostConfig, InvocationContext, ParserHost, RuntimeProfile};
use skript_parser::{
    ExpressionExpectedType, ExpressionNode, ExpressionNodeKind, ExpressionParseContext,
    ExpressionParseRequest, ExpressionParserConfig, ExpressionRootMode, MappedSource, TextRange,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use syntaxes::{Catalog, CatalogParts, ClassName, Multiplicity, Syntax};

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));
const ENTITY_DATA: &str = "ch.njol.skript.entity.EntityData";
const ENTITY_TYPE: &str = "ch.njol.skript.entity.EntityType";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn entity_type_catalog() -> Arc<Catalog> {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let source = snapshot.catalog();
    let source_view = source.source().cloned().expect("SSG source view");
    let mut syntaxes = source.syntaxes().to_vec();
    let mut found_entity_data = false;
    let mut found_entity_type = false;

    for syntax in &mut syntaxes {
        let Syntax::Type(value) = syntax else {
            continue;
        };
        if value.original_class.as_str() == ENTITY_DATA {
            // This synthetic value stands in for a ClassInfo.supplier result;
            // it does not execute Java runtime code. Keeping the full catalog
            // and its source view preserves the integration routing exercised here.
            value.literal_values.push("zombie".to_owned());
            found_entity_data = true;
        }
        if value.original_class.as_str() == ENTITY_TYPE {
            found_entity_type = true;
        }
    }

    assert!(found_entity_data, "fixture must contain EntityData");
    assert!(found_entity_type, "fixture must contain EntityType");

    Arc::new(
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
        .with_unchecked_source(source_view),
    )
}

fn legacy_entity_type_catalog() -> Arc<Catalog> {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let source = snapshot.catalog();
    let mut syntaxes = source.syntaxes().to_vec();
    let mut found_entity_data = false;
    let mut found_entity_type = false;

    for syntax in &mut syntaxes {
        let Syntax::Type(value) = syntax else {
            continue;
        };
        if value.original_class.as_str() == ENTITY_DATA {
            // Model a 2.6.4 catalog: EntityData has neither supplier flag nor
            // exported literals. The runtime profile below supplies the legacy
            // compatibility context independently of this synthetic catalog.
            value.literal_values.clear();
            value.type_literals.clear();
            value.has_supplier = false;
            found_entity_data = true;
        }
        if value.original_class.as_str() == ENTITY_TYPE {
            found_entity_type = true;
        }
    }

    assert!(found_entity_data, "fixture must contain EntityData");
    assert!(found_entity_type, "fixture must contain EntityType");

    Arc::new(Catalog::new(CatalogParts {
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

fn parser_context() -> ExpressionParseContext {
    ExpressionParseContext {
        syntax_context: 7,
        ..ExpressionParseContext::default()
    }
}

fn expected_type(class_name: &str) -> ExpressionExpectedType {
    ExpressionExpectedType {
        class_name: ClassName(class_name.to_owned()),
        plural: false,
    }
}

fn parse_expression(
    text: &str,
    class_name: &str,
    catalog: Arc<Catalog>,
    revision: u64,
    config: ExpressionParserConfig,
) -> WasmExpressionParseResult {
    parse_expression_with_profile(
        text,
        class_name,
        catalog,
        revision,
        config,
        RuntimeProfile {
            skript_version: Some("2.15.4".to_owned()),
            ..RuntimeProfile::default()
        },
    )
}

fn parse_expression_with_profile(
    text: &str,
    class_name: &str,
    catalog: Arc<Catalog>,
    revision: u64,
    config: ExpressionParserConfig,
    runtime_profile: RuntimeProfile,
) -> WasmExpressionParseResult {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog),
            runtime_profile,
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
                expected_types: vec![expected_type(class_name)],
                context: parser_context(),
            },
            config,
        )
        .unwrap_or_else(|error| panic!("EntityType parse failed: {error:?}"));
    transaction.cancel().unwrap();
    result
}

fn selected_node(result: WasmExpressionParseResult, label: &str) -> ExpressionNode {
    result.matches.selected.unwrap_or_else(|| {
        panic!(
            "{label} was not selected: failure={:#?}, alternatives={:#?}, effects={:#?}, calls={:#?}, component_failures={:#?}",
            result.matches.failure,
            result.matches.alternatives,
            result.effects,
            result.calls,
            result.failures
        )
    })
    .node
}

fn metadata<'a>(node: &'a ExpressionNode, key: &str) -> &'a str {
    let namespaced_key = format!("nlaocs.core-library/{key}");
    node.metadata
        .get(&namespaced_key)
        .map(String::as_str)
        .unwrap_or_else(|| panic!("node metadata is missing {namespaced_key:?}: {node:#?}"))
}

fn entity_data_metadata(node: &ExpressionNode) -> serde_json::Value {
    serde_json::from_str(metadata(node, "entity-data"))
        .unwrap_or_else(|error| panic!("entity-data metadata is not valid JSON: {error}"))
}

fn assert_entity_data_metadata(
    node: &ExpressionNode,
    expected_plural: bool,
    expected_range: Option<(&str, &str)>,
) {
    let value = entity_data_metadata(node);
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("entity-data metadata is not a JSON object: {value}"));
    let string = |key: &str| object.get(key).and_then(|value| value.as_str());

    assert_eq!(string("type-code-name"), Some("entitydata"));
    assert_eq!(string("literal-canonical"), Some("zombie"));
    assert_eq!(string("literal-source"), Some("supplier"));
    assert_eq!(
        string("literal-plural"),
        Some(if expected_plural { "true" } else { "false" })
    );
    if let Some((start, end)) = expected_range {
        assert_eq!(string("literal-range-start"), Some(start));
        assert_eq!(string("literal-range-end"), Some(end));
    }
}

#[test]
fn entity_type_literal_parses_amount_three_and_supplier_plural() {
    let node = selected_node(
        parse_expression(
            "3 zombies",
            ENTITY_TYPE,
            entity_type_catalog(),
            201,
            ExpressionParserConfig::default(),
        ),
        "amount-prefixed EntityType literal",
    );

    assert!(matches!(
        &node.kind,
        ExpressionNodeKind::Literal { parser_id }
            if parser_id == "core.literal.entity-type"
    ));
    assert_eq!(
        node.return_type.as_ref().map(ClassName::as_str),
        Some(ENTITY_TYPE)
    );
    assert_eq!(node.multiplicity, Some(Multiplicity::Single));
    assert_eq!(metadata(&node, "type-code-name"), "entitytype");
    assert_eq!(metadata(&node, "entity-type-amount"), "3");
    assert_eq!(metadata(&node, "entity-type-raw-amount"), "3");
    assert_entity_data_metadata(&node, true, Some(("2", "9")));
}

#[test]
fn entity_type_literal_accepts_repeated_ascii_spaces() {
    let node = selected_node(
        parse_expression(
            "2  zombies",
            ENTITY_TYPE,
            entity_type_catalog(),
            202,
            ExpressionParserConfig::default(),
        ),
        "repeated-space EntityType literal",
    );

    assert_eq!(metadata(&node, "entity-type-amount"), "2");
    assert_eq!(metadata(&node, "entity-type-raw-amount"), "2");
    assert_entity_data_metadata(&node, true, None);
}

#[test]
fn entity_type_active_type_routing_handles_zero_articles_and_overflow() {
    for (revision, text, amount, raw_amount, plural) in [
        (205, "0 zombies", "0", "0", true),
        (206, "a zombie", "1", "-1", false),
        (207, "an zombie", "1", "-1", false),
        (208, "2147483648 zombies", "2147483647", "2147483647", true),
    ] {
        let node = selected_node(
            parse_expression(
                text,
                ENTITY_TYPE,
                entity_type_catalog(),
                revision,
                ExpressionParserConfig::default(),
            ),
            text,
        );
        assert!(matches!(
            &node.kind,
            ExpressionNodeKind::Literal { parser_id }
                if parser_id == "core.literal.entity-type"
        ));
        assert_eq!(metadata(&node, "entity-type-amount"), amount, "{text}");
        assert_eq!(
            metadata(&node, "entity-type-raw-amount"),
            raw_amount,
            "{text}"
        );
        assert_entity_data_metadata(&node, plural, None);
    }
}

#[test]
fn entity_type_active_type_routing_rejects_negative_quantity() {
    let result = parse_expression(
        "-2 zombies",
        ENTITY_TYPE,
        entity_type_catalog(),
        209,
        ExpressionParserConfig::default(),
    );

    assert!(
        result.matches.selected.is_none(),
        "negative quantity was accepted as EntityType: {result:#?}"
    );
}

#[test]
fn entity_type_literals_only_rejects_unicode_space_and_second_article() {
    for (revision, text) in [
        (210, "3 \u{00a0}zombies"),
        (211, "3 \u{2003}zombies"),
        (212, "3 a zombie"),
    ] {
        let result = parse_expression(
            text,
            ENTITY_TYPE,
            entity_type_catalog(),
            revision,
            ExpressionParserConfig {
                root_mode: ExpressionRootMode::LiteralsOnly,
                ..ExpressionParserConfig::default()
            },
        );

        assert!(
            result.matches.selected.is_none(),
            "invalid literal-only EntityType input was accepted: {text:?}: {result:#?}"
        );
    }
}

#[test]
fn entity_type_supplier_whitespace_follows_skript_java_trim_boundary() {
    let legacy = parse_expression_with_profile(
        "3 \tzombies",
        ENTITY_TYPE,
        entity_type_catalog(),
        214,
        ExpressionParserConfig::default(),
        RuntimeProfile {
            skript_version: Some("2.9.5".to_owned()),
            ..RuntimeProfile::default()
        },
    );
    assert!(
        legacy.matches.selected.is_none(),
        "2.9.5 accepted tab whitespace before EntityData: {legacy:#?}"
    );

    let modern = selected_node(
        parse_expression_with_profile(
            "3 \tzombies",
            ENTITY_TYPE,
            entity_type_catalog(),
            215,
            ExpressionParserConfig::default(),
            RuntimeProfile {
                skript_version: Some("2.10.2".to_owned()),
                ..RuntimeProfile::default()
            },
        ),
        "2.10.2 Java-trimmed EntityType literal",
    );
    assert_eq!(metadata(&modern, "entity-type-amount"), "3");
    assert_eq!(metadata(&modern, "entity-type-raw-amount"), "3");
    assert_entity_data_metadata(&modern, true, None);
}

#[test]
fn entity_type_legacy_fallback_works_without_entity_data_supplier() {
    let node = selected_node(
        parse_expression_with_profile(
            "zombie",
            ENTITY_TYPE,
            legacy_entity_type_catalog(),
            213,
            ExpressionParserConfig::default(),
            RuntimeProfile {
                snapshot_schema_version: Some(5),
                minecraft_version: Some("1.12.2".to_owned()),
                skript_version: Some("2.6.4".to_owned()),
                ..RuntimeProfile::default()
            },
        ),
        "legacy EntityType literal",
    );

    assert!(matches!(
        &node.kind,
        ExpressionNodeKind::Literal { parser_id }
            if parser_id == "core.literal.entity-type"
    ));
    assert_eq!(metadata(&node, "entity-type-amount"), "1");
    assert_eq!(metadata(&node, "entity-type-raw-amount"), "-1");

    let value = entity_data_metadata(&node);
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("legacy entity-data metadata is not an object: {value}"));
    let string = |key: &str| object.get(key).and_then(|value| value.as_str());
    assert_eq!(string("entity-class"), Some("org.bukkit.entity.Zombie"));
    assert_eq!(string("entity-plural"), Some("false"));
    assert_eq!(string("entity-source"), Some("core.legacy-compatibility"));
    assert!(!object.contains_key("literal-source"));
}

#[test]
fn entity_type_dispatch_preserves_catalog_registration_identity() {
    let catalog = entity_type_catalog();
    let (definition_id, registration_id) = catalog
        .types()
        .find(|value| value.original_class.as_str() == ENTITY_TYPE)
        .map(|value| {
            (
                value.definition_id.as_str().to_owned(),
                value.registration_id.as_str().to_owned(),
            )
        })
        .expect("fixture must contain the EntityType catalog registration");

    let node = selected_node(
        parse_expression(
            "zombie",
            ENTITY_TYPE,
            catalog,
            203,
            ExpressionParserConfig::default(),
        ),
        "typed EntityType literal",
    );

    assert!(matches!(
        &node.kind,
        ExpressionNodeKind::Literal { parser_id }
            if parser_id == "core.literal.entity-type"
    ));
    assert_eq!(metadata(&node, "type-definition-id"), definition_id);
    assert_eq!(metadata(&node, "type-registration-id"), registration_id);
}

#[test]
fn unrelated_typed_expression_does_not_receive_entity_type_literal() {
    let result = parse_expression(
        "3 zombies",
        "java.lang.String",
        entity_type_catalog(),
        204,
        ExpressionParserConfig {
            root_mode: ExpressionRootMode::LiteralsOnly,
            ..ExpressionParserConfig::default()
        },
    );

    assert!(
        result.matches.selected.is_none(),
        "EntityType literal was selected for an unrelated expected type: {result:#?}"
    );
}

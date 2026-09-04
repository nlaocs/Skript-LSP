use parser_wasm::WasmExpressionParseResult;
use parser_wasm::host::{HostConfig, InvocationContext, ParserHost};
use skript_parser::{
    ExpressionExpectedType, ExpressionNode, ExpressionNodeKind, ExpressionParseContext,
    ExpressionParseRequest, ExpressionParserConfig, MappedSource, TextRange,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use syntaxes::{Catalog, ClassName, Multiplicity};

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));

const TIMESPAN_CLASS: &str = "ch.njol.skript.util.Timespan";
const ENTITY_DATA_CLASS: &str = "ch.njol.skript.entity.EntityData";
const ENTITY_TYPE_CLASS: &str = "ch.njol.skript.entity.EntityType";
const ENCHANTMENT_TYPE_CLASS: &str = "ch.njol.skript.util.EnchantmentType";

struct SnapshotFixture {
    name: &'static str,
    directory: &'static str,
    skript_version: &'static str,
    minecraft_version: &'static str,
    snapshot_id: &'static str,
    content_digest: &'static str,
    modern_language: bool,
    modern_supplier_literals: bool,
    types: &'static [TypeExpectation],
}

struct TypeExpectation {
    code_name: &'static str,
    source_index: usize,
    type_parse_order: usize,
    original_class: &'static str,
    definition_id: &'static str,
    parser_class: &'static str,
    has_supplier: bool,
    type_literal_count: usize,
}

const TIMESPAN_DEFINITION_ID: &str =
    "type:skript:d756f0428ee8bce26bb28851b5c118557a0df03dc060fc4335d8e40541ba4ea5";
const ENTITYDATA_DEFINITION_ID: &str =
    "type:skript:99404dd7dbbe109836a17d1933b1e35a6a94af29bd9db31e1b84b65206bcea61";
const ENTITYTYPE_DEFINITION_ID: &str =
    "type:skript:4ec39490fed64bf7dd2bce9389abf099819a972edb1e9aa03e88ebed7109af03";

const LEGACY_TYPES: &[TypeExpectation] = &[
    TypeExpectation {
        code_name: "timespan",
        source_index: 44,
        type_parse_order: 44,
        original_class: TIMESPAN_CLASS,
        definition_id: TIMESPAN_DEFINITION_ID,
        parser_class: "ch.njol.skript.classes.data.SkriptClasses$9",
        has_supplier: false,
        type_literal_count: 0,
    },
    TypeExpectation {
        code_name: "entitydata",
        source_index: 48,
        type_parse_order: 48,
        original_class: ENTITY_DATA_CLASS,
        definition_id: ENTITYDATA_DEFINITION_ID,
        parser_class: "ch.njol.skript.entity.EntityData$2",
        has_supplier: false,
        type_literal_count: 0,
    },
    TypeExpectation {
        code_name: "entitytype",
        source_index: 50,
        type_parse_order: 50,
        original_class: ENTITY_TYPE_CLASS,
        definition_id: ENTITYTYPE_DEFINITION_ID,
        parser_class: "ch.njol.skript.entity.EntityType$2",
        has_supplier: false,
        type_literal_count: 0,
    },
];

const MODERN_2154_TYPES: &[TypeExpectation] = &[
    TypeExpectation {
        code_name: "timespan",
        source_index: 101,
        type_parse_order: 101,
        original_class: TIMESPAN_CLASS,
        definition_id: TIMESPAN_DEFINITION_ID,
        parser_class: "ch.njol.skript.classes.data.SkriptClasses$5",
        has_supplier: false,
        type_literal_count: 0,
    },
    TypeExpectation {
        code_name: "entitydata",
        source_index: 107,
        type_parse_order: 107,
        original_class: ENTITY_DATA_CLASS,
        definition_id: ENTITYDATA_DEFINITION_ID,
        parser_class: "ch.njol.skript.entity.EntityData$2",
        has_supplier: true,
        type_literal_count: 167,
    },
    TypeExpectation {
        code_name: "entitytype",
        source_index: 108,
        type_parse_order: 108,
        original_class: ENTITY_TYPE_CLASS,
        definition_id: ENTITYTYPE_DEFINITION_ID,
        parser_class: "ch.njol.skript.entity.EntityType$1",
        has_supplier: false,
        type_literal_count: 0,
    },
];

const MODERN_2160_TYPES: &[TypeExpectation] = &[
    TypeExpectation {
        code_name: "timespan",
        source_index: 104,
        type_parse_order: 104,
        original_class: TIMESPAN_CLASS,
        definition_id: TIMESPAN_DEFINITION_ID,
        parser_class: "ch.njol.skript.classes.data.SkriptClasses$5",
        has_supplier: false,
        type_literal_count: 0,
    },
    TypeExpectation {
        code_name: "entitydata",
        source_index: 110,
        type_parse_order: 110,
        original_class: ENTITY_DATA_CLASS,
        definition_id: ENTITYDATA_DEFINITION_ID,
        parser_class: "ch.njol.skript.entity.EntityData$2",
        has_supplier: true,
        type_literal_count: 167,
    },
    TypeExpectation {
        code_name: "entitytype",
        source_index: 111,
        type_parse_order: 111,
        original_class: ENTITY_TYPE_CLASS,
        definition_id: ENTITYTYPE_DEFINITION_ID,
        parser_class: "ch.njol.skript.entity.EntityType$1",
        has_supplier: false,
        type_literal_count: 0,
    },
];

const FIXTURES: &[SnapshotFixture] = &[
    SnapshotFixture {
        name: "Skript 2.6.4 on Minecraft 1.12.2",
        directory: "skript-2.6.4-mc-1.12.2",
        skript_version: "2.6.4",
        minecraft_version: "1.12.2",
        snapshot_id: "8c4b52a721c73f2ea7750da6f7da036644da7d364d4ba90bdd42e1f7fb02a510",
        content_digest: "4cbcc106b772f4a1540527690298fb49ce90a25ec522b56feabd726d4350c5be",
        modern_language: false,
        modern_supplier_literals: false,
        types: LEGACY_TYPES,
    },
    SnapshotFixture {
        name: "Skript 2.15.4",
        directory: "skript-2.15.4",
        skript_version: "2.15.4",
        minecraft_version: "1.21.11",
        snapshot_id: "e945d534fab4aa2722b603445af9e0f885995b6d436249ec71fe4f37c517532e",
        content_digest: "a2204b2718a6d4bdd2ae54d75e184c16b8d32dfd1486ff50318a8d1e1aa0e4f5",
        modern_language: true,
        modern_supplier_literals: true,
        types: MODERN_2154_TYPES,
    },
    SnapshotFixture {
        name: "Skript 2.16.0",
        directory: "skript-2.16.0",
        skript_version: "2.16.0",
        minecraft_version: "1.21.11",
        snapshot_id: "572c548cc2ef5e327f76688d80137a3c9ed4f74de5738b5d627b121886533ed4",
        content_digest: "102e26933d2731c05fb45a7599c28b3e9f56d1e762c5cecbe73a5f2cdab71bdf",
        modern_language: true,
        modern_supplier_literals: true,
        types: MODERN_2160_TYPES,
    },
];

fn fixture_path(fixture: &SnapshotFixture) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/type-parser-versions")
        .join(fixture.directory)
}

fn context(revision: u64) -> InvocationContext {
    InvocationContext {
        invocation_id: revision,
        subscription_id: String::new(),
        document_id: "file:///workspace/type-parser-versions.sk".to_owned(),
        document_revision: revision,
        expansion: None,
        syntax_context: 3,
    }
}

fn parse_typed(
    host: &mut ParserHost,
    text: &str,
    expected_class: &str,
    revision: u64,
) -> WasmExpressionParseResult {
    let transaction = host
        .begin_parse(
            "file:///workspace",
            "file:///workspace/type-parser-versions.sk",
            revision,
        )
        .expect("parse must begin");
    let source = MappedSource::identity(text);
    let result = host
        .parse_expression_in_parse(
            &transaction,
            context(revision),
            ExpressionParseRequest {
                source: &source,
                range: TextRange::new(0, text.len()),
                expected_types: vec![ExpressionExpectedType {
                    class_name: ClassName(expected_class.to_owned()),
                    plural: false,
                }],
                context: ExpressionParseContext {
                    syntax_context: 3,
                    ..ExpressionParseContext::default()
                },
            },
            ExpressionParserConfig::default(),
        )
        .unwrap_or_else(|error| panic!("typed expression parsing failed for {text:?}: {error:?}"));
    transaction.cancel().expect("test parse may be cancelled");
    result
}

fn selected_node(result: WasmExpressionParseResult, label: &str) -> ExpressionNode {
    result.matches.selected.unwrap_or_else(|| {
        panic!(
            "{label} was not selected: failure={:#?}, alternatives={:#?}, calls={:#?}, failures={:#?}",
            result.matches.failure,
            result.matches.alternatives,
            result.calls,
            result.failures
        )
    }).node
}

fn metadata<'a>(node: &'a ExpressionNode, key: &str) -> Option<&'a str> {
    node.metadata
        .get(&format!("nlaocs.core-library/{key}"))
        .map(String::as_str)
}

fn json_metadata(node: &ExpressionNode, key: &str) -> serde_json::Value {
    serde_json::from_str(
        metadata(node, key)
            .unwrap_or_else(|| panic!("node is missing metadata {key:?}: {node:#?}")),
    )
    .unwrap_or_else(|error| panic!("metadata {key:?} is not JSON: {error}"))
}

fn load_snapshot(fixture: &SnapshotFixture) -> ssg::Snapshot {
    let snapshot = ssg::load(fixture_path(fixture))
        .unwrap_or_else(|error| panic!("{} fixture must load: {error}", fixture.name));
    let manifest = snapshot.manifest();
    assert_eq!(manifest.schema_version, 5, "{}", fixture.name);
    assert_eq!(
        manifest.snapshot_id, fixture.snapshot_id,
        "{}",
        fixture.name
    );
    assert_eq!(
        manifest.content_digest, fixture.content_digest,
        "{}",
        fixture.name
    );
    assert_eq!(manifest.language, "english", "{}", fixture.name);
    assert_eq!(
        manifest.server.minecraft_version, fixture.minecraft_version,
        "{}",
        fixture.name
    );
    assert_eq!(
        manifest.files,
        ssg::ALL_FILES
            .iter()
            .map(|file| (*file).to_owned())
            .collect::<Vec<_>>(),
        "{}",
        fixture.name
    );
    assert_eq!(
        manifest
            .plugins
            .iter()
            .map(|plugin| (plugin.load_order, plugin.name.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (0, "SkriptSyntaxGenerator"),
            (1, "Skript"),
            (2, "SkriptDummyAddon")
        ],
        "{}",
        fixture.name
    );
    let skript = manifest
        .plugins
        .iter()
        .find(|plugin| plugin.name == "Skript")
        .expect("SSG manifest must contain Skript");
    assert_eq!(skript.version, fixture.skript_version, "{}", fixture.name);

    let catalog = snapshot.catalog();
    assert_eq!(
        catalog.language_value("and"),
        Some("and"),
        "{}",
        fixture.name
    );
    assert_eq!(
        catalog.language_value("time.second.full").is_some(),
        fixture.modern_language,
        "{}",
        fixture.name
    );
    let source = catalog
        .source()
        .expect("SSG catalog must retain its source");
    assert_eq!(source.schema_version, 5, "{}", fixture.name);
    assert_eq!(source.snapshot_id, fixture.snapshot_id, "{}", fixture.name);
    assert_eq!(
        source.document_names().collect::<Vec<_>>(),
        ssg::ALL_FILES.to_vec(),
        "{}",
        fixture.name
    );
    let runtime = source.runtime.as_ref().expect("SSG source runtime");
    assert_eq!(runtime.minecraft_version, fixture.minecraft_version);
    assert_eq!(runtime.language, "english");
    assert_eq!(
        runtime
            .plugins
            .iter()
            .map(|plugin| {
                (
                    plugin.load_order,
                    plugin.name.as_str(),
                    plugin.version.as_str(),
                )
            })
            .collect::<Vec<_>>(),
        vec![
            (0, "SkriptSyntaxGenerator", "1.0"),
            (1, "Skript", fixture.skript_version),
            (2, "SkriptDummyAddon", "1.1.0")
        ]
    );

    snapshot
}

fn assert_type_metadata(catalog: &Catalog, fixture: &SnapshotFixture) {
    for expected in fixture.types {
        let actual = catalog
            .type_by_code_name(expected.code_name)
            .unwrap_or_else(|| panic!("{} must contain {}", fixture.name, expected.code_name));
        assert_eq!(
            actual.source_index, expected.source_index,
            "{}",
            fixture.name
        );
        assert_eq!(
            actual.type_parse_order, expected.type_parse_order,
            "{} {}",
            fixture.name, expected.code_name
        );
        assert_eq!(
            actual.original_class.as_str(),
            expected.original_class,
            "{} {}",
            fixture.name,
            expected.code_name
        );
        assert_eq!(
            actual.definition_id.as_str(),
            expected.definition_id,
            "{} {}",
            fixture.name,
            expected.code_name
        );
        assert_eq!(
            actual.registration_id.as_str(),
            expected.definition_id,
            "{} {}",
            fixture.name,
            expected.code_name
        );
        assert_eq!(actual.addon.name, "Skript", "{}", fixture.name);
        assert_eq!(
            actual.addon.version, fixture.skript_version,
            "{}",
            fixture.name
        );
        assert_eq!(
            actual.parser_class.as_ref().map(ClassName::as_str),
            Some(expected.parser_class),
            "{} {}",
            fixture.name,
            expected.code_name
        );
        assert!(actual.has_parser, "{} {}", fixture.name, expected.code_name);
        assert_eq!(
            actual.has_supplier, expected.has_supplier,
            "{}",
            fixture.name
        );
        assert_eq!(
            actual.type_literals.len(),
            expected.type_literal_count,
            "{} {}",
            fixture.name,
            expected.code_name
        );
    }
}

#[test]
fn real_ssg_snapshots_preserve_type_metadata_and_parse_standard_literals() {
    for (revision, fixture) in FIXTURES.iter().enumerate() {
        let revision = revision as u64 + 1;
        let snapshot = load_snapshot(fixture);
        let catalog = Arc::new(snapshot.catalog().clone());
        assert_type_metadata(&catalog, fixture);

        let mut host = ParserHost::new(
            CORE_LIBRARY,
            HostConfig {
                syntax_catalog: Some(catalog.clone()),
                ..HostConfig::default()
            },
        )
        .expect("CoreLibrary must initialize with the real SSG catalog");

        let timespan = catalog
            .type_by_code_name("timespan")
            .expect("real snapshot must contain Timespan");
        let timespan_node = selected_node(
            parse_typed(&mut host, "1 second", TIMESPAN_CLASS, revision),
            fixture.name,
        );
        assert!(matches!(
            &timespan_node.kind,
            ExpressionNodeKind::Literal { parser_id } if parser_id == "core.literal.timespan"
        ));
        assert_eq!(
            timespan_node.return_type.as_ref().map(ClassName::as_str),
            Some(TIMESPAN_CLASS)
        );
        assert_eq!(timespan_node.multiplicity, Some(Multiplicity::Single));
        assert_eq!(
            metadata(&timespan_node, "type-parser-code-name"),
            Some("timespan")
        );
        assert_eq!(
            metadata(&timespan_node, "type-parser-definition-id"),
            Some(timespan.definition_id.as_str())
        );
        assert_eq!(
            metadata(&timespan_node, "type-parser-registration-id"),
            Some(timespan.registration_id.as_str())
        );
        assert_eq!(
            metadata(&timespan_node, "timespan-milliseconds"),
            Some("1000")
        );

        let entity_type = catalog
            .type_by_code_name("entitytype")
            .expect("real snapshot must contain EntityType");
        let entity_node = selected_node(
            parse_typed(&mut host, "3 zombies", ENTITY_TYPE_CLASS, revision + 10),
            "EntityType",
        );
        assert!(matches!(
            &entity_node.kind,
            ExpressionNodeKind::Literal { parser_id } if parser_id == "core.literal.entity-type"
        ));
        assert_eq!(
            entity_node.return_type.as_ref().map(ClassName::as_str),
            Some(ENTITY_TYPE_CLASS)
        );
        assert_eq!(entity_node.multiplicity, Some(Multiplicity::Single));
        assert_eq!(
            metadata(&entity_node, "type-parser-definition-id"),
            Some(entity_type.definition_id.as_str())
        );
        assert_eq!(metadata(&entity_node, "entity-type-amount"), Some("3"));
        assert_eq!(metadata(&entity_node, "entity-type-raw-amount"), Some("3"));

        let entity_data = json_metadata(&entity_node, "entity-data");
        let entity_data = entity_data
            .as_object()
            .expect("EntityType metadata must contain an entity-data object");
        assert_eq!(
            entity_data
                .get("entity-plural")
                .and_then(serde_json::Value::as_str),
            Some("true")
        );
        if fixture.modern_supplier_literals {
            assert_eq!(
                entity_data
                    .get("literal-canonical")
                    .and_then(serde_json::Value::as_str),
                Some("zombie")
            );
            assert_eq!(
                entity_data
                    .get("literal-source")
                    .and_then(serde_json::Value::as_str),
                Some("supplier")
            );
            assert_eq!(
                entity_data
                    .get("type-addon-version")
                    .and_then(serde_json::Value::as_str),
                Some(fixture.skript_version)
            );
            assert_eq!(
                entity_data
                    .get("type-parser-class")
                    .and_then(serde_json::Value::as_str),
                Some(
                    catalog
                        .type_by_code_name("entitydata")
                        .expect("real snapshot must contain EntityData")
                        .parser_class
                        .as_ref()
                        .expect("supplier-backed EntityData must retain parserClass")
                        .as_str()
                )
            );
            assert_eq!(
                entity_data
                    .get("literal-range-start")
                    .and_then(serde_json::Value::as_str),
                Some("2")
            );
            assert_eq!(
                entity_data
                    .get("literal-range-end")
                    .and_then(serde_json::Value::as_str),
                Some("9")
            );
        } else {
            assert_eq!(
                entity_data
                    .get("entity-class")
                    .and_then(serde_json::Value::as_str),
                Some("org.bukkit.entity.Zombie")
            );
            assert_eq!(
                entity_data
                    .get("entity-source")
                    .and_then(serde_json::Value::as_str),
                Some("core.legacy-compatibility")
            );
            assert!(!entity_data.contains_key("literal-source"));
        }
    }
}

#[test]
fn real_modern_ssg_snapshots_parse_finite_enchantment_types() {
    for (revision, fixture) in FIXTURES.iter().skip(1).enumerate() {
        let snapshot = load_snapshot(fixture);
        let catalog = Arc::new(snapshot.catalog().clone());
        let enchantment = catalog
            .type_by_code_name("enchantment")
            .expect("modern snapshot must contain the Enchantment type");
        assert_eq!(enchantment.addon.version, fixture.skript_version);
        assert!(enchantment.has_parser);
        assert!(enchantment.has_supplier);
        assert_eq!(enchantment.type_literals.len(), 43);
        assert_eq!(
            enchantment.parser_class.as_ref().map(ClassName::as_str),
            Some("ch.njol.skript.classes.registry.RegistryParser")
        );
        assert_eq!(
            enchantment.type_parse_order,
            if fixture.skript_version == "2.15.4" {
                28
            } else {
                68
            }
        );
        let literal = enchantment
            .type_literals
            .iter()
            .find(|literal| literal.text == "sharpness")
            .expect("modern snapshot must export sharpness as a finite literal");
        assert_eq!(
            literal.value_class.as_str(),
            "org.bukkit.craftbukkit.enchantments.CraftEnchantment"
        );
        assert!(literal.represented_class.is_none());

        let mut host = ParserHost::new(
            CORE_LIBRARY,
            HostConfig {
                syntax_catalog: Some(catalog),
                ..HostConfig::default()
            },
        )
        .expect("CoreLibrary must initialize with the modern SSG catalog");
        let node = selected_node(
            parse_typed(
                &mut host,
                "sharpness 5",
                ENCHANTMENT_TYPE_CLASS,
                revision as u64 + 100,
            ),
            fixture.name,
        );
        assert!(matches!(
            &node.kind,
            ExpressionNodeKind::Literal { parser_id } if parser_id == "core.literal.enchantment-type"
        ));
        assert_eq!(metadata(&node, "enchantment"), Some("sharpness"));
        assert_eq!(metadata(&node, "enchantment-level"), Some("5"));
    }
}

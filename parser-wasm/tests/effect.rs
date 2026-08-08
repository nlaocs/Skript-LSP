use parser_wasm::host::{HostConfig, InvocationContext, ParserHost};
use skript_parser::{
    EffectParseRequest, EffectParserConfig, ExpressionExpectedType, ExpressionParseContext,
    ExpressionParseRequest, ExpressionParserConfig, MappedSource, RawTreeOptions, TextRange,
    parse_raw_tree,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use syntaxes::{
    Catalog, CatalogParts, ClassName, PossibleReturnTypesState, ReturnTypeState, Syntax,
};

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));
const EFFECT_ADDON: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/effect-addon.wasm"
));
const DYNAMIC_SYNTAX_ADDON: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/dynamic-syntax-addon.wasm"
));

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn effect_catalog() -> Arc<Catalog> {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let source = snapshot.catalog();
    let syntaxes = source
        .syntaxes()
        .iter()
        .filter(|syntax| match syntax {
            Syntax::Type(value) => matches!(value.code_name.as_str(), "string" | "object"),
            Syntax::Effect(value) => {
                value.common.definition_id.as_str()
                    == "effect:skript:751b28432979bd1f00e370ffe6f6c3279e4936b90071eda5ed732d7cda2c0504"
                    || value.common.patterns.iter().any(|pattern| {
                        matches!(
                            pattern.source.as_str(),
                            "dummy effect registered through wrapper"
                                | "run dummy fixture effect [with %-string%]"
                        )
                    })
            }
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

fn full_dynamic_catalog() -> Arc<Catalog> {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let source = snapshot.catalog();
    let mut syntaxes = source.syntaxes().to_vec();
    for syntax in &mut syntaxes {
        let Syntax::Expression(expression) = syntax else {
            continue;
        };
        if expression
            .common
            .element_class
            .as_str()
            .ends_with(".PropExprSize")
        {
            expression.return_type_state = ReturnTypeState::Dynamic;
            expression.possible_return_types = vec![ClassName("java.lang.Long".to_owned())];
            expression.possible_return_types_state = PossibleReturnTypesState::Partial;
        } else if expression
            .common
            .element_class
            .as_str()
            .ends_with(".ExprParse")
        {
            expression.return_type_state = ReturnTypeState::Dynamic;
            expression.possible_return_types.clear();
            expression.possible_return_types_state = PossibleReturnTypesState::Unresolved;
        }
    }
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
    }))
}

fn context(revision: u64) -> InvocationContext {
    InvocationContext {
        invocation_id: revision,
        subscription_id: String::new(),
        document_id: "file:///workspace/effect.sk".to_owned(),
        document_revision: revision,
        expansion: None,
        syntax_context: 0,
    }
}

fn parse_effect(
    host: &mut ParserHost,
    transaction: &parser_wasm::state::ParseTransaction,
    revision: u64,
    text: &str,
) -> parser_wasm::host::WasmEffectParseResult {
    let source = MappedSource::identity(text);
    let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
    let node = tree.get(tree.roots[0]).expect("one Simple node");
    host.parse_effect_in_parse(
        transaction,
        context(revision),
        EffectParseRequest {
            source: &source,
            node,
            context: ExpressionParseContext::default(),
        },
        EffectParserConfig::default(),
    )
    .expect("Effect pipeline must remain recoverable")
}

#[test]
fn wasm_effect_hook_replaces_metadata_and_keeps_selected_state() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(effect_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    host.load_addon(EFFECT_ADDON)
        .expect("Effect addon must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 1)
        .unwrap();

    let result = parse_effect(
        &mut host,
        &transaction,
        1,
        "dummy effect registered through wrapper",
    );
    let selected = result.matches.selected.expect("Effect must be selected");
    assert_eq!(
        selected.metadata.get("wasm").map(String::as_str),
        Some("replaced")
    );
    assert!(
        result
            .calls
            .iter()
            .any(|call| call.subscription_id == "effect.replace")
    );
    let writes = transaction.read_write_set().unwrap().writes;
    assert!(writes.iter().any(|write| write.key == "category-before"));
    assert!(writes.iter().any(|write| write.key == "replace"));
    assert!(writes.iter().all(|write| write.key != "reject"));
    transaction.cancel().unwrap();
}

#[test]
fn wasm_effect_reject_restores_nested_expression_and_hook_state() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(effect_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    host.load_addon(EFFECT_ADDON)
        .expect("Effect addon must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 2)
        .unwrap();

    let result = parse_effect(
        &mut host,
        &transaction,
        2,
        "run dummy fixture effect with \"metadata\"",
    );
    let unknown = result
        .matches
        .unknown
        .expect("rejected Effect becomes unknown");
    assert!(unknown.failure.is_some());
    assert!(
        result
            .effects
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "effect-fixture-reject")
    );
    assert_eq!(transaction.state_revision().unwrap(), 0);
    assert!(transaction.read_write_set().unwrap().writes.is_empty());
    transaction.cancel().unwrap();
}

#[test]
fn dynamic_effect_uses_the_same_end_to_end_pipeline() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(effect_catalog()),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    host.load_addon(DYNAMIC_SYNTAX_ADDON)
        .expect("dynamic syntax addon must load");
    host.load_addon(EFFECT_ADDON)
        .expect("Effect addon must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 3)
        .unwrap();

    let result = parse_effect(&mut host, &transaction, 3, "dummy initialize \"value\"");
    let selected = result
        .matches
        .selected
        .expect("dynamic Effect must be selected");
    assert_eq!(
        selected.matched.registration_id,
        "dynamic:nlaocs.test.dynamic-syntax/initial-effect"
    );
    assert_eq!(selected.handler.as_deref(), Some("dynamic.initial-effect"));
    assert_eq!(selected.expressions.len(), 1);
    let writes = transaction.read_write_set().unwrap().writes;
    assert!(writes.iter().any(|write| write.key == "category-before"));
    assert!(writes.iter().any(|write| write.key == "category-after"));
    transaction.cancel().unwrap();
}

#[test]
fn dynamic_size_and_parse_expressions_work_inside_effects() {
    let catalog = full_dynamic_catalog();
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 4)
        .unwrap();

    let size_text = "all offline players's size";
    let size_source = MappedSource::identity(size_text);
    let size = host
        .parse_expression_in_parse(
            &transaction,
            context(4),
            ExpressionParseRequest {
                source: &size_source,
                range: TextRange::new(0, size_text.len()),
                expected_types: vec![ExpressionExpectedType {
                    class_name: ClassName("java.lang.Number".to_owned()),
                    plural: false,
                }],
                context: ExpressionParseContext::default(),
            },
            ExpressionParserConfig::default(),
        )
        .unwrap();
    assert!(
        size.matches.selected.is_some(),
        "size must resolve as Number"
    );

    let vector = parse_effect(
        &mut host,
        &transaction,
        4,
        "send new vector from yaw all offline players's size and pitch all offline players's size",
    );
    assert!(
        vector.matches.selected.is_some(),
        "vector send must parse: {:#?}",
        vector.matches.unknown
    );
    transaction.cancel().unwrap();

    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 5)
        .unwrap();
    let parsed = parse_effect(
        &mut host,
        &transaction,
        5,
        "set {_parsed} to \"42\" parsed as number",
    );
    assert!(
        parsed.matches.selected.is_some(),
        "typed ExprParse must parse: {:#?}",
        parsed.matches.unknown
    );
    transaction.cancel().unwrap();

    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/effect.sk", 6)
        .unwrap();
    let parsed = parse_effect(
        &mut host,
        &transaction,
        6,
        "set {_parsed::*} to \"value: 42\" parsed as \"value: %number%\"",
    );
    assert!(
        parsed.matches.selected.is_some(),
        "pattern ExprParse must parse: {:#?}",
        parsed.matches.unknown
    );
    transaction.cancel().unwrap();
}

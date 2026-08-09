use parser_wasm::host::{HostConfig, HostError, InvocationContext, ParserHost};
use skript_parser::{
    ExpressionParseContext, MappedSource, RawTreeOptions, SectionBodyNode, SectionDiagnosticKind,
    SectionParseRequest, SectionParserConfig, parse_raw_tree,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use syntaxes::{Catalog, CatalogParts, DefinitionId, RegistrationId, Syntax};

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn catalog() -> Arc<Catalog> {
    Arc::new(ssg::load(fixture()).unwrap().catalog().clone())
}

fn duplicate_effect_catalog() -> Arc<Catalog> {
    let snapshot = ssg::load(fixture()).unwrap();
    let source = snapshot.catalog();
    let mut syntaxes = source.syntaxes().to_vec();
    let mut duplicate = source
        .effects()
        .find(|effect| {
            effect
                .common
                .patterns
                .iter()
                .any(|pattern| pattern.source == "dummy effect registered through wrapper")
        })
        .expect("fixture Effect")
        .clone();
    duplicate.common.definition_id = DefinitionId("effect:test:duplicate".to_owned());
    duplicate.common.registration_id = RegistrationId("effect:test:duplicate:0".to_owned());
    duplicate.common.registration_order = usize::MAX - 1;
    syntaxes.push(Syntax::Effect(duplicate));
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
        document_id: "file:///workspace/section.sk".to_owned(),
        document_revision: revision,
        expansion: None,
        syntax_context: 0,
    }
}

fn parse(
    host: &mut ParserHost,
    transaction: &parser_wasm::state::ParseTransaction,
    revision: u64,
    input: &str,
    config: SectionParserConfig,
) -> Result<parser_wasm::WasmSectionParseResult, HostError> {
    let source = MappedSource::identity(input);
    let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
    let node = tree.get(tree.roots[0]).expect("one root Section");
    host.parse_section_in_parse(
        transaction,
        context(revision),
        SectionParseRequest {
            source: &source,
            tree: &tree,
            node,
            context: ExpressionParseContext::default(),
        },
        config,
    )
}

#[test]
fn conditional_and_loop_sections_recursively_claim_their_bodies() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .unwrap();
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/section.sk", 1)
        .unwrap();
    let input = "if dummy fixture condition:\n    dummy effect registered through wrapper\n    while dummy fixture condition:\n        dummy effect registered through wrapper\n";
    let result = parse(
        &mut host,
        &transaction,
        1,
        input,
        SectionParserConfig::default(),
    )
    .expect("nested Sections must parse");
    let outer = result.matches.selected.expect("conditional Section");

    assert_eq!(outer.conditions.len(), 1);
    assert_eq!(outer.body.len(), 2);
    let SectionBodyNode::Section(nested) = &outer.body[1] else {
        panic!("second child must be a nested Section");
    };
    let nested = nested.selected.as_ref().expect("while Section");
    assert!(nested.loop_section);
    assert_eq!(nested.conditions.len(), 1);
    assert!(result.matches.diagnostics.is_empty());
    assert!(result.effects.context_updates.iter().any(|update| {
        update.key == "core.section.loop" && update.value.as_deref() == Some(b"true")
    }));
    assert!(
        result
            .calls
            .iter()
            .filter(|call| {
                call.component_id == "nlaocs.core-library"
                    && call.subscription_id == "core.section-semantics"
            })
            .count()
            >= 4
    );
    transaction.cancel().unwrap();
}

#[test]
fn unknown_body_lines_are_retained_as_partial_ast_diagnostics() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .unwrap();
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/section.sk", 2)
        .unwrap();
    let result = parse(
        &mut host,
        &transaction,
        2,
        "if dummy fixture condition:\n    this is not a registered effect\n",
        SectionParserConfig::default(),
    )
    .expect("an incomplete body remains recoverable");

    assert!(result.matches.selected.is_some());
    assert!(
        result
            .matches
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == SectionDiagnosticKind::Unclaimed)
    );
    transaction.cancel().unwrap();
}

#[test]
fn multiple_body_claims_are_reported_without_discarding_the_selected_effect() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(duplicate_effect_catalog()),
            ..HostConfig::default()
        },
    )
    .unwrap();
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/section.sk", 6)
        .unwrap();
    let result = parse(
        &mut host,
        &transaction,
        6,
        "dummy fixture section:\n    dummy effect registered through wrapper\n",
        SectionParserConfig::default(),
    )
    .expect("ambiguous body claims remain recoverable");

    assert!(result.matches.selected.is_some());
    assert!(
        result
            .matches
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == SectionDiagnosticKind::MultipleClaims)
    );
    transaction.cancel().unwrap();
}

#[test]
fn effect_and_expression_section_metadata_is_preserved() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .unwrap();
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/section.sk", 4)
        .unwrap();
    let expression = parse(
        &mut host,
        &transaction,
        4,
        "a virtual world border:\n    dummy effect registered through wrapper\n",
        SectionParserConfig::default(),
    )
    .expect("SectionExpression must use the Section pipeline")
    .matches
    .selected
    .expect("world border SectionExpression");
    assert!(expression.section_expression);
    transaction.cancel().unwrap();

    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/section.sk", 5)
        .unwrap();
    let effect = parse(
        &mut host,
        &transaction,
        5,
        "shoot players:\n    dummy effect registered through wrapper\n",
        SectionParserConfig::default(),
    )
    .expect("EffectSection must use the Section pipeline")
    .matches
    .selected
    .expect("shoot EffectSection");
    assert!(effect.effect_section);
    transaction.cancel().unwrap();
}

#[test]
fn section_recursion_uses_the_shared_expression_depth_limit() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .unwrap();
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/section.sk", 3)
        .unwrap();
    let mut config = SectionParserConfig::default();
    config.expression.max_depth = 2;
    let error = parse(
        &mut host,
        &transaction,
        3,
        "if dummy fixture condition:\n    if dummy fixture condition:\n        if dummy fixture condition:\n            dummy effect registered through wrapper\n",
        config,
    )
    .expect_err("deep Sections must respect the configured recursion limit");

    assert!(matches!(&error, HostError::SectionParser(_)));
    assert!(error.to_string().contains("recursion depth limit of 2"));
    transaction.cancel().unwrap();
}

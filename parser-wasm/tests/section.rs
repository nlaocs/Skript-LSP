use parser_wasm::host::{HostConfig, HostError, InvocationContext, ParserHost, RuntimeProfile};
use skript_parser::{
    ExpressionParseContext, MappedSource, RawTreeOptions, SectionBodyNode, SectionDiagnosticKind,
    SectionParseRequest, SectionParserConfig, parse_raw_tree,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use syntax_pattern_parser::syntax;
use syntaxes::{Catalog, CatalogParts, ClassName, DefinitionId, Pattern, RegistrationId, Syntax};

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));
const EFFECT_ADDON: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/effect-addon.wasm"
));

fn fixture() -> PathBuf {
    // EffectSections with EntityData captures require the schema 5 runtime parser patterns.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/type-parser-versions/skript-2.15.4")
}

fn catalog() -> Arc<Catalog> {
    Arc::new(ssg::load(fixture()).unwrap().catalog().clone())
}

fn duplicate_effect_catalog() -> Arc<Catalog> {
    let snapshot = ssg::load(fixture()).unwrap();
    let source = snapshot.catalog();
    let source_view = source.source().cloned().expect("SSG source view");
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

fn enter_rejection_fallback_catalog() -> Arc<Catalog> {
    let snapshot = ssg::load(fixture()).unwrap();
    let source = snapshot.catalog();
    let source_view = source.source().cloned().expect("SSG source view");
    let mut syntaxes = source.syntaxes().to_vec();
    let mut fallback = source
        .sections()
        .find(|section| {
            section.common.element_class.as_str()
                == "jp.nlaocs.skriptDummyAddon.fixture.LegacySyntaxes$DummySection"
        })
        .expect("fixture Section")
        .clone();
    let pattern_source = "filter %~objects% to match";
    fallback.common.definition_id = DefinitionId("section:test:filter-fallback".to_owned());
    fallback.common.registration_id = RegistrationId("section:test:filter-fallback:0".to_owned());
    fallback.common.registration_order = usize::MAX;
    fallback.common.element_class = ClassName("test.GenericFilterSection".to_owned());
    fallback.common.patterns = vec![Pattern {
        source: pattern_source.to_owned(),
        parsed: syntax::parse(pattern_source, source.plural_rules())
            .expect("fallback Section pattern must parse"),
    }];
    syntaxes.push(Syntax::Section(fallback));
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

    assert_eq!(outer.conditions().count(), 1);
    assert_eq!(outer.body.len(), 2);
    let SectionBodyNode::Section(nested) = &outer.body[1] else {
        panic!("second child must be a nested Section");
    };
    let nested = nested.selected.as_ref().expect("while Section");
    assert!(nested.loop_section);
    assert_eq!(nested.conditions().count(), 1);
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
fn loop_control_effects_follow_section_scope() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .unwrap();
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/section.sk", 7)
        .unwrap();
    let result = parse(
        &mut host,
        &transaction,
        7,
        "while dummy fixture condition:\n    continue this loop\n    exit this loop\n",
        SectionParserConfig::default(),
    )
    .expect("loop control Effects must parse inside a loop Section");
    let outer = result.matches.selected.expect("while Section");
    assert_eq!(outer.body.len(), 2);

    let SectionBodyNode::Effect(continue_effect) = &outer.body[0] else {
        panic!("first loop body node must be continue: {:#?}", outer.body);
    };
    let continue_effect = continue_effect
        .selected
        .as_ref()
        .expect("continue must be accepted inside a loop");
    assert!(continue_effect.matched.pattern.starts_with("continue"));
    assert_eq!(
        continue_effect
            .metadata
            .get("nlaocs.core-library/semantic-mode")
            .map(String::as_str),
        Some("continue-loop"),
        "{continue_effect:#?}"
    );
    assert_eq!(
        continue_effect
            .metadata
            .get("nlaocs.core-library/available-loop-depth")
            .map(String::as_str),
        Some("1")
    );

    let SectionBodyNode::Effect(exit_effect) = &outer.body[1] else {
        panic!("second loop body node must be exit: {:#?}", outer.body);
    };
    let exit_effect = exit_effect
        .selected
        .as_ref()
        .expect("exit this loop must be accepted inside a loop");
    assert!(exit_effect.matched.pattern.starts_with("(exit|stop)"));
    assert_eq!(
        exit_effect
            .metadata
            .get("nlaocs.core-library/exit-target")
            .map(String::as_str),
        Some("loops")
    );
    assert_eq!(
        exit_effect
            .metadata
            .get("nlaocs.core-library/exit-count")
            .map(String::as_str),
        Some("1")
    );
    transaction.cancel().unwrap();

    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/section.sk", 8)
        .unwrap();
    // The Effects are still recognized under `if`, but this scope is not a
    // loop, so Skript rejects both loop-control Effects here.
    let result = parse(
        &mut host,
        &transaction,
        8,
        "if dummy fixture condition:\n    continue this loop\n    exit this loop\n",
        SectionParserConfig::default(),
    )
    .expect("a conditional body remains recoverable when loop controls are rejected");
    let outer = result.matches.selected.expect("conditional Section");
    assert_eq!(outer.body.len(), 2);
    for (index, body) in outer.body.iter().enumerate() {
        let SectionBodyNode::Effect(effect) = body else {
            panic!("conditional body node {index} must be an Effect: {body:#?}");
        };
        assert!(
            effect.selected.is_none(),
            "loop control Effect {index} must be rejected outside a loop: {effect:#?}"
        );
        assert!(effect.unknown.is_some());
    }
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
fn duplicate_body_candidates_select_the_first_and_retain_the_alternative() {
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
    let selected = result.matches.selected.as_ref().unwrap();
    let SectionBodyNode::Effect(effect) = &selected.body[0] else {
        panic!("the body line must remain an Effect");
    };
    assert!(effect.selected.is_some());
    assert_eq!(effect.alternatives.len(), 1);
    assert_eq!(
        effect.alternatives[0].matched.definition_id,
        "effect:test:duplicate"
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

#[test]
fn section_enter_rejection_retries_a_later_header_candidate() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(enter_rejection_fallback_catalog()),
            runtime_profile: RuntimeProfile {
                // SecFilter was introduced in 2.10, so the supported 2.6.4
                // profile exercises fallback after a native rejection.
                skript_version: Some("2.6.4".to_owned()),
                ..RuntimeProfile::default()
            },
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/section.sk", 10)
        .unwrap();
    let result = parse(
        &mut host,
        &transaction,
        10,
        "filter {_list::*} to match:\n    dummy effect registered through wrapper\n",
        SectionParserConfig::default(),
    )
    .expect("a rejected Section candidate must remain recoverable");

    let selected = result
        .matches
        .selected
        .as_ref()
        .expect("the fallback Section must be selected after enter rejection");
    assert_eq!(
        selected.element_class.as_ref().map(ClassName::as_str),
        Some("test.GenericFilterSection")
    );
    assert!(result.matches.unknown.is_none());
    assert!(result.calls.iter().any(|call| {
        call.component_id == "nlaocs.core-library"
            && call.subscription_id == "core.section-semantics"
    }));
    transaction.cancel().unwrap();
}

#[test]
fn rejected_section_body_does_not_leak_addon_state_or_context() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .unwrap();
    host.load_addon(EFFECT_ADDON)
        .expect("Effect addon must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/section.sk", 11)
        .unwrap();
    let result = parse(
        &mut host,
        &transaction,
        11,
        "dummy fixture section:\n    run dummy fixture effect with \"metadata\"\n",
        SectionParserConfig::default(),
    )
    .expect("a rejected body Effect must remain recoverable");

    let selected = result
        .matches
        .selected
        .as_ref()
        .expect("the enclosing Section remains selected");
    let SectionBodyNode::Effect(effect) = &selected.body[0] else {
        panic!("the rejected body must remain an Effect node: {selected:#?}");
    };
    assert!(effect.selected.is_none());
    assert!(effect.unknown.is_some());

    let writes = transaction.read_write_set().unwrap().writes;
    assert!(
        writes.iter().all(|write| {
            !matches!(
                write.key.as_str(),
                "category-before" | "category-after" | "not-applicable" | "replace" | "reject"
            )
        }),
        "rejected body StateStore writes leaked: {writes:#?}"
    );
    assert!(
        result
            .effects
            .context_updates
            .iter()
            .all(|update| { update.key != "reject-effects-must-be-rolled-back" })
    );
    assert!(
        result.calls.iter().all(|call| {
            !matches!(
                call.subscription_id.as_str(),
                "effect.category" | "effect.not-applicable" | "effect.replace" | "effect.reject"
            )
        }),
        "rejected body hook calls leaked: {:#?}",
        result.calls
    );
    transaction.cancel().unwrap();
}

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, LazyLock},
};

use parser_wasm::{
    ParseTransaction,
    host::{HostConfig, InvocationContext, ParserHost, RuntimeProfile, WasmExpressionParseResult},
    state::{NamespaceVisibility, StateScope},
};
use skript_parser::{
    DefaultExpressionFailureKind, EffectParseRequest, EffectParserConfig, ExpressionExpectedType,
    ExpressionNode, ExpressionNodeKind, ExpressionParseContext, ExpressionParseRequest,
    ExpressionParserConfig, FailureTrace, MappedSource, OriginKind, PatternCapture,
    PatternFailureReason, RawTreeOptions, TextRange, TypeCaptureState, parse_raw_tree,
};
use syntax_pattern_parser::syntax;
use syntaxes::{
    Catalog, CatalogParts, Class, ClassKind, ClassName, CommonSyntax, DefinitionId, Multiplicity,
    Pattern, PossibleReturnTypesState, RegistrationId, ResolutionState, ReturnTypeState, Syntax,
    TypeCodeName,
};

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));
const ADDON_A: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/expression-data-addon-a.wasm"
));
const ADDON_B: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/expression-data-addon-b.wasm"
));
const MATCHING_ADDON: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/matching-addon.wasm"
));
const COMPONENT_A: &str = "test.expression-data-a";
const COMPONENT_B: &str = "test.expression-data-b";
const DOCUMENT: &str = "file:///workspace/default-expression.sk";
const EXPRESSION_ID: &str = "expression:test.default-expression:0";

static CATALOG: LazyLock<Arc<Catalog>> = LazyLock::new(|| {
    let snapshot = ssg::load(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/type-parser-versions/skript-2.16.0"),
    )
    .expect("Skript 2.16.0 snapshot must load");
    let source = snapshot.catalog();
    let mut syntaxes = source
        .syntaxes()
        .iter()
        .filter(|syntax| matches!(syntax, Syntax::Type(_)))
        .cloned()
        .collect::<Vec<_>>();
    let mut expression = source
        .expressions()
        .next()
        .expect("snapshot Expression")
        .clone();
    configure_common(
        &mut expression.common,
        EXPRESSION_ID,
        &[
            "fixture default [value %number%]",
            "fixture nullable [value %-number%]",
            "fixture literal [value %*number%]",
            "fixture expression [value %~number%]",
            "fixture timed [value %number@-1%]",
            "fixture pair [first %number%] [second %number%]",
            "fixture rollback %number% [default %number%]",
            "fixture rollback 42",
            "fixture custom rollback %number% [default %fixturevalue%]",
            "fixture custom rollback 42",
        ],
        source,
    );
    expression.return_type = Some(ClassName("java.lang.Object".to_owned()));
    expression.return_type_state = ReturnTypeState::Static;
    expression.possible_return_types = vec![ClassName("java.lang.Object".to_owned())];
    expression.possible_return_types_state = PossibleReturnTypesState::Complete;
    expression.return_type_multiplicity = Some(Multiplicity::Single);
    expression.return_type_multiplicity_state = ResolutionState::Resolved;
    expression.section_expression = false;
    syntaxes.push(Syntax::Expression(expression.clone()));
    configure_common(
        &mut expression.common,
        "expression:test.default-expression:custom:0",
        &["fixture custom [value %fixturevalue%]"],
        source,
    );
    syntaxes.push(Syntax::Expression(expression));

    let mut custom = source
        .type_by_code_name("number")
        .expect("snapshot number Type")
        .clone();
    custom.definition_id = DefinitionId("type:test.default-expression".to_owned());
    custom.registration_id = RegistrationId("type:test.default-expression:0".to_owned());
    custom.source_index = usize::MAX;
    custom.original_class = ClassName("fixture.DefaultValue".to_owned());
    custom.code_name = TypeCodeName("fixturevalue".to_owned());
    custom.addon.name = "FixtureAddon".to_owned();
    custom.default_expression = Some(syntaxes::DefaultExpressionDescriptor {
        implementation_class: ClassName("fixture.DefaultExpression".to_owned()),
        literal: Some(false),
        return_type: Some(ClassName("java.lang.Number".to_owned())),
        single: Some(true),
    });
    custom.parser_class = None;
    custom.has_parser = false;
    custom.has_supplier = false;
    custom.literal_values.clear();
    custom.type_literals.clear();
    custom.parser_patterns.clear();
    custom.user_input_patterns = vec!["fixturevalues?".to_owned()];
    syntaxes.push(Syntax::Type(custom));
    let mut classes = source.classes().to_vec();
    classes.push(Class {
        name: ClassName("fixture.DefaultValue".to_owned()),
        binary_name: "fixture.DefaultValue".to_owned(),
        kind: ClassKind::Class,
        super_class: Some(ClassName("java.lang.Object".to_owned())),
        interfaces: Vec::new(),
        component_type: None,
        container_element_type: None,
        methods: None,
        provider: None,
    });
    let mut effect = source.effects().next().expect("snapshot Effect").clone();
    configure_common(
        &mut effect.common,
        "effect:test.default-expression:0",
        &["fixture effect [value %number%]"],
        source,
    );
    syntaxes.push(Syntax::Effect(effect));
    Arc::new(
        Catalog::new(CatalogParts {
            syntaxes,
            classes,
            converters: source.converters().to_vec(),
            comparators: source.comparators().to_vec(),
            event_values: Vec::new(),
            properties: Vec::new(),
            operators: source.operators().to_vec(),
            operations: source.operations().clone(),
            differences: Vec::new(),
            aliases: source.aliases().clone(),
            plural_rules: source.plural_rules().clone(),
            language: source
                .language_entries()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect(),
        })
        .with_unchecked_source(source.source().cloned().expect("snapshot source identity")),
    )
});

fn configure_common(common: &mut CommonSyntax, id: &str, patterns: &[&str], catalog: &Catalog) {
    common.definition_id = DefinitionId(id.strip_suffix(":0").unwrap_or(id).to_owned());
    common.registration_id = RegistrationId(id.to_owned());
    common.element_class = ClassName("fixture.DefaultConsumer".to_owned());
    common.super_class = None;
    common.addon.name = "FixtureAddon".to_owned();
    common.priority = None;
    common.priority_name = None;
    common.events.clear();
    common.supported_events = None;
    common.supported_events_state = None;
    common.experimental_syntax = None;
    common.experimental_syntax_state = None;
    common.related_property = None;
    common.patterns = patterns
        .iter()
        .map(|pattern| Pattern {
            source: (*pattern).to_owned(),
            parsed: syntax::parse(pattern, catalog.plural_rules()).expect("fixture pattern"),
        })
        .collect();
}

fn host(addons: &[&[u8]]) -> ParserHost {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(Arc::clone(&CATALOG)),
            runtime_profile: RuntimeProfile {
                skript_version: Some("2.16.0".to_owned()),
                ..RuntimeProfile::default()
            },
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary host");
    for addon in addons {
        host.load_addon(addon).expect("third-party addon");
    }
    host
}

fn begin(host: &ParserHost, revision: u64) -> ParseTransaction {
    host.begin_parse("file:///workspace", DOCUMENT, revision)
        .expect("parse transaction")
}

fn context(revision: u64) -> InvocationContext {
    InvocationContext {
        invocation_id: revision,
        subscription_id: String::new(),
        document_id: DOCUMENT.to_owned(),
        document_revision: revision,
        expansion: None,
        syntax_context: 0,
    }
}

fn parse(
    host: &mut ParserHost,
    transaction: &ParseTransaction,
    text: &str,
    mode: &str,
    revision: u64,
) -> WasmExpressionParseResult {
    let source = MappedSource::identity(text);
    host.parse_expression_in_parse(
        transaction,
        context(revision),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: vec![ExpressionExpectedType {
                class_name: ClassName("java.lang.Object".to_owned()),
                plural: false,
            }],
            context: ExpressionParseContext {
                values: BTreeMap::from([("fixture.default-mode".to_owned(), mode.to_owned())]),
                ..ExpressionParseContext::default()
            },
        },
        ExpressionParserConfig::default(),
    )
    .expect("default parse must be recoverable")
}

fn selected(result: &WasmExpressionParseResult) -> &ExpressionNode {
    &result
        .matches
        .selected
        .as_ref()
        .unwrap_or_else(|| panic!("expected a matched fixture: {result:#?}"))
        .node
}

fn child(result: &WasmExpressionParseResult) -> &ExpressionNode {
    selected(result)
        .children
        .first()
        .expect("one default child")
}

fn default_calls(result: &WasmExpressionParseResult) -> Vec<&str> {
    result
        .calls
        .iter()
        .filter(|call| {
            call.subscription_id.starts_with("default-test.")
                && !call.subscription_id.contains(".consume.")
                && !call.subscription_id.contains(".observe.")
        })
        .map(|call| call.subscription_id.as_str())
        .collect()
}

fn has_capture_failure(
    trace: &FailureTrace,
    capture_index: usize,
    kind: DefaultExpressionFailureKind,
) -> bool {
    trace.failure.reasons.iter().any(|reason| {
        matches!(reason, PatternFailureReason::DefaultExpression {
        capture_index: actual_index, kind: actual, ..
    } if *actual_index == capture_index && *actual == kind)
    }) || trace
        .cause
        .as_deref()
        .is_some_and(|cause| has_capture_failure(cause, capture_index, kind))
}

fn assert_failure(result: &WasmExpressionParseResult, kind: DefaultExpressionFailureKind) {
    assert!(
        result.matches.selected.is_none(),
        "failed default was accepted: {result:#?}"
    );
    let failure = result
        .matches
        .failure
        .as_ref()
        .expect("recognized omitted capture must retain failure");
    assert!(
        failure
            .trace
            .as_ref()
            .is_some_and(|trace| has_capture_failure(trace, 0, kind)),
        "missing typed default failure: {failure:#?}"
    );
}

fn write_count(transaction: &ParseTransaction) -> usize {
    transaction
        .read_write_set()
        .expect("open transaction access set")
        .writes
        .len()
}

fn assert_no_seed_state(transaction: &ParseTransaction) {
    let mut invocation = transaction.begin_invocation(COMPONENT_A).unwrap();
    assert!(
        invocation
            .get(
                StateScope::Document,
                NamespaceVisibility::Private,
                "default-expression-test",
                "default-test.a.seed",
            )
            .unwrap()
            .is_none(),
        "a discarded provider must not leave a readable state value"
    );
    invocation.rollback();
}

#[test]
fn explicit_child_state_is_discarded_after_default_failure_without_matching_hooks() {
    // Only A is loaded: its manifest has no Matching subscriptions.
    let mut host = host(&[ADDON_A]);
    let control = begin(&host, 1);
    let explicit = parse(
        &mut host,
        &control,
        "fixture custom rollback 42",
        "resolve",
        1,
    );
    assert_eq!(
        child(&explicit)
            .metadata
            .get("test.expression-data-a/explicit-child")
            .map(String::as_str),
        Some("recorded")
    );
    assert_eq!(selected(&explicit).children.len(), 2);
    assert!(matches!(
        selected(&explicit).children[1].kind,
        ExpressionNodeKind::Default { .. }
    ));
    assert_eq!(write_count(&control), 2);
    control.cancel().unwrap();

    for (revision, mode) in [(2, "reject"), (3, "unresolved")] {
        let transaction = begin(&host, revision);
        let result = parse(
            &mut host,
            &transaction,
            "fixture custom rollback 42",
            mode,
            revision,
        );
        let fallback = selected(&result);
        assert!(fallback.children.is_empty());
        assert!(fallback.captures.is_empty());
        assert!(
            !fallback
                .metadata
                .contains_key("test.expression-data-a/explicit-child")
        );
        assert_eq!(write_count(&transaction), 0);
        let mut invocation = transaction.begin_invocation(COMPONENT_A).unwrap();
        for key in ["explicit-child", "default-test.a.seed"] {
            assert!(
                invocation
                    .get(
                        StateScope::Document,
                        NamespaceVisibility::Private,
                        "default-expression-test",
                        key,
                    )
                    .unwrap()
                    .is_none(),
                "rejected child state survived into the fallback pattern: {key}"
            );
        }
        invocation.rollback();
        transaction.cancel().unwrap();
    }
}

#[test]
fn pattern_after_reject_does_not_restore_explicit_or_default_child_state() {
    let mut host = host(&[ADDON_A, ADDON_B]);
    let transaction = begin(&host, 1);
    let result = parse(&mut host, &transaction, "fixture rollback 42", "resolve", 1);
    let fallback = selected(&result);
    assert!(fallback.children.is_empty());
    assert!(fallback.captures.is_empty());
    assert_eq!(
        fallback
            .metadata
            .get("test.expression-data-b/default-child-count")
            .map(String::as_str),
        Some("0")
    );
    assert_eq!(write_count(&transaction), 1);
    for component in [COMPONENT_A, COMPONENT_B] {
        let mut invocation = transaction.begin_invocation(component).unwrap();
        for key in [
            "explicit-child",
            "default-test.a.seed",
            "default-test.b.enrich",
        ] {
            assert!(
                invocation
                    .get(
                        StateScope::Document,
                        NamespaceVisibility::Private,
                        "default-expression-test",
                        key,
                    )
                    .unwrap()
                    .is_none(),
                "Pattern After rejection retained {component}/{key}"
            );
        }
        if component == COMPONENT_B {
            assert!(
                invocation
                    .get(
                        StateScope::Document,
                        NamespaceVisibility::Private,
                        "default-expression-test",
                        "next-pattern-clean",
                    )
                    .unwrap()
                    .is_some(),
                "the next Pattern Before must observe the rollback before it executes"
            );
        }
        invocation.rollback();
    }
    transaction.cancel().unwrap();
}

#[test]
fn default_flows_through_two_addons_and_registered_expression_hook() {
    let mut host = host(&[ADDON_A, ADDON_B]);
    let transaction = begin(&host, 1);
    let result = parse(&mut host, &transaction, "fixture default", "resolve", 1);
    let root = selected(&result);
    let value = child(&result);
    let ExpressionNodeKind::Default { info } = &value.kind else {
        panic!("implicit child missing")
    };
    assert_eq!(info.capture_index, 0);
    assert!(root.captures.iter().any(|capture| matches!(
        capture,
        PatternCapture::TypeExpression {
            state: TypeCaptureState::Default,
            ..
        }
    )));
    assert_eq!(info.component_id, COMPONENT_A);
    assert_eq!(info.provider_id, "default-test.a.seed");
    assert_eq!(info.requested_type.class_name.as_str(), "java.lang.Number");
    assert_eq!(
        value.return_type.as_ref().map(ClassName::as_str),
        Some("java.lang.Number")
    );
    assert_eq!(value.multiplicity, Some(Multiplicity::Single));
    assert_eq!(value.span.local_range, TextRange::new(15, 15));
    assert!(
        value
            .span
            .mapped
            .origins
            .iter()
            .all(|origin| origin.kind == OriginKind::Exact)
    );
    assert_eq!(
        root.metadata
            .get("test.expression-data-b/default-child-count")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        value
            .metadata
            .get("test.expression-data-b/observed-order")
            .map(String::as_str),
        Some("seed>late>same")
    );
    assert_eq!(
        value
            .metadata
            .get("test.expression-data-b/observed-component")
            .map(String::as_str),
        Some(COMPONENT_A)
    );
    assert_eq!(
        value.public_data[0].schema_id,
        "test.default-expression.evidence"
    );
    assert!(value.public_data.iter().any(|data| data.schema_id
        == "test.default-expression.observation"
        && data.json == r#"{"observedImplicit":true}"#));
    assert_eq!(
        default_calls(&result),
        [
            "default-test.a.seed",
            "default-test.a.late",
            "default-test.a.same-priority",
            "default-test.b.enrich"
        ]
    );
    assert_eq!(write_count(&transaction), 4);
    transaction.cancel().unwrap();
}

#[test]
fn priority_precedes_load_order_and_equal_priority_uses_load_order() {
    let mut host = host(&[ADDON_B, ADDON_A]);
    let transaction = begin(&host, 1);
    let result = parse(&mut host, &transaction, "fixture default", "resolve", 1);
    assert_eq!(
        default_calls(&result),
        [
            "default-test.a.seed",
            "default-test.b.enrich",
            "default-test.a.late",
            "default-test.a.same-priority"
        ]
    );
    assert_eq!(
        child(&result)
            .metadata
            .get("test.expression-data-b/observed-order")
            .map(String::as_str),
        Some("seed")
    );
    transaction.cancel().unwrap();
}

#[test]
fn addon_can_resolve_its_own_type_through_an_exact_registration() {
    let mut host = host(&[ADDON_A, ADDON_B]);
    let transaction = begin(&host, 1);
    let result = parse(&mut host, &transaction, "fixture custom", "resolve", 1);
    let value = child(&result);
    let ExpressionNodeKind::Default { info } = &value.kind else {
        panic!("custom default")
    };
    assert_eq!(info.type_registration_id, "type:test.default-expression:0");
    assert_eq!(
        value.return_type.as_ref().map(ClassName::as_str),
        Some("fixture.DefaultValue")
    );
    assert_eq!(default_calls(&result), ["default-test.a.custom"]);
    assert_eq!(
        value
            .metadata
            .get("test.expression-data-a/enabled")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        info.catalog_references[0].registration_id.as_deref(),
        Some("type:test.default-expression:0")
    );
    assert_eq!(
        selected(&result)
            .metadata
            .get("test.expression-data-b/default-child-count")
            .map(String::as_str),
        Some("1")
    );
    transaction.cancel().unwrap();
}

#[test]
fn explicit_and_nullable_captures_never_invoke_default_providers() {
    let mut host = host(&[ADDON_A, ADDON_B]);
    for (revision, source, state, children) in [
        (1, "fixture default value 42", TypeCaptureState::Explicit, 1),
        (2, "fixture nullable", TypeCaptureState::Null, 0),
    ] {
        let transaction = begin(&host, revision);
        let result = parse(&mut host, &transaction, source, "trap", revision);
        let root = selected(&result);
        assert_eq!(root.children.len(), children);
        assert!(root.captures.iter().any(|capture| matches!(capture, PatternCapture::TypeExpression { state: actual, .. } if *actual == state)));
        assert!(default_calls(&result).is_empty());
        assert_eq!(write_count(&transaction), 0);
        assert_eq!(
            root.metadata
                .get("test.expression-data-b/default-child-count")
                .map(String::as_str),
            Some("0")
        );
        transaction.cancel().unwrap();
    }
}

#[test]
fn missing_provider_and_reported_unresolved_are_not_successes() {
    for (addons, mode) in [(Vec::new(), "resolve"), (vec![ADDON_A], "unresolved")] {
        let mut host = host(&addons);
        let transaction = begin(&host, 1);
        let result = parse(&mut host, &transaction, "fixture custom", mode, 1);
        assert_failure(&result, DefaultExpressionFailureKind::Unresolved);
        assert_eq!(write_count(&transaction), 0);
        assert!(
            default_calls(&result).is_empty(),
            "unaccepted default effects must be discarded"
        );
        transaction.cancel().unwrap();
    }
}

#[test]
fn reject_keeps_diagnostics_and_discards_all_prior_provider_state() {
    let mut host = host(&[ADDON_A, ADDON_B]);
    let transaction = begin(&host, 1);
    let result = parse(&mut host, &transaction, "fixture default", "reject", 1);
    assert_failure(&result, DefaultExpressionFailureKind::Rejected);
    assert!(
        result
            .effects
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "fixture.default.context")
    );
    assert_eq!(write_count(&transaction), 0);
    assert_no_seed_state(&transaction);
    assert!(result.matches.alternatives.is_empty());
    // Rollback restores this transaction without poisoning subsequent defaults.
    let recovered = parse(&mut host, &transaction, "fixture default", "resolve", 1);
    assert_eq!(
        child(&recovered)
            .metadata
            .get("test.expression-data-a/order")
            .map(String::as_str),
        Some("seed>late>same")
    );
    assert_eq!(write_count(&transaction), 4);
    transaction.cancel().unwrap();
}

#[test]
fn trap_or_invalid_output_discards_an_earlier_success_and_metadata() {
    for mode in [
        "trap",
        "exhaust-fuel",
        "invalid",
        "spoof-owner",
        "both",
        "invalid-time",
        "unknown-type",
        "forged-reference",
    ] {
        let mut host = host(&[ADDON_A, ADDON_B]);
        let transaction = begin(&host, 1);
        let result = parse(&mut host, &transaction, "fixture default", mode, 1);
        assert_failure(&result, DefaultExpressionFailureKind::Unresolved);
        assert_eq!(write_count(&transaction), 0);
        assert_no_seed_state(&transaction);
        assert!(result.matches.alternatives.is_empty());
        assert!(result.effects.context_updates.is_empty());
        assert!(default_calls(&result).is_empty());
        transaction.cancel().unwrap();
    }
}

#[test]
fn later_omitted_capture_failure_discards_earlier_default() {
    let mut host = host(&[ADDON_A, ADDON_B]);
    let complete = begin(&host, 1);
    let result = parse(&mut host, &complete, "fixture pair", "resolve", 1);
    assert_eq!(selected(&result).children.len(), 2);
    assert_eq!(default_calls(&result).len(), 8);
    assert_eq!(
        selected(&result)
            .metadata
            .get("test.expression-data-b/default-child-count")
            .map(String::as_str),
        Some("2")
    );
    complete.cancel().unwrap();

    for (revision, mode, kind) in [
        (2, "reject-second", DefaultExpressionFailureKind::Rejected),
        (3, "trap-second", DefaultExpressionFailureKind::Unresolved),
    ] {
        let transaction = begin(&host, revision);
        let result = parse(&mut host, &transaction, "fixture pair", mode, revision);
        assert!(result.matches.selected.is_none());
        assert!(result.matches.alternatives.is_empty());
        let trace = result
            .matches
            .failure
            .as_ref()
            .and_then(|failure| failure.trace.as_ref())
            .expect("second omitted capture retains failure");
        assert!(has_capture_failure(trace, 1, kind));
        assert_eq!(write_count(&transaction), 0);
        assert_no_seed_state(&transaction);
        assert!(default_calls(&result).is_empty());
        transaction.cancel().unwrap();
    }
}

#[test]
fn matching_addon_keeps_failed_default_scopes_balanced() {
    for (source, mode, capture_index, kind) in [
        (
            "fixture custom",
            "reject",
            0,
            DefaultExpressionFailureKind::Rejected,
        ),
        (
            "fixture custom",
            "unresolved",
            0,
            DefaultExpressionFailureKind::Unresolved,
        ),
        (
            "fixture pair",
            "reject-second",
            1,
            DefaultExpressionFailureKind::Rejected,
        ),
        (
            "fixture pair",
            "trap-second",
            1,
            DefaultExpressionFailureKind::Unresolved,
        ),
    ] {
        // A registered Matching addon exercises dispatch even when its
        // registration selector does not handle this particular Expression.
        let addons: &[&[u8]] = if mode == "unresolved" {
            &[ADDON_A, MATCHING_ADDON]
        } else {
            &[ADDON_A, ADDON_B, MATCHING_ADDON]
        };
        let mut host = host(addons);
        let transaction = begin(&host, 1);
        let result = parse(&mut host, &transaction, source, mode, 1);
        assert!(result.matches.selected.is_none());
        assert!(result.matches.alternatives.is_empty());
        let trace = result
            .matches
            .failure
            .as_ref()
            .and_then(|failure| failure.trace.as_ref())
            .expect("failed default remains a recoverable candidate failure");
        assert!(has_capture_failure(trace, capture_index, kind));
        assert_eq!(write_count(&transaction), 0);
        assert_no_seed_state(&transaction);
        assert!(result.calls.is_empty());
        assert!(result.effects.context_updates.is_empty());
        assert!(result.effects.parse_requests.is_empty());
        assert!(result.effects.parse_results.is_empty());

        let recovered = parse(&mut host, &transaction, "fixture default", "resolve", 1);
        let value = child(&recovered);
        let ExpressionNodeKind::Default { info } = &value.kind else {
            panic!("a later parse must still resolve its omitted capture");
        };
        // A trapped component is disabled; the independently loaded B provider
        // must remain usable in the same host and transaction.
        let (component, order, writes) = match mode {
            "trap-second" => (COMPONENT_B, "seed", 1),
            "unresolved" => (COMPONENT_A, "seed>late>same", 3),
            _ => (COMPONENT_A, "seed>late>same", 4),
        };
        assert_eq!(info.component_id, component);
        assert_eq!(
            value
                .metadata
                .get(&format!("{component}/order"))
                .map(String::as_str),
            Some(order)
        );
        assert_eq!(write_count(&transaction), writes);
        transaction.cancel().unwrap();
    }
}

#[test]
fn replacement_provider_receives_its_own_host_stamped_identity() {
    let mut host = host(&[ADDON_A, ADDON_B]);
    let transaction = begin(&host, 1);
    let result = parse(
        &mut host,
        &transaction,
        "fixture default",
        "replace-owner",
        1,
    );
    let value = child(&result);
    let ExpressionNodeKind::Default { info } = &value.kind else {
        panic!("replacement default")
    };
    assert_eq!(info.component_id, COMPONENT_B);
    assert_eq!(info.provider_id, "default-test.b.replacement");
    assert_eq!(
        value.return_type.as_ref().map(ClassName::as_str),
        Some("java.lang.Long")
    );
    assert_eq!(
        value
            .metadata
            .get("test.expression-data-a/order")
            .map(String::as_str),
        Some("seed>late>same")
    );
    assert_eq!(write_count(&transaction), 4);
    transaction.cancel().unwrap();
}

#[test]
fn native_type_and_literal_restrictions_reject_provider_results_transactionally() {
    for (source, mode) in [
        ("fixture default", "wrong-type"),
        ("fixture default", "multiple"),
        ("fixture literal", "resolve"),
        ("fixture expression", "literal"),
    ] {
        let mut host = host(&[ADDON_A]);
        let transaction = begin(&host, 1);
        let result = parse(&mut host, &transaction, source, mode, 1);
        assert_failure(&result, DefaultExpressionFailureKind::Rejected);
        assert_eq!(write_count(&transaction), 0);
        transaction.cancel().unwrap();
    }
}

#[test]
fn provider_receives_time_and_accepts_literal_only_capture_when_literal() {
    let mut host = host(&[ADDON_A]);
    for (revision, source, mode, time, literal) in [
        (1, "fixture literal", "literal", 0, true),
        (2, "fixture timed", "resolve", -1, false),
    ] {
        let transaction = begin(&host, revision);
        let result = parse(&mut host, &transaction, source, mode, revision);
        let ExpressionNodeKind::Default { info } = &child(&result).kind else {
            panic!("default child")
        };
        assert_eq!(info.time, time);
        assert_eq!(info.is_literal, literal);
        transaction.cancel().unwrap();
    }
}

#[test]
fn cancelling_a_selected_default_does_not_publish_document_state() {
    let mut host = host(&[ADDON_A, ADDON_B]);
    let cancelled = begin(&host, 1);
    selected(&parse(
        &mut host,
        &cancelled,
        "fixture default",
        "resolve",
        1,
    ));
    assert_eq!(write_count(&cancelled), 4);
    cancelled.cancel().unwrap();
    let next = begin(&host, 2);
    for (component, key) in [
        (COMPONENT_A, "default-test.a.seed"),
        (COMPONENT_B, "default-test.b.enrich"),
    ] {
        let mut invocation = next.begin_invocation(component).unwrap();
        assert!(
            invocation
                .get(
                    StateScope::Document,
                    NamespaceVisibility::Private,
                    "default-expression-test",
                    key
                )
                .unwrap()
                .is_none()
        );
        invocation.rollback();
    }
    next.cancel().unwrap();
}

#[test]
fn effect_hook_reads_the_implicit_capture_from_shared_semantic_summary() {
    let mut host = host(&[ADDON_A, ADDON_B]);
    let transaction = begin(&host, 1);
    let source = MappedSource::identity("fixture effect");
    let raw = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 16));
    let result = host
        .parse_effect_in_parse(
            &transaction,
            context(1),
            EffectParseRequest {
                source: &source,
                node: raw.get(raw.roots[0]).unwrap(),
                context: ExpressionParseContext::default(),
            },
            EffectParserConfig::default(),
        )
        .expect("Effect default parse");
    let effect = result
        .matches
        .selected
        .as_ref()
        .expect("Effect candidate retained");
    assert_eq!(
        effect
            .metadata
            .get("test.expression-data-b/default-child-count")
            .map(String::as_str),
        Some("1")
    );
    assert!(matches!(
        effect.expressions().next().unwrap().kind,
        ExpressionNodeKind::Default { .. }
    ));
    assert_eq!(write_count(&transaction), 4);
    transaction.cancel().unwrap();
}

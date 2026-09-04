use parser_wasm::host::{HookCall, HostConfig, InvocationContext, ParserHost, RuntimeProfile};
use skript_parser::{
    ExpressionExpectedType, ExpressionNode, ExpressionNodeKind, ExpressionParseContext,
    ExpressionParseRequest, ExpressionParserConfig, MappedSource, TextRange,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use syntax_pattern_parser::syntax;
use syntaxes::{
    Catalog, CatalogParts, ClassName, DefinitionId, Multiplicity, Pattern,
    PossibleReturnTypesState, RegistrationId, ResolutionState, ReturnTypeState, Syntax,
};

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));
const TYPE_PARSER_ADDON: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/type-parser-addon.wasm"
));

const COMPONENT_ID: &str = "test.type-parser-addon";
const SUBSCRIPTION_ID: &str = "type-parser.number";
const DIRECT_NO_MATCH_CLASS: &str = "org.bukkit.enchantments.Enchantment";
const DIRECT_NO_MATCH_INPUT: &str = "direct-unmatched-enchantment";
const NO_APPLICABLE_CLASS: &str = "org.bukkit.Chunk";
const PARSER_ID: &str = "test.type-parser-addon.number-word";
const NUMBER_CLASS: &str = "java.lang.Number";
const NUMBER_PARSER_CLASS: &str = "fixture.NumberParser";
const BLOCK_DATA_CLASS: &str = "org.bukkit.block.data.BlockData";
const BLOCK_DATA_PARSER_CLASS: &str = "fixture.BlockDataParser";
const BLOCK_DATA_INPUT: &str = "fixture:block[axis=x]";
const BLOCK_DATA_REQUIRED_PROVIDER: &str = "fixture.block-data-registry";
const LOOT_TABLE_CLASS: &str = "org.bukkit.loot.LootTable";
const LOOT_TABLE_PARSER_CLASS: &str = "fixture.LootTableParser";
const LOOT_TABLE_INPUT: &str = "fixture:loot";
const ENCHANTMENT_TYPE_CLASS: &str = "ch.njol.skript.util.EnchantmentType";
const ENCHANTMENT_TYPE_PARSER_CLASS: &str = "fixture.EnchantmentTypeParser";
const ENCHANTMENT_TYPE_INPUT: &str = "fixture enchantment 5";
const SPECIAL_INPUT: &str = "forty-two";
const UNRESOLVED_INPUT: &str = "registry-number";
const REQUIRED_PROVIDER: &str = "fixture.number-registry";
const TYPE_PROVIDER_INPUT: &str = "shared-type-input";
const NATIVE_INVALID_INPUT: &str = "native-invalid-type-input";
const REGISTERED_EXPRESSION_INPUT: &str = "pi";
const TYPE_A_PARSER_ID: &str = "test.type-parser-addon.type-a";
const TYPE_A_STATE_KEY: &str = "type-a";
const TYPE_A_DIAGNOSTIC_CODE: &str = "type-parser-test.type-a";
const TYPE_B_PARSER_ID: &str = "test.type-parser-addon.type-b";
const TYPE_B_STATE_KEY: &str = "type-b";
const TYPE_B_DIAGNOSTIC_CODE: &str = "type-parser-test.type-b";
const INVALID_TYPE_A_STATE_KEY: &str = "type-a-invalid";
const INVALID_TYPE_A_DIAGNOSTIC_CODE: &str = "type-parser-test.type-a-invalid";
const DEFERRED_TYPE_STATE_KEY: &str = "deferred-type";
const DEFERRED_TYPE_DIAGNOSTIC_CODE: &str = "type-parser-test.deferred-type";
const NESTED_EXPRESSION_INPUT: &str = "wrapped shared-type-input";
const NESTED_EXPRESSION_REGISTRATION_ID: &str = "expression:test.type-parser-addon:nested:0";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn catalog() -> Arc<Catalog> {
    let snapshot = ssg::load(fixture()).expect("schema 3 SSG fixture must load");
    let source = snapshot.catalog();
    let mut syntaxes = source.syntaxes().to_vec();
    let mut found_number = false;
    let mut found_enchantment = false;
    let mut parser_backed_types = 0;
    for syntax in &mut syntaxes {
        let Syntax::Type(value) = syntax else {
            continue;
        };
        match value.code_name.as_str() {
            "number" => {
                value.parser_class = Some(ClassName(NUMBER_PARSER_CLASS.to_owned()));
                found_number = true;
            }
            "blockdata" => {
                value.parser_class = Some(ClassName(BLOCK_DATA_PARSER_CLASS.to_owned()));
                parser_backed_types += 1;
            }
            "loottable" => {
                value.parser_class = Some(ClassName(LOOT_TABLE_PARSER_CLASS.to_owned()));
                parser_backed_types += 1;
            }
            "enchantmenttype" => {
                value.parser_class = Some(ClassName(ENCHANTMENT_TYPE_PARSER_CLASS.to_owned()));
                parser_backed_types += 1;
            }
            // Schema 3 predates the finite registry literal export. Add one
            // deterministic value so this fixture can cover both parser paths.
            "enchantment" => {
                value
                    .literal_values
                    .extend(["sharpness".to_owned(), UNRESOLVED_INPUT.to_owned()]);
                found_enchantment = true;
            }
            _ => {}
        }
    }
    assert!(found_number, "fixture must contain the Number Type");
    assert!(
        found_enchantment,
        "fixture must contain the Enchantment Type"
    );
    assert_eq!(
        parser_backed_types, 3,
        "fixture must contain every environment-backed Type"
    );
    // Test-only registration, not a built-in Skript syntax: exercise Type effects
    // through a parent match as well as through the standalone Type entry point.
    let nested_pattern = "wrapped %number/blockdata%";
    let mut nested_expression = source
        .expressions()
        .find(|expression| {
            expression
                .common
                .patterns
                .iter()
                .any(|pattern| pattern.source == "dummy direct registry expression")
        })
        .expect("fixture must contain an Expression to clone for the nested Type test")
        .clone();
    nested_expression.common.registration_order = usize::MAX;
    nested_expression.common.definition_id =
        DefinitionId("expression:test.type-parser-addon:nested".to_owned());
    nested_expression.common.registration_id =
        RegistrationId(NESTED_EXPRESSION_REGISTRATION_ID.to_owned());
    nested_expression.common.element_class =
        ClassName("test.type-parser-addon.NestedExpression".to_owned());
    nested_expression.common.priority_name = None;
    nested_expression.common.priority = None;
    nested_expression.common.patterns = vec![Pattern {
        source: nested_pattern.to_owned(),
        parsed: syntax::parse(nested_pattern, source.plural_rules())
            .expect("nested Type test pattern must parse"),
    }];
    nested_expression.return_type = Some(ClassName("java.lang.Object".to_owned()));
    nested_expression.return_type_state = ReturnTypeState::Static;
    nested_expression.possible_return_types = vec![ClassName("java.lang.Object".to_owned())];
    nested_expression.possible_return_types_state = PossibleReturnTypesState::Complete;
    nested_expression.return_type_multiplicity = Some(Multiplicity::Single);
    nested_expression.return_type_multiplicity_state = ResolutionState::Resolved;
    syntaxes.push(Syntax::Expression(nested_expression));
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
        document_id: "file:///workspace/type-parser.sk".to_owned(),
        document_revision: revision,
        expansion: None,
        syntax_context: 3,
    }
}

fn parse_number(text: &str, revision: u64, addon: bool) -> parser_wasm::WasmExpressionParseResult {
    parse_typed(text, NUMBER_CLASS, revision, addon)
}

fn parse_typed(
    text: &str,
    expected_class: &str,
    revision: u64,
    addon: bool,
) -> parser_wasm::WasmExpressionParseResult {
    parse_typed_alternatives(text, &[expected_class], revision, addon)
}

fn parse_typed_alternatives(
    text: &str,
    expected_classes: &[&str],
    revision: u64,
    addon: bool,
) -> parser_wasm::WasmExpressionParseResult {
    let mut host = parser_host(addon);

    let transaction = host
        .begin_parse(
            "file:///workspace",
            "file:///workspace/type-parser.sk",
            revision,
        )
        .expect("parse must begin");
    let result =
        parse_typed_in_transaction(&mut host, &transaction, text, expected_classes, revision);
    transaction.cancel().expect("test parse may be cancelled");
    result
}

fn parser_host(addon: bool) -> ParserHost {
    parser_host_version("2.15.4", addon)
}

fn parser_host_version(skript_version: &str, addon: bool) -> ParserHost {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog()),
            runtime_profile: RuntimeProfile {
                skript_version: Some(skript_version.to_owned()),
                ..RuntimeProfile::default()
            },
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must initialize with the SSG catalog");
    if addon {
        host.load_addon(TYPE_PARSER_ADDON)
            .expect("external Type parser addon must initialize");
    }
    host
}

fn parse_typed_in_transaction(
    host: &mut ParserHost,
    transaction: &parser_wasm::state::ParseTransaction,
    text: &str,
    expected_classes: &[&str],
    revision: u64,
) -> parser_wasm::WasmExpressionParseResult {
    let source = MappedSource::identity(text);
    host.parse_expression_in_parse(
        transaction,
        context(revision),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: expected_classes
                .iter()
                .map(|class_name| ExpressionExpectedType {
                    class_name: ClassName((*class_name).to_owned()),
                    plural: false,
                })
                .collect(),
            context: ExpressionParseContext {
                syntax_context: 3,
                ..ExpressionParseContext::default()
            },
        },
        ExpressionParserConfig::default(),
    )
    .expect("typed expression parsing must complete")
}

fn selected_node(result: parser_wasm::WasmExpressionParseResult, label: &str) -> ExpressionNode {
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

fn call_index(calls: &[HookCall], component_id: &str, subscription_id: &str) -> usize {
    calls
        .iter()
        .position(|call| {
            call.component_id == component_id && call.subscription_id == subscription_id
        })
        .unwrap_or_else(|| panic!("missing hook call {component_id}/{subscription_id}: {calls:#?}"))
}

fn write_keys(transaction: &parser_wasm::state::ParseTransaction) -> Vec<String> {
    transaction
        .read_write_set()
        .expect("state access set must remain available")
        .writes
        .into_iter()
        .map(|write| write.key)
        .collect()
}

fn has_diagnostic(result: &parser_wasm::WasmExpressionParseResult, code: &str) -> bool {
    result
        .effects
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == code)
}

#[test]
fn direct_registration_no_match_is_distinct_from_missing_parser() {
    let missing = parse_typed(DIRECT_NO_MATCH_INPUT, DIRECT_NO_MATCH_CLASS, 10, false);
    assert!(missing.matches.selected.is_none());
    let trace = missing
        .matches
        .failure
        .as_ref()
        .and_then(|failure| failure.trace.as_ref())
        .expect("the absent direct Type parser must retain a failure trace");
    assert!(trace.root_cause().failure.reasons.iter().any(|reason| {
        matches!(
            reason,
            skript_parser::PatternFailureReason::TypeParserUnresolved { reason, .. }
                if reason.contains("no WASM Type parser")
        )
    }));

    let handled = parse_typed(DIRECT_NO_MATCH_INPUT, DIRECT_NO_MATCH_CLASS, 11, true);
    assert!(handled.matches.selected.is_none());
    let trace = handled
        .matches
        .failure
        .as_ref()
        .and_then(|failure| failure.trace.as_ref())
        .expect("the invalid direct Type input must retain a failure trace");
    assert!(!trace.root_cause().failure.reasons.iter().any(|reason| {
        matches!(
            reason,
            skript_parser::PatternFailureReason::TypeParserUnresolved { reason, .. }
                if reason.contains("no WASM Type parser")
        )
    }));
    assert!(
        handled.failures.is_empty(),
        "direct Type no-match must remain a normal parser result: {handled:#?}"
    );

    let no_applicable = parse_typed("unhandled-type-input", NO_APPLICABLE_CLASS, 11, true);
    let trace = no_applicable
        .matches
        .failure
        .as_ref()
        .and_then(|failure| failure.trace.as_ref())
        .expect("an unhandled Type must retain the missing-parser failure");
    assert!(trace.root_cause().failure.reasons.iter().any(|reason| {
        matches!(
            reason,
            skript_parser::PatternFailureReason::TypeParserUnresolved { reason, .. }
                if reason.contains("no WASM Type parser")
        )
    }));
}

#[test]
fn competing_type_providers_keep_only_selected_effects_across_same_transaction() {
    let mut host = parser_host(true);
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/type-parser.sk", 12)
        .expect("parse must begin");

    let result = parse_typed_in_transaction(
        &mut host,
        &transaction,
        TYPE_PROVIDER_INPUT,
        &["java.lang.Object"],
        12,
    );
    let selected = result
        .matches
        .selected
        .as_ref()
        .expect("Type A must win the shared input");
    assert!(matches!(
        &selected.node.kind,
        ExpressionNodeKind::Literal { parser_id } if parser_id == TYPE_A_PARSER_ID
    ));
    assert!(has_diagnostic(&result, TYPE_A_DIAGNOSTIC_CODE));
    assert!(!has_diagnostic(&result, TYPE_B_DIAGNOSTIC_CODE));
    let keys = write_keys(&transaction);
    assert!(keys.iter().any(|key| key == TYPE_A_STATE_KEY));
    assert!(!keys.iter().any(|key| key == TYPE_B_STATE_KEY));

    let second = parse_typed_in_transaction(
        &mut host,
        &transaction,
        TYPE_PROVIDER_INPUT,
        &["java.lang.Object"],
        12,
    );
    let selected = second
        .matches
        .selected
        .as_ref()
        .expect("the second parse must also select Type A");
    assert!(matches!(
        &selected.node.kind,
        ExpressionNodeKind::Literal { parser_id } if parser_id == TYPE_A_PARSER_ID
    ));
    assert!(has_diagnostic(&second, TYPE_A_DIAGNOSTIC_CODE));
    assert!(!has_diagnostic(&second, TYPE_B_DIAGNOSTIC_CODE));
    let keys = write_keys(&transaction);
    assert!(!keys.iter().any(|key| key == TYPE_B_STATE_KEY));
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn native_invalid_type_candidate_is_discarded_when_type_b_succeeds() {
    let mut host = parser_host(true);
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/type-parser.sk", 13)
        .expect("parse must begin");
    let result = parse_typed_in_transaction(
        &mut host,
        &transaction,
        NATIVE_INVALID_INPUT,
        &["java.lang.Object"],
        13,
    );
    let selected = result
        .matches
        .selected
        .as_ref()
        .expect("Type B must survive native candidate validation");
    assert!(matches!(
        &selected.node.kind,
        ExpressionNodeKind::Literal { parser_id } if parser_id == TYPE_B_PARSER_ID
    ));
    assert!(has_diagnostic(&result, TYPE_B_DIAGNOSTIC_CODE));
    assert!(!has_diagnostic(&result, INVALID_TYPE_A_DIAGNOSTIC_CODE));
    assert!(!result.matches.alternatives.iter().any(|candidate| {
        matches!(
            &candidate.node.kind,
            ExpressionNodeKind::Literal { parser_id } if parser_id == TYPE_A_PARSER_ID
        )
    }));
    let keys = write_keys(&transaction);
    assert!(keys.iter().any(|key| key == TYPE_B_STATE_KEY));
    assert!(!keys.iter().any(|key| key == INVALID_TYPE_A_STATE_KEY));
    assert!(
        result.failures.is_empty(),
        "native rejection leaked a failure: {result:#?}"
    );
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn registered_expression_beats_a_deferred_type_candidate() {
    let mut host = parser_host(true);
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/type-parser.sk", 14)
        .expect("parse must begin");
    let result = parse_typed_in_transaction(
        &mut host,
        &transaction,
        REGISTERED_EXPRESSION_INPUT,
        &["java.lang.Object"],
        14,
    );
    let selected = result
        .matches
        .selected
        .as_ref()
        .expect("the registered Pi expression must be selected");
    assert!(matches!(
        &selected.node.kind,
        ExpressionNodeKind::Registered { .. }
    ));
    assert_eq!(
        selected.node.return_type.as_ref().map(ClassName::as_str),
        Some("java.lang.Double")
    );
    assert!(!has_diagnostic(&result, DEFERRED_TYPE_DIAGNOSTIC_CODE));
    let keys = write_keys(&transaction);
    assert!(!keys.iter().any(|key| key == DEFERRED_TYPE_STATE_KEY));
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn core_number_parser_rejects_external_word_input_without_addon() {
    let result = parse_number(SPECIAL_INPUT, 1, false);
    assert!(
        result.matches.selected.is_none(),
        "CoreLibrary must not accept the fixture-only word input: {result:#?}"
    );
    assert!(
        result.failures.is_empty(),
        "CoreLibrary failed unexpectedly: {result:#?}"
    );
}

#[test]
fn external_type_parser_accepts_a_typed_leaf_through_the_number_registration() {
    let catalog = catalog();
    let (number_definition_id, number_registration_id) = catalog
        .types()
        .find(|value| value.code_name.as_str() == "number")
        .map(|value| {
            (
                value.definition_id.as_str().to_owned(),
                value.registration_id.as_str().to_owned(),
            )
        })
        .expect("SSG fixture must contain the Number Type registration");

    let result = parse_number(SPECIAL_INPUT, 2, true);
    let selected = result
        .matches
        .selected
        .as_ref()
        .expect("the external Type parser must select a candidate");
    assert!(matches!(
        &selected.node.kind,
        ExpressionNodeKind::Literal { parser_id } if parser_id == PARSER_ID
    ));
    assert_eq!(
        selected.node.return_type.as_ref().map(ClassName::as_str),
        Some(NUMBER_CLASS)
    );
    assert_eq!(selected.node.multiplicity, Some(Multiplicity::Single));
    assert_eq!(
        selected
            .node
            .metadata
            .get("test.type-parser-addon/special-input")
            .map(String::as_str),
        Some(SPECIAL_INPUT)
    );
    assert_eq!(
        selected
            .node
            .metadata
            .get("test.type-parser-addon/active-type-code-name")
            .map(String::as_str),
        Some("number")
    );
    assert_eq!(
        selected
            .node
            .metadata
            .get("test.type-parser-addon/active-type-definition-id")
            .map(String::as_str),
        Some(number_definition_id.as_str())
    );
    assert_eq!(
        selected
            .node
            .metadata
            .get("test.type-parser-addon/active-type-registration-id")
            .map(String::as_str),
        Some(number_registration_id.as_str())
    );
    assert_eq!(
        selected
            .node
            .metadata
            .get("test.type-parser-addon/active-type-parser-class")
            .map(String::as_str),
        Some(NUMBER_PARSER_CLASS)
    );
    assert!(
        result.failures.is_empty(),
        "addon dispatch failed: {result:#?}"
    );

    let core_type = call_index(&result.calls, "nlaocs.core-library", "core.type-candidates");
    let addon_type = call_index(&result.calls, COMPONENT_ID, SUBSCRIPTION_ID);
    assert!(
        core_type < addon_type,
        "CoreLibrary Type handling must precede the external addon: {result:#?}"
    );
}

#[test]
fn core_number_parser_still_wins_for_standard_numeric_input() {
    let result = parse_number("42", 3, true);
    let node = selected_node(result, "standard Number input");
    assert!(matches!(
        &node.kind,
        ExpressionNodeKind::Literal { parser_id } if parser_id == "core.literal.number"
    ));
    assert!(
        node.return_type.is_some(),
        "CoreLibrary must provide a concrete numeric return type"
    );
    assert_eq!(node.multiplicity, Some(Multiplicity::Single));
}

#[test]
fn external_type_parser_reports_a_required_provider_without_rejecting_the_type() {
    let result = parse_number(UNRESOLVED_INPUT, 4, true);
    assert!(result.matches.selected.is_none());
    let trace = result
        .matches
        .failure
        .as_ref()
        .and_then(|failure| failure.trace.as_ref())
        .expect("an unresolved Type provider must remain in the failure trace");
    assert!(trace.root_cause().failure.reasons.iter().any(|reason| {
        matches!(
            reason,
            skript_parser::PatternFailureReason::TypeParserUnresolved {
                parser_class: Some(parser_class),
                required_provider: Some(provider),
                ..
            } if parser_class == NUMBER_PARSER_CLASS && provider == REQUIRED_PROVIDER
        )
    }));
    assert!(
        result.failures.is_empty(),
        "unresolved is a normal parser outcome, not a component failure: {result:#?}"
    );
}

#[test]
fn exact_parser_backed_types_without_an_addon_report_their_registration_route() {
    for (revision, input, class) in [
        (5, BLOCK_DATA_INPUT, BLOCK_DATA_CLASS),
        (6, LOOT_TABLE_INPUT, LOOT_TABLE_CLASS),
    ] {
        let result = parse_typed(input, class, revision, false);
        assert!(result.matches.selected.is_none());
        let trace = result
            .matches
            .failure
            .as_ref()
            .and_then(|failure| failure.trace.as_ref())
            .expect("a parser-backed Type without a provider must be diagnosable");
        assert!(trace.root_cause().failure.reasons.iter().any(|reason| {
            matches!(
                reason,
                skript_parser::PatternFailureReason::TypeParserUnresolved {
                    required_provider: Some(provider),
                    reason,
                    ..
                } if provider.starts_with("type-parser/type:skript:")
                    && reason.contains("no WASM Type parser")
            )
        }));
    }
}

#[test]
fn preserves_every_required_provider_across_type_alternatives() {
    let result =
        parse_typed_alternatives(UNRESOLVED_INPUT, &[NUMBER_CLASS, BLOCK_DATA_CLASS], 7, true);
    assert!(result.matches.selected.is_none());
    let trace = result
        .matches
        .failure
        .as_ref()
        .and_then(|failure| failure.trace.as_ref())
        .expect("all unresolved Type providers must remain in one failure trace");
    let providers = trace
        .root_cause()
        .failure
        .reasons
        .iter()
        .filter_map(|reason| match reason {
            skript_parser::PatternFailureReason::TypeParserUnresolved {
                required_provider, ..
            } => required_provider.as_deref(),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(providers.contains(&REQUIRED_PROVIDER), "trace={trace:#?}");
    assert!(
        providers.contains(&BLOCK_DATA_REQUIRED_PROVIDER),
        "trace={trace:#?}"
    );
}

#[test]
fn enchantment_type_uses_snapshot_values_and_reports_unknown_registry_entries() {
    let known = parse_typed("sharpness 5", ENCHANTMENT_TYPE_CLASS, 8, false);
    let node = selected_node(known, "known enchantment with level");
    assert_eq!(
        node.metadata
            .get("nlaocs.core-library/enchantment-level")
            .map(String::as_str),
        Some("5")
    );
    assert_eq!(
        node.metadata
            .get("nlaocs.core-library/enchantment")
            .map(String::as_str),
        Some("sharpness")
    );

    let unknown = parse_typed(ENCHANTMENT_TYPE_INPUT, ENCHANTMENT_TYPE_CLASS, 9, false);
    assert!(unknown.matches.selected.is_none());
    let trace = unknown
        .matches
        .failure
        .as_ref()
        .and_then(|failure| failure.trace.as_ref())
        .expect("unknown enchantment must preserve its provider requirement");
    assert!(trace.root_cause().failure.reasons.iter().any(|reason| {
        matches!(
            reason,
            skript_parser::PatternFailureReason::TypeParserUnresolved {
                required_provider: Some(provider),
                ..
            } if provider == "minecraft.registry.enchantment"
        )
    }));
}

#[test]
fn external_registry_provider_resolves_environment_backed_types() {
    for (input, class, parser_id, canonical) in [
        (
            BLOCK_DATA_INPUT,
            BLOCK_DATA_CLASS,
            "test.type-parser-addon.block-data",
            "fixture:block",
        ),
        (
            LOOT_TABLE_INPUT,
            LOOT_TABLE_CLASS,
            "test.type-parser-addon.loot-table",
            LOOT_TABLE_INPUT,
        ),
        (
            ENCHANTMENT_TYPE_INPUT,
            ENCHANTMENT_TYPE_CLASS,
            "test.type-parser-addon.enchantment-type",
            "fixture enchantment",
        ),
    ] {
        let node = selected_node(
            parse_typed(input, class, 20 + input.len() as u64, true),
            input,
        );
        assert!(matches!(
            &node.kind,
            ExpressionNodeKind::Literal { parser_id: selected } if selected == parser_id
        ));
        assert_eq!(
            node.return_type.as_ref().map(ClassName::as_str),
            Some(class)
        );
        assert_eq!(
            node.metadata
                .get("test.type-parser-addon/provider-identity")
                .map(String::as_str),
            Some(COMPONENT_ID)
        );
        assert_eq!(
            node.metadata
                .get("test.type-parser-addon/canonical-value")
                .map(String::as_str),
            Some(canonical)
        );
    }
}

#[test]
// RuntimeProfile routing only; real-version metadata compatibility lives in type_parser_versions.rs.
fn standard_type_dispatch_uses_the_runtime_version_profile() {
    for (revision, version) in [(40, "2.6.4"), (41, "2.15.4"), (42, "2.16.0")] {
        let mut host = parser_host_version(version, false);
        let transaction = host
            .begin_parse(
                "file:///workspace",
                "file:///workspace/type-parser.sk",
                revision,
            )
            .expect("parse must begin");
        let node = selected_node(
            parse_typed_in_transaction(
                &mut host,
                &transaction,
                "1 second",
                &["ch.njol.skript.util.Timespan"],
                revision,
            ),
            version,
        );
        assert!(matches!(
            &node.kind,
            ExpressionNodeKind::Literal { parser_id } if parser_id == "core.literal.timespan"
        ));
        transaction.cancel().expect("test parse may be cancelled");
    }
}

#[test]
fn nested_type_parse_keeps_selected_provider_effects_at_registered_root() {
    let mut host = parser_host(true);
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/type-parser.sk", 15)
        .expect("parse must begin");
    let result = parse_typed_in_transaction(
        &mut host,
        &transaction,
        NESTED_EXPRESSION_INPUT,
        &["java.lang.Object"],
        15,
    );
    let selected = result
        .matches
        .selected
        .as_ref()
        .expect("the synthetic registered Expression must be selected");
    assert!(matches!(
        &selected.node.kind,
        ExpressionNodeKind::Registered {
            registration_id, ..
        } if registration_id == NESTED_EXPRESSION_REGISTRATION_ID
    ));
    assert_eq!(
        selected.node.children.len(),
        1,
        "the root must retain its typed child"
    );
    let child = &selected.node.children[0];
    assert!(matches!(
        &child.kind,
        ExpressionNodeKind::Literal { parser_id } if parser_id == TYPE_A_PARSER_ID
    ));
    assert!(has_diagnostic(&result, TYPE_A_DIAGNOSTIC_CODE));
    assert!(!has_diagnostic(&result, TYPE_B_DIAGNOSTIC_CODE));
    let keys = write_keys(&transaction);
    assert!(keys.iter().any(|key| key == TYPE_A_STATE_KEY));
    assert!(!keys.iter().any(|key| key == TYPE_B_STATE_KEY));
    assert!(
        result.failures.is_empty(),
        "nested Type dispatch failed: {result:#?}"
    );
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn unresolved_type_provider_state_is_rolled_back_when_another_type_wins() {
    let mut host = parser_host(true);
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/type-parser.sk", 8)
        .expect("parse must begin");
    let result = parse_typed_in_transaction(
        &mut host,
        &transaction,
        UNRESOLVED_INPUT,
        &["java.lang.Object"],
        8,
    );

    let node = selected_node(result, "finite literal fallback");
    assert!(matches!(node.kind, ExpressionNodeKind::Literal { .. }));
    assert!(
        transaction
            .read_write_set()
            .expect("state access set must remain available")
            .writes
            .iter()
            .all(|write| write.key != "unresolved-number"),
        "the unresolved Number provider write must not survive the selected Enchantment Type"
    );
    transaction.cancel().expect("test parse may be cancelled");
}

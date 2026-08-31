use parser_wasm::host::{
    HostConfig, HostError, InvocationContext, ParserHost, RuntimeProfile, WasmEffectParseResult,
    WasmPatternMatchResult,
};
use skript_parser::{
    EffectParseRequest, EffectParserConfig, ExpressionParseContext, MappedSource, MatchInput,
    MatchPattern, MatchSyntaxKind, PatternCandidate, PatternMatcherConfig, RawTreeOptions,
    RejectTypeExpressions, TextRange, parse_raw_tree,
};
use std::path::Path;
use std::sync::Arc;
use syntax_pattern_parser::syntax::PluralRules;
use syntaxes::{Catalog, CatalogParts, RegistrationId, Syntax};

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));
const MATCHING_ADDON: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/matching-addon.wasm"
));
const EFFECT_ADDON: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/effect-addon.wasm"
));

fn context() -> InvocationContext {
    InvocationContext {
        invocation_id: 1,
        subscription_id: String::new(),
        document_id: "file:///workspace/test.sk".to_owned(),
        document_revision: 1,
        expansion: None,
        syntax_context: 0,
    }
}

fn core_config() -> HostConfig {
    HostConfig {
        runtime_profile: RuntimeProfile {
            skript_version: Some("2.15.4".to_owned()),
            ..RuntimeProfile::default()
        },
        ..HostConfig::default()
    }
}

fn assert_failed_candidate_has_no_side_effects(
    result: &WasmPatternMatchResult,
    transaction: &parser_wasm::state::ParseTransaction,
) {
    assert!(result.matches.selected.is_none());
    assert!(
        result.calls.is_empty(),
        "failed candidate calls leaked: {:#?}",
        result.calls
    );
    assert!(
        result.effects.context_updates.is_empty(),
        "failed candidate context updates leaked: {:#?}",
        result.effects.context_updates
    );
    assert!(
        result.effects.diagnostics.is_empty(),
        "failed candidate diagnostics leaked: {:#?}",
        result.effects.diagnostics
    );
    assert!(
        result.effects.parse_requests.is_empty(),
        "failed candidate parse requests leaked: {:#?}",
        result.effects.parse_requests
    );
    assert!(
        result.effects.parse_results.is_empty(),
        "failed candidate parse results leaked: {:#?}",
        result.effects.parse_results
    );
    assert!(
        result.failures.is_empty(),
        "failed candidate component failures leaked"
    );
    assert!(
        transaction.read_write_set().unwrap().writes.is_empty(),
        "failed candidate StateStore writes leaked: {:#?}",
        transaction.read_write_set().unwrap().writes
    );
}

fn effect_fixture() -> impl AsRef<Path> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn semantic_fallback_catalog() -> Arc<Catalog> {
    let snapshot = ssg::load(effect_fixture()).expect("schema 3 fixture must load");
    let source = snapshot.catalog();
    let source_view = source.source().cloned().expect("SSG source view");
    let mut syntaxes = source
        .syntaxes()
        .iter()
        .filter(|syntax| match syntax {
            Syntax::Type(value) => matches!(value.code_name.as_str(), "string" | "object"),
            Syntax::Effect(value) => value
                .common
                .patterns
                .iter()
                .any(|pattern| pattern.source == "run dummy fixture effect [with %-string%]"),
            _ => false,
        })
        .cloned()
        .collect::<Vec<_>>();
    let rejected = syntaxes
        .iter()
        .find(|syntax| {
            matches!(
                syntax,
                Syntax::Effect(value)
                    if value.common.registration_id.as_str()
                        == "effect:skriptdummyaddon:224a969f6e9d408a3346b355ad040b4e4d82122708036cc40768e2d594725925:b3845096cfe66e4b677f17594ff4e1c1046c24b7526fe6452fbaabb0d9007f99:0"
            )
        })
        .cloned()
        .expect("fixture must contain the rejecting Effect");
    let Syntax::Effect(mut fallback) = rejected else {
        unreachable!("the fixture candidate is an Effect");
    };
    fallback.common.registration_id = RegistrationId("effect:test:semantic-fallback".to_owned());
    fallback.common.registration_order = fallback.common.registration_order.saturating_add(1_000);
    syntaxes.push(Syntax::Effect(fallback));

    Arc::new(
        Catalog::new(CatalogParts {
            syntaxes,
            converters: Vec::new(),
            comparators: Vec::new(),
            event_values: Vec::new(),
            properties: Vec::new(),
            operators: Vec::new(),
            operations: source.operations().clone(),
            differences: Vec::new(),
            classes: Vec::new(),
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

fn parse_effect_for_test(
    host: &mut ParserHost,
    transaction: &parser_wasm::state::ParseTransaction,
    revision: u64,
    text: &str,
) -> WasmEffectParseResult {
    let source = MappedSource::identity(text);
    let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
    let node = tree
        .roots
        .first()
        .and_then(|id| tree.get(*id))
        .expect("one Simple node");
    host.parse_effect_in_parse(
        transaction,
        InvocationContext {
            invocation_id: revision,
            subscription_id: String::new(),
            document_id: "file:///workspace/test.sk".to_owned(),
            document_revision: revision,
            expansion: None,
            syntax_context: 0,
        },
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
fn wasm_matching_hook_overrides_elements_and_keeps_only_selected_candidate_state() {
    let mut host = ParserHost::new(CORE_LIBRARY, core_config()).expect("CoreLibrary must load");
    host.load_addon(MATCHING_ADDON)
        .expect("matching addon must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 1)
        .unwrap();

    let rules = PluralRules::from_json(include_str!(
        "../../syntax-pattern-parser/tests/data/PluralRules-2.15.4.json"
    ))
    .unwrap();
    let pattern = syntax_pattern_parser::syntax::parse("<.+>", &rules).unwrap();
    let candidates = [
        PatternCandidate {
            kind: MatchSyntaxKind::Effect,
            definition_id: "effect:selected".to_owned(),
            registration_id: "effect:hook-override#0".to_owned(),
            priority: 0,
            registration_order: 0,
            resolved_order: None,
            patterns: vec![MatchPattern {
                pattern_index: 0,
                source: "<.+>",
                parsed: &pattern,
            }],
        },
        PatternCandidate {
            kind: MatchSyntaxKind::Effect,
            definition_id: "effect:alternative".to_owned(),
            registration_id: "effect:hook-override#0".to_owned(),
            priority: 0,
            registration_order: 1,
            resolved_order: None,
            patterns: vec![MatchPattern {
                pattern_index: 0,
                source: "<.+>",
                parsed: &pattern,
            }],
        },
    ];
    let source = MappedSource::identity("handled");
    let result = host
        .match_patterns_in_parse(
            &transaction,
            context(),
            MatchInput::from_source(&source, TextRange::new(0, 7)).unwrap(),
            &candidates,
            &mut RejectTypeExpressions,
            PatternMatcherConfig::default(),
        )
        .unwrap();

    assert_eq!(
        result.matches.selected.as_ref().unwrap().definition_id,
        "effect:selected"
    );
    assert_eq!(result.matches.alternatives.len(), 1);
    assert_eq!(
        result.matches.alternatives[0].definition_id,
        "effect:alternative"
    );
    assert!(
        result
            .calls
            .iter()
            .any(|call| call.component_id == "test.matching-addon")
    );
    assert!(result.failures.is_empty());
    assert!(!result.effects.context_updates.is_empty());
    assert!(
        result
            .effects
            .context_updates
            .iter()
            .all(|update| update.key == "effect:selected")
    );

    let writes = transaction.read_write_set().unwrap().writes;
    assert!(!writes.is_empty());
    assert!(
        writes
            .iter()
            .all(|write| write.key.starts_with("effect:selected:"))
    );
    assert!(
        writes
            .iter()
            .all(|write| !write.key.starts_with("effect:alternative:"))
    );
    transaction.commit().unwrap();
}

#[test]
fn regex_patterns_require_a_wasm_matching_handler() {
    let mut host = ParserHost::new(CORE_LIBRARY, core_config()).expect("CoreLibrary must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 1)
        .unwrap();
    let rules = PluralRules::from_json(include_str!(
        "../../syntax-pattern-parser/tests/data/PluralRules-2.15.4.json"
    ))
    .unwrap();
    let pattern = syntax_pattern_parser::syntax::parse("<.+>", &rules).unwrap();
    let candidates = [PatternCandidate {
        kind: MatchSyntaxKind::Effect,
        definition_id: "effect:without-handler".to_owned(),
        registration_id: "effect:without-handler#0".to_owned(),
        priority: 0,
        registration_order: 0,
        resolved_order: None,
        patterns: vec![MatchPattern {
            pattern_index: 0,
            source: "<.+>",
            parsed: &pattern,
        }],
    }];
    let source = MappedSource::identity("handled");

    let result = host
        .match_patterns_in_parse(
            &transaction,
            context(),
            MatchInput::from_source(&source, TextRange::new(0, 7)).unwrap(),
            &candidates,
            &mut RejectTypeExpressions,
            PatternMatcherConfig::default(),
        )
        .unwrap();

    assert!(result.matches.selected.is_none());
    assert!(result.matches.alternatives.is_empty());
    assert!(result.matches.primary_failure().is_none());
    assert!(result.calls.is_empty());
    transaction.cancel().unwrap();
}

#[test]
fn failed_registration_scope_rolls_back_matching_state_calls_and_effects() {
    let mut host = ParserHost::new(CORE_LIBRARY, core_config()).expect("CoreLibrary must load");
    host.load_addon(MATCHING_ADDON)
        .expect("matching addon must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 1)
        .unwrap();
    let source = MappedSource::identity("handled");
    let result = host
        .match_patterns_in_parse(
            &transaction,
            context(),
            MatchInput::from_source(&source, TextRange::new(0, source.virtual_source().len()))
                .unwrap(),
            &[PatternCandidate {
                kind: MatchSyntaxKind::Effect,
                definition_id: "effect:registration-failure".to_owned(),
                registration_id: "effect:hook-override#0".to_owned(),
                priority: 0,
                registration_order: 0,
                resolved_order: None,
                patterns: Vec::new(),
            }],
            &mut RejectTypeExpressions,
            PatternMatcherConfig::default(),
        )
        .unwrap();

    // The registration hook ran and recorded the attempt, but an empty
    // registration cannot be selected. Nothing from that failed branch may
    // escape the candidate transaction.
    assert_failed_candidate_has_no_side_effects(&result, &transaction);
    transaction.cancel().unwrap();
}

#[test]
fn failed_pattern_scope_rolls_back_matching_state_calls_and_effects() {
    let mut host = ParserHost::new(CORE_LIBRARY, core_config()).expect("CoreLibrary must load");
    host.load_addon(MATCHING_ADDON)
        .expect("matching addon must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 1)
        .unwrap();
    // Use a deliberately empty parsed AST to isolate Pattern-scope failure:
    // the matching fixture overrides every Element that it sees, so an empty
    // AST lets the non-empty input fail after Pattern hooks without creating
    // an unrelated Element match.
    let pattern = syntax_pattern_parser::syntax::ParseResult {
        elements: Vec::new(),
        warnings: Vec::new(),
    };
    let source = MappedSource::identity("handled");
    let result = host
        .match_patterns_in_parse(
            &transaction,
            context(),
            MatchInput::from_source(&source, TextRange::new(0, source.virtual_source().len()))
                .unwrap(),
            &[PatternCandidate {
                kind: MatchSyntaxKind::Effect,
                definition_id: "effect:pattern-failure".to_owned(),
                registration_id: "effect:hook-override#0".to_owned(),
                priority: 0,
                registration_order: 0,
                resolved_order: None,
                patterns: vec![MatchPattern {
                    pattern_index: 0,
                    source: "synthetic pattern",
                    parsed: &pattern,
                }],
            }],
            &mut RejectTypeExpressions,
            PatternMatcherConfig::default(),
        )
        .unwrap();

    // The Pattern hook is reached even though the synthetic empty pattern
    // cannot consume the non-empty input. Its parse-scoped write and matching
    // call must be discarded together with the failed candidate.
    assert_failed_candidate_has_no_side_effects(&result, &transaction);
    transaction.cancel().unwrap();
}

#[test]
fn failed_element_scope_error_rolls_back_matching_state_before_returning() {
    let mut host = ParserHost::new(CORE_LIBRARY, core_config()).expect("CoreLibrary must load");
    host.load_addon(MATCHING_ADDON)
        .expect("matching addon must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 1)
        .unwrap();
    let rules = PluralRules::from_json(include_str!(
        "../../syntax-pattern-parser/tests/data/PluralRules-2.15.4.json"
    ))
    .unwrap();
    let pattern = syntax_pattern_parser::syntax::parse("<.+>", &rules).unwrap();
    let source = MappedSource::identity("handled ");
    let error = host
        .match_patterns_in_parse(
            &transaction,
            context(),
            MatchInput::from_source(&source, TextRange::new(0, source.virtual_source().len()))
                .unwrap(),
            &[PatternCandidate {
                kind: MatchSyntaxKind::Effect,
                definition_id: "effect:element-failure".to_owned(),
                registration_id: "effect:hook-override#0".to_owned(),
                priority: 0,
                registration_order: 0,
                resolved_order: None,
                patterns: vec![MatchPattern {
                    pattern_index: 0,
                    source: "<.+>",
                    parsed: &pattern,
                }],
            }],
            &mut RejectTypeExpressions,
            PatternMatcherConfig::default(),
        )
        .expect_err("the fixture's full-range element override exceeds the trimmed input");

    assert!(matches!(
        error,
        HostError::PatternMatcher(skript_parser::PatternMatchError::InvalidInputRange { .. })
    ));
    // This is the error path, so no result can expose calls/effects. The
    // transaction assertion proves that the Element hook's attempted write
    // was nevertheless rolled back before the error escaped.
    assert!(transaction.read_write_set().unwrap().writes.is_empty());
    transaction.cancel().unwrap();
}

#[test]
fn semantic_rejection_rolls_back_before_the_next_effect_candidate() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(semantic_fallback_catalog()),
            ..core_config()
        },
    )
    .expect("CoreLibrary must load");
    host.load_addon(EFFECT_ADDON)
        .expect("Effect addon must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 1)
        .unwrap();

    // The fixture addon rejects the first registration after structural
    // matching and writes `reject` plus rejection-only HookEffects. The
    // cloned registration is structurally identical but is not subscribed to
    // that rejection hook, so it must become the selected fallback.
    let result = parse_effect_for_test(
        &mut host,
        &transaction,
        1,
        "run dummy fixture effect with \"metadata\"",
    );
    let selected = result
        .matches
        .selected
        .expect("fallback Effect must be selected after rejection");
    assert_eq!(
        selected.matched.registration_id,
        "effect:test:semantic-fallback"
    );
    assert!(
        transaction
            .read_write_set()
            .unwrap()
            .writes
            .iter()
            .all(|write| write.key != "reject"),
        "semantic rejection StateStore writes leaked into the fallback"
    );
    assert!(
        result
            .calls
            .iter()
            .all(|call| call.subscription_id != "effect.reject"),
        "semantic rejection call leaked into the fallback"
    );
    assert!(
        result
            .effects
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "effect-fixture-reject-effects"),
        "semantic rejection HookEffects diagnostic leaked into the fallback"
    );
    assert!(
        result
            .effects
            .context_updates
            .iter()
            .all(|update| update.key != "reject-effects-must-be-rolled-back"),
        "semantic rejection context update leaked into the fallback"
    );
    transaction.cancel().unwrap();
}

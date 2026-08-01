use parser_wasm::host::{HostConfig, InvocationContext, ParserHost};
use skript_parser::{
    MappedSource, MatchInput, MatchPattern, MatchSyntaxKind, PatternCandidate,
    PatternMatcherConfig, RejectTypeExpressions, TextRange,
};
use syntax_pattern_parser::syntax::PluralRules;

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));
const MATCHING_ADDON: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/matching-addon.wasm"
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

#[test]
fn wasm_matching_hook_overrides_elements_and_keeps_only_selected_candidate_state() {
    let mut host =
        ParserHost::new(CORE_LIBRARY, HostConfig::default()).expect("CoreLibrary must load");
    host.load_addon(MATCHING_ADDON)
        .expect("matching addon must load");
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 1)
        .unwrap();

    let rules = PluralRules::from_json(include_str!(
        "../../syntax-pattern-parser/tests/data/PluralRules-2.15.4.json"
    ))
    .unwrap();
    let pattern = syntax_pattern_parser::syntax::parse("never", &rules).unwrap();
    let candidates = [
        PatternCandidate {
            kind: MatchSyntaxKind::Effect,
            definition_id: "effect:selected".to_owned(),
            registration_id: "effect:hook-override#0".to_owned(),
            priority: 0,
            registration_order: 0,
            resolved_order: None,
            patterns: vec![MatchPattern {
                source: "never",
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
                source: "never",
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

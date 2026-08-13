use proptest::prelude::*;
use skript_parser::{
    MappedSource, MatchInput, MatchPattern, MatchSyntaxKind, NoopPatternMatchHooks,
    PatternCandidate, PatternMatcherConfig, RejectTypeExpressions, TextRange,
    match_pattern_candidates,
};
use syntax_pattern_parser::syntax::{self, PluralRules};

fn parse(source: &str) -> syntax_pattern_parser::syntax::ParseResult {
    let rules = PluralRules::from_json(include_str!(
        "../../syntax-pattern-parser/tests/data/PluralRules-2.15.4.json"
    ))
    .unwrap();
    syntax::parse(source, &rules).unwrap()
}

fn run(input: &str, source: &str, parsed: &syntax_pattern_parser::syntax::ParseResult) {
    let mapped = MappedSource::identity(input);
    let execute = || {
        match_pattern_candidates(
            MatchInput::from_source(&mapped, TextRange::new(0, input.len())).unwrap(),
            &[PatternCandidate {
                kind: MatchSyntaxKind::Condition,
                definition_id: "condition:property".to_owned(),
                registration_id: "condition:property#0".to_owned(),
                priority: 0,
                registration_order: 0,
                resolved_order: None,
                patterns: vec![MatchPattern {
                    pattern_index: 0,
                    source,
                    parsed,
                }],
            }],
            &mut RejectTypeExpressions,
            &mut NoopPatternMatchHooks,
            PatternMatcherConfig::default(),
        )
        .unwrap()
    };
    assert_eq!(execute(), execute());
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        max_shrink_iters: 2048,
        ..ProptestConfig::default()
    })]

    #[test]
    fn arbitrary_utf8_inputs_are_deterministic_and_panic_free(
        input in prop::collection::vec(any::<char>(), 0..96)
            .prop_map(|chars| chars.into_iter().collect::<String>())
    ) {
        let patterns = [
            ("literal", parse("literal")),
            ("choice", parse("(alpha|beta|)")),
            ("optional", parse("[prefix ][(deep|nested)] value")),
            ("regex", parse("<(.+)>")),
            ("metadata", parse("(left:one|right:two) [1¦old]")),
        ];
        for (source, parsed) in &patterns {
            run(&input, source, parsed);
        }
    }
}

#[test]
fn deeply_ambiguous_optionals_are_deterministic() {
    let source = "[a][a][a][a][a][a][a][a][a][a]";
    let parsed = parse(source);
    for input in ["", "a", "aaaaa", "aaaaaaaaaa", "aaaaaaaaaaa"] {
        run(input, source, &parsed);
    }
}

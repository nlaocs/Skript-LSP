use skript_parser::{
    MappedSource, MatchInput, MatchPattern, MatchSyntaxKind, NoopPatternMatchHooks,
    PatternCandidate, PatternCapture, PatternMatcherConfig, PatternOmissionKind,
    RejectTypeExpressions, TextRange, TypeCaptureState, match_pattern_candidates,
};
use syntax_pattern_parser::syntax::{self, PluralRules};

fn matched_captures(input: &str, pattern: &str) -> Vec<PatternCapture> {
    let rules = PluralRules::from_json(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../syntax-pattern-parser/tests/data/PluralRules-2.15.4.json"
    )))
    .unwrap();
    let parsed = syntax::parse(pattern, &rules).unwrap();
    let source = MappedSource::identity(input);
    let candidate = PatternCandidate {
        kind: MatchSyntaxKind::Effect,
        definition_id: "effect:omission".into(),
        registration_id: "effect:omission:0".into(),
        priority: 0,
        registration_order: 0,
        resolved_order: None,
        patterns: vec![MatchPattern {
            pattern_index: 0,
            source: pattern,
            parsed: &parsed,
        }],
    };
    match_pattern_candidates(
        MatchInput::from_source(&source, TextRange::new(0, input.len())).unwrap(),
        &[candidate],
        &mut RejectTypeExpressions,
        &mut NoopPatternMatchHooks,
        PatternMatcherConfig::default(),
    )
    .unwrap()
    .selected
    .unwrap()
    .matched
    .captures
}

#[test]
fn optional_omission_is_recorded_at_the_actual_insertion_anchor() {
    let pattern = "do [to %audiences%]";
    let captures = matched_captures("do", pattern);
    let PatternCapture::TypeExpression {
        state,
        omission,
        span,
        ..
    } = &captures[0]
    else {
        panic!()
    };
    assert_eq!(*state, TypeCaptureState::Omitted);
    let omission = omission.unwrap();
    assert_eq!(omission.kind, PatternOmissionKind::SkippedOption);
    assert_eq!(
        &pattern[omission.pattern_span.start..omission.pattern_span.end],
        "[to %audiences%]"
    );
    assert!(span.local_range.is_empty());
    assert_eq!(span.local_range.start, 2);
}

#[test]
fn unselected_choice_and_nullable_omission_are_distinct_from_initialized_defaults() {
    let captures = matched_captures("do done", "do (done|%audiences%)");
    let PatternCapture::TypeExpression {
        state, omission, ..
    } = &captures[0]
    else {
        panic!()
    };
    assert_eq!(*state, TypeCaptureState::Omitted);
    assert_eq!(
        omission.unwrap().kind,
        PatternOmissionKind::UnselectedChoice
    );
    let nullable = matched_captures("do", "do [to %-audiences%]");
    let PatternCapture::TypeExpression {
        state, omission, ..
    } = &nullable[0]
    else {
        panic!()
    };
    assert_eq!(*state, TypeCaptureState::Null);
    assert_eq!(omission.unwrap().kind, PatternOmissionKind::SkippedOption);
}

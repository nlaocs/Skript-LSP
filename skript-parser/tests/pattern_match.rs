use skript_parser::{
    CandidateMatches, MappedSource, MatchInput, MatchPattern, MatchSyntaxKind,
    NoopPatternMatchHooks, PatternCandidate, PatternCapture, PatternFailureReason,
    PatternHookControl, PatternHookEvent, PatternHookOutcome, PatternHookScope, PatternHookTiming,
    PatternMatchError, PatternMatchHooks, PatternMatchLimit, PatternMatcherConfig,
    RejectTypeExpressions, TextRange, TypeExpressionRequest, TypeExpressionResolution,
    TypeExpressionResolver, match_pattern_candidates,
};
use syntax_pattern_parser::syntax::{
    self, ParseResult, PatternElement, PluralRules, Span, Spanned,
};

fn parse(source: &str) -> ParseResult {
    let rules = PluralRules::from_json(include_str!(
        "../../syntax-pattern-parser/tests/data/PluralRules-2.15.4.json"
    ))
    .unwrap();
    syntax::parse(source, &rules).unwrap()
}

fn candidate<'a>(source: &'a str, parsed: &'a ParseResult, order: usize) -> PatternCandidate<'a> {
    PatternCandidate {
        kind: MatchSyntaxKind::Effect,
        definition_id: "effect:test".to_owned(),
        registration_id: "effect:test#0".to_owned(),
        priority: 0,
        registration_order: order,
        resolved_order: None,
        patterns: vec![MatchPattern { source, parsed }],
    }
}

fn match_one(
    input: &str,
    source: &str,
    parsed: &ParseResult,
) -> Result<CandidateMatches, PatternMatchError> {
    let mapped = MappedSource::identity(input);
    match_pattern_candidates(
        MatchInput::from_source(&mapped, TextRange::new(0, input.len()))?,
        &[candidate(source, parsed, 0)],
        &mut RejectTypeExpressions,
        &mut NoopPatternMatchHooks,
        PatternMatcherConfig::default(),
    )
}

#[test]
fn matches_structural_variants_and_skript_literal_rules() {
    let source = "active[ |-](group|model)[s]|";
    let pattern = parse(source);
    for input in ["active group", "active-models", ""] {
        let result = match_one(input, source, &pattern).unwrap();
        assert!(result.selected.is_some(), "{input:?}");
        assert!(result.failure.is_none());
    }
    assert!(
        match_one("active thing", source, &pattern)
            .unwrap()
            .selected
            .is_none()
    );

    let literal = parse("hello world");
    for input in ["hello world", "  HeLLo world  "] {
        assert!(
            match_one(input, "hello world", &literal)
                .unwrap()
                .selected
                .is_some()
        );
    }
    assert!(
        match_one("helloworld", "hello world", &literal)
            .unwrap()
            .selected
            .is_none()
    );
}

#[test]
fn captures_regex_groups_with_utf8_byte_spans() {
    let source = "name <(.)(.+)>";
    let pattern = parse(source);
    let input = "name 日本語";
    let selected = match_one(input, source, &pattern)
        .unwrap()
        .selected
        .unwrap();
    let PatternCapture::Regex {
        value,
        span,
        groups,
        ..
    } = &selected.matched.captures[0]
    else {
        panic!("regex capture expected");
    };
    assert_eq!(value, "日本語");
    assert_eq!(span.local_range, TextRange::new(5, input.len()));
    assert_eq!(groups[0].value.as_deref(), Some("日"));
    assert_eq!(
        groups[0].span.as_ref().unwrap().local_range,
        TextRange::new(5, 8)
    );
    assert_eq!(groups[1].value.as_deref(), Some("本語"));
}

#[test]
fn delegates_types_only_at_legal_skript_boundaries() {
    struct Resolver;
    impl TypeExpressionResolver for Resolver {
        fn resolve(
            &mut self,
            request: TypeExpressionRequest<'_>,
        ) -> Result<Vec<TypeExpressionResolution>, String> {
            assert_eq!(request.expression.alternatives[0].name, "string");
            assert!(!request.candidate_ends.contains(&9));
            Ok(vec![TypeExpressionResolution {
                range: TextRange::new(request.remaining.start, 13),
                alternative_index: Some(0),
                resolution_id: Some("expr:1".to_owned()),
            }])
        }
    }

    let source = "print %string% now";
    let pattern = parse(source);
    let input = "print \"a now\" now";
    let mapped = MappedSource::identity(input);
    let result = match_pattern_candidates(
        MatchInput::from_source(&mapped, TextRange::new(0, input.len())).unwrap(),
        &[candidate(source, &pattern, 0)],
        &mut Resolver,
        &mut NoopPatternMatchHooks,
        PatternMatcherConfig::default(),
    )
    .unwrap();
    let PatternCapture::TypeExpression {
        value,
        resolution_id,
        ..
    } = &result.selected.unwrap().matched.captures[0]
    else {
        panic!("type capture expected");
    };
    assert_eq!(value, "\"a now\"");
    assert_eq!(resolution_id.as_deref(), Some("expr:1"));
}

#[test]
fn records_explicit_implicit_tags_and_xor_marks() {
    let source = "(foo:first|bar:second) [1¦old] (2¦value)";
    let pattern = parse(source);
    let selected = match_one("second old value", source, &pattern)
        .unwrap()
        .selected
        .unwrap();
    assert_eq!(selected.matched.tags[0].value, "bar");
    assert!(!selected.matched.tags[0].implicit);
    assert_eq!(selected.matched.mark, 3);

    let source = ":(foo|bar)";
    let pattern = parse(source);
    let selected = match_one("bar", source, &pattern)
        .unwrap()
        .selected
        .unwrap();
    assert_eq!(selected.matched.tags[0].value, "bar");
    assert!(selected.matched.tags[0].implicit);
}

#[test]
fn ranks_candidates_and_preserves_pattern_index() {
    let other = parse("other");
    let first_match = parse("test");
    let second_match = parse("test");
    let third_match = parse("test");
    let first = PatternCandidate {
        priority: 1,
        patterns: vec![
            MatchPattern {
                source: "other",
                parsed: &other,
            },
            MatchPattern {
                source: "test",
                parsed: &first_match,
            },
        ],
        ..candidate("other", &other, 0)
    };
    let second = PatternCandidate {
        definition_id: "effect:second".to_owned(),
        registration_id: "effect:second#0".to_owned(),
        ..candidate("test", &second_match, 20)
    };
    let third = PatternCandidate {
        definition_id: "effect:third".to_owned(),
        registration_id: "effect:third#0".to_owned(),
        ..candidate("test", &third_match, 10)
    };
    let input = MappedSource::identity("test");
    let matches = match_pattern_candidates(
        MatchInput::from_source(&input, TextRange::new(0, 4)).unwrap(),
        &[first, second, third],
        &mut RejectTypeExpressions,
        &mut NoopPatternMatchHooks,
        PatternMatcherConfig::default(),
    )
    .unwrap();
    assert_eq!(
        matches.selected.as_ref().unwrap().definition_id,
        "effect:third"
    );
    assert_eq!(
        matches
            .alternatives
            .iter()
            .map(|value| value.definition_id.as_str())
            .collect::<Vec<_>>(),
        ["effect:second", "effect:test"]
    );
    assert_eq!(matches.alternatives[1].pattern_index, 1);
}

#[test]
fn reports_farthest_failure_invalid_regex_and_limits() {
    let pattern = parse("abc(def|xyz)");
    let failure = match_one("abcxyQ", "abc(def|xyz)", &pattern)
        .unwrap()
        .failure
        .unwrap();
    assert_eq!(failure.offset, 5);
    assert!(
        failure
            .reasons
            .iter()
            .any(|reason| matches!(reason, PatternFailureReason::Literal { .. }))
    );

    let malformed = ParseResult {
        elements: vec![Spanned::new(
            PatternElement::Regex("(".to_owned()),
            Span::new(0, 3),
        )],
        warnings: Vec::new(),
    };
    assert!(matches!(
        match_one("x", "<(>", &malformed),
        Err(PatternMatchError::InvalidRegex { .. })
    ));

    let branching = parse("(a|b|c)");
    let input = MappedSource::identity("z");
    let error = match_pattern_candidates(
        MatchInput::from_source(&input, TextRange::new(0, 1)).unwrap(),
        &[candidate("(a|b|c)", &branching, 0)],
        &mut RejectTypeExpressions,
        &mut NoopPatternMatchHooks,
        PatternMatcherConfig {
            max_backtracks: 1,
            ..PatternMatcherConfig::default()
        },
    )
    .unwrap_err();
    assert!(matches!(
        error,
        PatternMatchError::LimitExceeded {
            kind: PatternMatchLimit::Backtracks,
            ..
        }
    ));
}

#[test]
fn hooks_observe_nested_paths_and_override_elements() {
    #[derive(Default)]
    struct Hooks {
        paths: Vec<Vec<skript_parser::PatternPathSegment>>,
    }
    impl PatternMatchHooks for Hooks {
        fn dispatch(&mut self, event: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
            if event.scope == PatternHookScope::Element && event.timing == PatternHookTiming::Before
            {
                self.paths.push(event.element_path.to_vec());
                if event.pattern_span == Some(Span::new(1, 8)) {
                    return Ok(PatternHookControl::Match(TextRange::new(0, 7)));
                }
            }
            Ok(PatternHookControl::Continue)
        }
    }

    let source = "(ignored)";
    let pattern = parse(source);
    let input = MappedSource::identity("handled");
    let mut hooks = Hooks::default();
    let result = match_pattern_candidates(
        MatchInput::from_source(&input, TextRange::new(0, 7)).unwrap(),
        &[candidate(source, &pattern, 0)],
        &mut RejectTypeExpressions,
        &mut hooks,
        PatternMatcherConfig::default(),
    )
    .unwrap();
    assert!(result.selected.is_some());
    assert!(hooks.paths.iter().any(|path| path.len() >= 2));
}

#[test]
fn generated_input_keeps_local_and_mapped_spans_separate() {
    let pattern = parse("<.+>");
    let call_source = MappedSource::identity("macro()");
    let call_site = call_source.map_range(TextRange::new(0, 7)).unwrap();
    let result = match_pattern_candidates(
        MatchInput::generated("generated", call_site.clone()),
        &[candidate("<.+>", &pattern, 0)],
        &mut RejectTypeExpressions,
        &mut NoopPatternMatchHooks,
        PatternMatcherConfig::default(),
    )
    .unwrap();
    let span = result.selected.unwrap().matched.span;
    assert_eq!(span.local_range, TextRange::new(0, 9));
    assert_eq!(span.mapped, call_site);
}

#[test]
fn broad_scope_hooks_can_override_complete_candidates() {
    struct Hooks {
        scope: PatternHookScope,
    }
    impl PatternMatchHooks for Hooks {
        fn dispatch(&mut self, event: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
            if event.scope == self.scope && event.timing == PatternHookTiming::Before {
                return Ok(PatternHookControl::Match(event.input_range));
            }
            Ok(PatternHookControl::Continue)
        }
    }

    let pattern = parse("never");
    for scope in [PatternHookScope::Definition, PatternHookScope::Registration] {
        let input = MappedSource::identity("handled");
        let result = match_pattern_candidates(
            MatchInput::from_source(&input, TextRange::new(0, 7)).unwrap(),
            &[candidate("never", &pattern, 0)],
            &mut RejectTypeExpressions,
            &mut Hooks { scope },
            PatternMatcherConfig::default(),
        )
        .unwrap();
        assert_eq!(
            result.selected.unwrap().matched.span.local_range,
            TextRange::new(0, 7)
        );
    }
}

#[test]
fn rejected_registration_still_closes_the_definition_scope() {
    #[derive(Default)]
    struct Hooks {
        definition_after: Vec<PatternHookOutcome>,
    }
    impl PatternMatchHooks for Hooks {
        fn dispatch(&mut self, event: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
            if event.scope == PatternHookScope::Registration
                && event.timing == PatternHookTiming::After
                && matches!(event.outcome, PatternHookOutcome::Matched { .. })
            {
                return Ok(PatternHookControl::Fail("rejected registration".to_owned()));
            }
            if event.scope == PatternHookScope::Definition
                && event.timing == PatternHookTiming::After
            {
                self.definition_after.push(event.outcome);
            }
            Ok(PatternHookControl::Continue)
        }
    }

    let pattern = parse("handled");
    let input = MappedSource::identity("handled");
    let mut hooks = Hooks::default();
    let result = match_pattern_candidates(
        MatchInput::from_source(&input, TextRange::new(0, 7)).unwrap(),
        &[candidate("handled", &pattern, 0)],
        &mut RejectTypeExpressions,
        &mut hooks,
        PatternMatcherConfig::default(),
    )
    .unwrap();

    assert!(result.selected.is_none());
    assert!(matches!(
        hooks.definition_after.as_slice(),
        [PatternHookOutcome::Failed { .. }]
    ));
}

#[test]
fn failed_after_hooks_can_rescue_candidates_at_every_scope() {
    struct Hooks {
        scope: PatternHookScope,
    }
    impl PatternMatchHooks for Hooks {
        fn dispatch(&mut self, event: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
            if event.scope == self.scope
                && event.timing == PatternHookTiming::After
                && matches!(event.outcome, PatternHookOutcome::Failed { .. })
            {
                return Ok(PatternHookControl::Match(TextRange::new(0, 7)));
            }
            Ok(PatternHookControl::Continue)
        }
    }

    let pattern = parse("never");
    for scope in [
        PatternHookScope::Definition,
        PatternHookScope::Registration,
        PatternHookScope::Pattern,
        PatternHookScope::Element,
    ] {
        let input = MappedSource::identity("handled");
        let result = match_pattern_candidates(
            MatchInput::from_source(&input, TextRange::new(0, 7)).unwrap(),
            &[candidate("never", &pattern, 0)],
            &mut RejectTypeExpressions,
            &mut Hooks { scope },
            PatternMatcherConfig::default(),
        )
        .unwrap();
        assert!(
            result.selected.is_some(),
            "{scope:?} after-hook should rescue the candidate"
        );
    }
}

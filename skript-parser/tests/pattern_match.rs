use skript_parser::{
    CandidateMatches, FailureTrace, MappedSource, MatchInput, MatchPattern, MatchSyntaxKind,
    NoopPatternMatchHooks, PatternCandidate, PatternCapture, PatternFailureReason,
    PatternHookControl, PatternHookEvent, PatternHookOutcome, PatternHookScope, PatternHookTiming,
    PatternMatch, PatternMatchEnvironment, PatternMatchError, PatternMatchHooks, PatternMatchLimit,
    PatternMatcherConfig, RejectTypeExpressions, TextRange, TypeCaptureState,
    TypeExpressionOutcome, TypeExpressionRequest, TypeExpressionResolution, TypeExpressionResolver,
    match_pattern_candidates, match_pattern_candidates_with_environment,
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
        patterns: vec![MatchPattern {
            pattern_index: 0,
            source,
            parsed,
        }],
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
fn mixed_syntax_kinds_preserve_parser_phase_then_registry_order() {
    let source = "same pattern";
    let parsed = parse(source);
    let section_late = PatternCandidate {
        kind: MatchSyntaxKind::Section,
        definition_id: "section:test:late".to_owned(),
        registration_id: "section:test:late#0".to_owned(),
        resolved_order: Some(5),
        ..candidate(source, &parsed, 50)
    };
    let effect_first_in_registry = PatternCandidate {
        definition_id: "effect:test:first".to_owned(),
        registration_id: "effect:test:first#0".to_owned(),
        resolved_order: Some(0),
        ..candidate(source, &parsed, 0)
    };
    let section_early = PatternCandidate {
        kind: MatchSyntaxKind::Section,
        definition_id: "section:test:early".to_owned(),
        registration_id: "section:test:early#0".to_owned(),
        resolved_order: Some(1),
        ..candidate(source, &parsed, 10)
    };
    let mapped = MappedSource::identity(source);
    let result = match_pattern_candidates(
        MatchInput::from_source(&mapped, TextRange::new(0, source.len())).unwrap(),
        &[section_late, effect_first_in_registry, section_early],
        &mut RejectTypeExpressions,
        &mut NoopPatternMatchHooks,
        PatternMatcherConfig::default(),
    )
    .unwrap();

    let selected = result.selected.expect("one candidate must be selected");
    assert_eq!(selected.kind, MatchSyntaxKind::Section);
    assert_eq!(selected.registration_id, "section:test:early#0");
    assert_eq!(
        result
            .alternatives
            .iter()
            .map(|candidate| candidate.registration_id.as_str())
            .collect::<Vec<_>>(),
        vec!["section:test:late#0", "effect:test:first#0"]
    );
}

#[derive(Debug, Default)]
struct AcceptTypeExpressions;

impl TypeExpressionResolver for AcceptTypeExpressions {
    fn resolve(
        &mut self,
        request: TypeExpressionRequest<'_>,
    ) -> Result<TypeExpressionOutcome, String> {
        Ok(request
            .candidate_ends
            .iter()
            .copied()
            .map(|end| TypeExpressionResolution {
                range: TextRange::new(request.remaining.start, end),
                alternative_index: Some(0),
                resolution_id: None,
            })
            .collect::<Vec<_>>()
            .into())
    }
}

#[test]
fn matches_structural_variants_and_skript_literal_rules() {
    let source = "active[ |-](group|model)[s]|";
    let pattern = parse(source);
    for input in ["active group", "active-models", ""] {
        let result = match_one(input, source, &pattern).unwrap();
        assert!(result.selected.is_some(), "{input:?}");
        assert!(result.primary_failure().is_none());
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
fn matches_legacy_event_structure_pattern() {
    let source = "[on] <.+>";
    let pattern = parse(source);
    let selected = match_one("on join", source, &pattern).unwrap().selected;
    assert!(selected.is_some());
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
fn capture_indexes_follow_pattern_slots_when_an_optional_capture_is_omitted() {
    let source = "[<foo>]<bar>";
    let pattern = parse(source);
    let selected = match_one("bar", source, &pattern)
        .unwrap()
        .selected
        .expect("the second regex must match after omitting the first");

    assert_eq!(selected.matched.captures.len(), 1);
    assert_eq!(selected.matched.captures[0].capture_index(), 1);
}

#[test]
fn capture_indexes_include_unselected_choice_branches() {
    let source = "(<a>|<b>)<c>";
    let pattern = parse(source);
    let selected = match_one("bc", source, &pattern)
        .unwrap()
        .selected
        .expect("the second branch and trailing regex must match");

    assert_eq!(selected.matched.captures.len(), 2);
    assert_eq!(selected.matched.captures[0].capture_index(), 1);
    assert_eq!(selected.matched.captures[1].capture_index(), 2);
}

#[test]
fn omitted_typed_captures_distinguish_defaults_from_nullable_slots() {
    for (source, expected_state) in [
        ("do [%number%]", TypeCaptureState::Omitted),
        ("do [%-number%]", TypeCaptureState::Null),
    ] {
        let selected = match_one("do", source, &parse(source))
            .unwrap()
            .selected
            .unwrap();
        assert_eq!(selected.matched.captures.len(), 1);
        let PatternCapture::TypeExpression {
            state,
            value,
            span,
            expression,
            ..
        } = &selected.matched.captures[0]
        else {
            panic!("typed capture expected");
        };
        assert_eq!(*state, expected_state);
        assert_eq!(
            expression.nullable,
            expected_state == TypeCaptureState::Null
        );
        assert!(value.is_empty());
        assert_eq!(span.local_range, TextRange::empty(2));
        assert_eq!(span.mapped.virtual_range, TextRange::empty(2));
        assert_eq!(
            span.mapped.origins[0].kind,
            skript_parser::OriginKind::Exact
        );
        assert_eq!(span.mapped.origins[0].original_range, TextRange::empty(2));
    }
}

#[test]
fn explicit_and_zero_length_nullable_resolutions_have_distinct_states() {
    for (input, source, expected_state) in [
        ("do 1", "do %number%", TypeCaptureState::Explicit),
        ("do", "do %-number%", TypeCaptureState::Null),
    ] {
        let mapped = MappedSource::identity(input);
        let parsed = parse(source);
        let result = match_pattern_candidates(
            MatchInput::from_source(&mapped, TextRange::new(0, input.len())).unwrap(),
            &[candidate(source, &parsed, 0)],
            &mut AcceptTypeExpressions,
            &mut NoopPatternMatchHooks,
            PatternMatcherConfig::default(),
        )
        .unwrap();
        let selected = result.selected.unwrap();
        let PatternCapture::TypeExpression { state, value, .. } = &selected.matched.captures[0]
        else {
            panic!("typed capture expected");
        };
        assert_eq!(*state, expected_state);
        assert_eq!(value.is_empty(), expected_state == TypeCaptureState::Null);
    }
}

#[test]
fn nested_unselected_branches_keep_every_typed_slot_at_the_insertion_position() {
    let source = "do [[%number%]] (done|%string%)";
    let selected = match_one("do done", source, &parse(source))
        .unwrap()
        .selected
        .unwrap();
    assert_eq!(selected.matched.captures.len(), 2);
    for (index, capture) in selected.matched.captures.iter().enumerate() {
        let PatternCapture::TypeExpression { state, span, .. } = capture else {
            panic!("typed capture expected");
        };
        assert_eq!(capture.capture_index(), index);
        assert_eq!(*state, TypeCaptureState::Omitted);
        assert_eq!(span.local_range, TextRange::empty(3));
    }
}

#[test]
fn typed_omissions_share_stable_slot_indices_with_regex_captures() {
    let source = "[<foo>][%number%](<bar>|%string%)";
    let selected = match_one("bar", source, &parse(source))
        .unwrap()
        .selected
        .unwrap();
    assert_eq!(
        selected
            .matched
            .captures
            .iter()
            .map(PatternCapture::capture_index)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert!(matches!(
        selected.matched.captures[0],
        PatternCapture::TypeExpression {
            state: TypeCaptureState::Omitted,
            ..
        }
    ));
    assert!(matches!(
        selected.matched.captures[1],
        PatternCapture::Regex { .. }
    ));
    assert!(matches!(
        selected.matched.captures[2],
        PatternCapture::TypeExpression {
            state: TypeCaptureState::Omitted,
            ..
        }
    ));
}

#[derive(Default)]
struct CompletingEnvironment {
    calls: usize,
    reject_calls: usize,
    state: usize,
    checkpoints: Vec<usize>,
    completed_states_seen_after: Vec<usize>,
    states_seen_before: Vec<usize>,
    scopes: Vec<(PatternHookScope, usize)>,
    match_states: Vec<(usize, usize)>,
    explicit_state: usize,
    override_pattern: bool,
    override_failed_registration: Option<TextRange>,
    reject_registration_before: bool,
    reject_after: Option<PatternHookScope>,
    error_after: Option<PatternHookScope>,
    report_exploratory_type_failure: bool,
}

impl PatternMatchEnvironment for CompletingEnvironment {
    fn begin_pattern_match(&mut self) -> Result<(), String> {
        self.match_states.push((self.state, self.scopes.len()));
        Ok(())
    }

    fn finish_pattern_match(&mut self, accepted: bool) -> Result<(), String> {
        let (state, scope_depth) = self.match_states.pop().unwrap();
        if !accepted {
            self.state = state;
            self.scopes.truncate(scope_depth);
        }
        assert_eq!(self.scopes.len(), scope_depth);
        Ok(())
    }

    fn select_type_resolution(
        &mut self,
        _resolution: &TypeExpressionResolution,
    ) -> Result<(), String> {
        self.state = self.explicit_state;
        Ok(())
    }

    fn checkpoint_pattern_branch(&mut self) -> Result<Option<u64>, String> {
        let checkpoint = self.checkpoints.len() as u64;
        self.checkpoints.push(self.state);
        Ok(Some(checkpoint))
    }

    fn restore_pattern_branch(&mut self, checkpoint: u64) -> Result<(), String> {
        self.state = self.checkpoints[checkpoint as usize];
        Ok(())
    }

    fn resolve_type(
        &mut self,
        request: TypeExpressionRequest<'_>,
    ) -> Result<TypeExpressionOutcome, String> {
        let failure = if self.report_exploratory_type_failure {
            let source = MappedSource::identity(request.input);
            let range = TextRange::new(request.remaining.start, request.remaining.end);
            Some(FailureTrace::leaf(skript_parser::PatternFailure {
                span: MatchInput::from_source(&source, TextRange::new(0, request.input.len()))
                    .unwrap()
                    .map_range(range)
                    .unwrap(),
                reasons: vec![PatternFailureReason::TypeParserUnresolved {
                    definition_id: "type:test".to_owned(),
                    registration_id: "type:test#0".to_owned(),
                    parser_class: None,
                    reason: "an explored type has no provider".to_owned(),
                    required_provider: None,
                }],
            }))
        } else {
            None
        };
        let mut outcome = AcceptTypeExpressions.resolve(request)?;
        outcome.failure = failure;
        Ok(outcome)
    }

    fn complete_typed_captures(
        &mut self,
        _candidate: &PatternCandidate<'_>,
        _pattern: &MatchPattern<'_>,
        matched: &mut PatternMatch,
    ) -> Result<Option<FailureTrace>, String> {
        assert_eq!(
            self.state, self.explicit_state,
            "rejected completion state must have rolled back"
        );
        self.calls += 1;
        self.state = self.calls;
        let PatternCapture::TypeExpression {
            capture_index,
            expression,
            state,
            span,
            resolution_id,
            ..
        } = matched
            .captures
            .iter_mut()
            .find(|capture| {
                matches!(
                    capture,
                    PatternCapture::TypeExpression {
                        state: TypeCaptureState::Omitted,
                        ..
                    }
                )
            })
            .expect("an omitted typed capture is required")
        else {
            panic!("typed capture expected");
        };
        if self.calls <= self.reject_calls {
            return Ok(Some(FailureTrace::leaf(skript_parser::PatternFailure {
                span: span.clone(),
                reasons: vec![PatternFailureReason::DefaultExpression {
                    capture_index: *capture_index,
                    expression: expression.clone(),
                    kind: skript_parser::DefaultExpressionFailureKind::Rejected,
                    reason: "test provider rejected the omitted argument".to_owned(),
                }],
            })));
        }
        *state = TypeCaptureState::Default;
        *resolution_id = Some("test:default".to_owned());
        Ok(None)
    }

    fn dispatch_hook(&mut self, event: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
        let mut scope_state = None;
        if matches!(
            event.scope,
            PatternHookScope::Definition
                | PatternHookScope::Registration
                | PatternHookScope::Pattern
        ) {
            match event.timing {
                PatternHookTiming::Before => self.scopes.push((event.scope, self.state)),
                PatternHookTiming::After => {
                    let (scope, state) = self.scopes.pop().unwrap();
                    assert_eq!(scope, event.scope);
                    scope_state = Some(state);
                }
            }
        }
        if event.scope == PatternHookScope::Pattern {
            if event.timing == PatternHookTiming::Before {
                self.states_seen_before.push(self.state);
                if self.override_pattern {
                    return Ok(PatternHookControl::Match(event.input_range));
                }
            }
            if event.timing == PatternHookTiming::After
                && matches!(event.outcome, PatternHookOutcome::Matched { .. })
            {
                self.completed_states_seen_after.push(self.state);
            }
        }
        let mut control = PatternHookControl::Continue;
        if event.scope == PatternHookScope::Registration && event.registration_id == "effect:test#0"
        {
            if event.timing == PatternHookTiming::Before && self.reject_registration_before {
                control = PatternHookControl::Fail("test registration rejected".to_owned());
            } else if event.timing == PatternHookTiming::After
                && matches!(event.outcome, PatternHookOutcome::Failed { .. })
                && let Some(range) = self.override_failed_registration
            {
                control = PatternHookControl::Match(range);
            }
        }
        if event.timing == PatternHookTiming::After
            && matches!(event.outcome, PatternHookOutcome::Matched { .. })
            && self.calls == 1
        {
            if self.error_after == Some(event.scope) {
                self.state = usize::MAX;
                return Err("test after hook failed".to_owned());
            }
            if self.reject_after == Some(event.scope) {
                self.state = usize::MAX;
                control = PatternHookControl::Fail("test after hook rejected".to_owned());
            }
        }
        if let Some(state) = scope_state
            && ((matches!(event.outcome, PatternHookOutcome::Failed { .. })
                && matches!(control, PatternHookControl::Continue))
                || matches!(control, PatternHookControl::Fail(_)))
        {
            self.state = state;
        }
        Ok(control)
    }
}

#[test]
fn completion_rejection_rolls_back_and_tries_the_next_complete_branch() {
    let source = "do [[%number%]]";
    let parsed = parse(source);
    let mapped = MappedSource::identity("do");
    let mut environment = CompletingEnvironment {
        reject_calls: 1,
        ..Default::default()
    };
    let result = match_pattern_candidates_with_environment(
        MatchInput::from_source(&mapped, TextRange::new(0, 2)).unwrap(),
        &[candidate(source, &parsed, 0)],
        &mut environment,
        PatternMatcherConfig::default(),
    )
    .unwrap();
    let selected = result
        .selected
        .expect("the second complete branch must be attempted");
    assert_eq!(environment.calls, 2);
    assert_eq!(environment.completed_states_seen_after, vec![2]);
    assert!(matches!(
        selected.matched.captures[0],
        PatternCapture::TypeExpression {
            state: TypeCaptureState::Default,
            ..
        }
    ));
}

#[test]
fn after_hook_rejection_restores_defaults_before_the_next_patterns_before_hook() {
    for override_pattern in [false, true] {
        let source = "do [[%number%]]";
        let parsed = parse(source);
        let mapped = MappedSource::identity("do");
        let mut candidate = candidate(source, &parsed, 0);
        candidate.patterns.push(MatchPattern {
            pattern_index: 1,
            source,
            parsed: &parsed,
        });
        let mut environment = CompletingEnvironment {
            override_pattern,
            reject_after: Some(PatternHookScope::Pattern),
            ..Default::default()
        };
        let result = match_pattern_candidates_with_environment(
            MatchInput::from_source(&mapped, TextRange::new(0, 2)).unwrap(),
            &[candidate],
            &mut environment,
            PatternMatcherConfig::default(),
        )
        .unwrap();
        assert_eq!(result.selected.unwrap().pattern_index, 1);
        assert_eq!(environment.states_seen_before, vec![0, 0]);
        assert_eq!(environment.completed_states_seen_after, vec![1, 2]);
        assert_eq!(
            environment.state, 2,
            "the selected default's state must remain active"
        );
    }
}

#[test]
fn failed_registration_override_closes_definition_before_the_next_candidate() {
    for reject_registration_before in [false, true] {
        let source = "do [%number%]";
        let parsed = parse(source);
        let fallback = parse("do");
        let mapped = MappedSource::identity("do");
        let mut fallback_candidate = candidate("do", &fallback, 1);
        fallback_candidate.registration_id = "effect:fallback#0".to_owned();
        let mut environment = CompletingEnvironment {
            reject_calls: usize::MAX,
            override_failed_registration: Some(TextRange::new(0, 2)),
            reject_registration_before,
            ..Default::default()
        };
        let result = match_pattern_candidates_with_environment(
            MatchInput::from_source(&mapped, TextRange::new(0, 2)).unwrap(),
            &[candidate(source, &parsed, 0), fallback_candidate],
            &mut environment,
            PatternMatcherConfig::default(),
        )
        .unwrap();
        assert_eq!(
            result.selected.unwrap().registration_id,
            "effect:fallback#0"
        );
        assert!(environment.calls > 0);
        assert!(environment.scopes.is_empty());
        assert_eq!(environment.state, 0);
    }
}

#[test]
fn rejected_scope_does_not_restore_explicit_child_state_from_before_completion() {
    let source = "do %string%[ to %number%]";
    let parsed = parse(source);
    let mapped = MappedSource::identity("do value");
    let mut candidate = candidate(source, &parsed, 0);
    candidate.patterns.push(MatchPattern {
        pattern_index: 1,
        source,
        parsed: &parsed,
    });
    let mut environment = CompletingEnvironment {
        explicit_state: 7,
        reject_after: Some(PatternHookScope::Pattern),
        ..Default::default()
    };
    let result = match_pattern_candidates_with_environment(
        MatchInput::from_source(&mapped, TextRange::new(0, 8)).unwrap(),
        &[candidate],
        &mut environment,
        PatternMatcherConfig::default(),
    )
    .unwrap();
    assert_eq!(result.selected.unwrap().pattern_index, 1);
    assert_eq!(environment.states_seen_before, vec![0, 0]);
    assert_eq!(environment.completed_states_seen_after, vec![1, 2]);
    assert_eq!(environment.state, 2);
}

#[test]
fn rejected_completion_balances_pattern_hooks_even_with_a_before_override() {
    for override_pattern in [false, true] {
        let source = "do [[%number%]]";
        let parsed = parse(source);
        let mapped = MappedSource::identity("do");
        let mut environment = CompletingEnvironment {
            override_pattern,
            reject_calls: usize::MAX,
            ..Default::default()
        };
        let result = match_pattern_candidates_with_environment(
            MatchInput::from_source(&mapped, TextRange::new(0, 2)).unwrap(),
            &[candidate(source, &parsed, 0)],
            &mut environment,
            PatternMatcherConfig::default(),
        )
        .unwrap();
        assert!(result.selected.is_none());
        assert!(environment.scopes.is_empty());
        assert_eq!(environment.state, 0);
        assert!(matches!(
            result
                .failures
                .primary()
                .unwrap()
                .trace
                .root_cause()
                .failure
                .reasons[0],
            PatternFailureReason::DefaultExpression { .. }
        ));
    }
}

#[test]
fn final_after_hook_rejection_or_error_restores_completion_state_at_every_scope() {
    for scope in [
        PatternHookScope::Pattern,
        PatternHookScope::Registration,
        PatternHookScope::Definition,
    ] {
        for error in [false, true] {
            let source = "do [%number%]";
            let parsed = parse(source);
            let mapped = MappedSource::identity("do");
            let mut environment = CompletingEnvironment {
                reject_after: (!error).then_some(scope),
                error_after: error.then_some(scope),
                ..Default::default()
            };
            let result = match_pattern_candidates_with_environment(
                MatchInput::from_source(&mapped, TextRange::new(0, 2)).unwrap(),
                &[candidate(source, &parsed, 0)],
                &mut environment,
                PatternMatcherConfig::default(),
            );
            if error {
                assert!(
                    matches!(result, Err(PatternMatchError::Hook { message }) if message == "test after hook failed")
                );
            } else {
                assert!(result.unwrap().selected.is_none());
            }
            assert_eq!(
                environment.state, 0,
                "completion state leaked after {scope:?}, error={error}"
            );
        }
    }
}

#[test]
fn completion_rejection_keeps_the_recognized_candidate_and_argument_reason() {
    let source = "do [%number%]";
    let parsed = parse(source);
    let mapped = MappedSource::identity("do");
    let mut environment = CompletingEnvironment {
        reject_calls: usize::MAX,
        ..Default::default()
    };
    let result = match_pattern_candidates_with_environment(
        MatchInput::from_source(&mapped, TextRange::new(0, 2)).unwrap(),
        &[candidate(source, &parsed, 0)],
        &mut environment,
        PatternMatcherConfig::default(),
    )
    .unwrap();
    assert!(result.selected.is_none());
    assert_eq!(environment.state, 0);
    let failure = result.failures.primary().unwrap();
    assert_eq!(failure.registration_id, "effect:test#0");
    assert_eq!(failure.pattern.as_deref(), Some(source));
    assert_eq!(
        failure.trace.root_cause().failure.span.local_range,
        TextRange::empty(2)
    );
    assert!(matches!(&failure.trace.root_cause().failure.reasons[0],
        PatternFailureReason::DefaultExpression { reason, .. }
            if reason == "test provider rejected the omitted argument"
    ));
}

#[test]
fn completed_default_failure_outweighs_exploratory_type_failure_with_a_concrete_span() {
    let source = "do %string%[ to %number%]";
    let parsed = parse(source);
    let later_source = "do %string% missing";
    let later_parsed = parse(later_source);
    let mut candidate = candidate(source, &parsed, 0);
    candidate.patterns.push(MatchPattern {
        pattern_index: 1,
        source: later_source,
        parsed: &later_parsed,
    });
    let mapped = MappedSource::identity("do value");
    let mut environment = CompletingEnvironment {
        reject_calls: usize::MAX,
        report_exploratory_type_failure: true,
        ..Default::default()
    };
    let result = match_pattern_candidates_with_environment(
        MatchInput::from_source(&mapped, TextRange::new(0, 8)).unwrap(),
        &[candidate],
        &mut environment,
        PatternMatcherConfig::default(),
    )
    .unwrap();
    assert!(result.selected.is_none());
    let failure = result.failures.primary().unwrap();
    assert_eq!(failure.pattern.as_deref(), Some(source));
    assert_eq!(
        failure.trace.root_cause().failure.span.local_range,
        TextRange::empty(8)
    );
    assert!(matches!(
        failure.trace.root_cause().failure.reasons[0],
        PatternFailureReason::DefaultExpression {
            capture_index: 1,
            ..
        }
    ));
    assert!(matches!(
        result
            .failures
            .fallback
            .unwrap()
            .root_cause()
            .failure
            .reasons[0],
        PatternFailureReason::DefaultExpression {
            capture_index: 1,
            ..
        }
    ));
}

#[test]
fn synthetic_pattern_matches_also_complete_typed_slots_before_after_hooks() {
    let source = "other %number%";
    let parsed = parse(source);
    let mapped = MappedSource::identity("do");
    let mut environment = CompletingEnvironment {
        override_pattern: true,
        ..Default::default()
    };
    let result = match_pattern_candidates_with_environment(
        MatchInput::from_source(&mapped, TextRange::new(0, 2)).unwrap(),
        &[candidate(source, &parsed, 0)],
        &mut environment,
        PatternMatcherConfig::default(),
    )
    .unwrap();
    let selected = result.selected.unwrap();
    assert_eq!(environment.calls, 1);
    assert_eq!(environment.completed_states_seen_after, vec![1]);
    let PatternCapture::TypeExpression {
        span, state, value, ..
    } = &selected.matched.captures[0]
    else {
        panic!("typed capture expected");
    };
    assert_eq!(*state, TypeCaptureState::Default);
    assert!(value.is_empty());
    assert_eq!(span.local_range, TextRange::empty(2));
}

// These boundary-focused tests use stub resolvers that accept the requested range without
// validating it as a real Skript expression. They isolate which candidate ends the matcher
// offers to the expression parser; expression syntax is covered by the expression tests.
#[test]
fn delegates_types_only_at_legal_skript_boundaries() {
    struct Resolver;
    impl TypeExpressionResolver for Resolver {
        fn resolve(
            &mut self,
            request: TypeExpressionRequest<'_>,
        ) -> Result<TypeExpressionOutcome, String> {
            assert_eq!(request.expression.alternatives[0].name, "string");
            assert_eq!(request.candidate_ends, &[13]);
            Ok(vec![TypeExpressionResolution {
                range: TextRange::new(request.remaining.start, 13),
                alternative_index: Some(0),
                resolution_id: Some("expr:1".to_owned()),
            }]
            .into())
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
fn successful_type_resolution_keeps_its_branch_state_for_after_hooks() {
    #[derive(Default)]
    struct Environment {
        state: u8,
        checkpoints: Vec<u8>,
        observed_after_state: Option<u8>,
    }

    impl PatternMatchEnvironment for Environment {
        fn checkpoint_pattern_branch(&mut self) -> Result<Option<u64>, String> {
            let checkpoint = self.checkpoints.len() as u64;
            self.checkpoints.push(self.state);
            Ok(Some(checkpoint))
        }

        fn restore_pattern_branch(&mut self, checkpoint: u64) -> Result<(), String> {
            self.state = *self
                .checkpoints
                .get(checkpoint as usize)
                .ok_or_else(|| "unknown checkpoint".to_owned())?;
            Ok(())
        }

        fn resolve_type(
            &mut self,
            request: TypeExpressionRequest<'_>,
        ) -> Result<TypeExpressionOutcome, String> {
            self.state = 1;
            Ok(vec![TypeExpressionResolution {
                range: TextRange::new(request.remaining.start, request.candidate_ends[0]),
                alternative_index: Some(0),
                resolution_id: Some("expression:stateful".to_owned()),
            }]
            .into())
        }

        fn dispatch_hook(
            &mut self,
            event: PatternHookEvent<'_>,
        ) -> Result<PatternHookControl, String> {
            if event.scope == PatternHookScope::Element
                && event.timing == PatternHookTiming::After
                && matches!(event.outcome, PatternHookOutcome::Matched { .. })
            {
                self.observed_after_state = Some(self.state);
            }
            Ok(PatternHookControl::Continue)
        }
    }

    let source = "%string%";
    let pattern = parse(source);
    let input = "value";
    let mapped = MappedSource::identity(input);
    let mut environment = Environment::default();
    let result = match_pattern_candidates_with_environment(
        MatchInput::from_source(&mapped, TextRange::new(0, input.len())).unwrap(),
        &[candidate(source, &pattern, 0)],
        &mut environment,
        PatternMatcherConfig::default(),
    )
    .unwrap();

    assert!(result.selected.is_some());
    assert_eq!(environment.observed_after_state, Some(1));
}

#[test]
fn carries_type_boundary_lookahead_out_of_nested_groups() {
    struct Resolver;
    impl TypeExpressionResolver for Resolver {
        fn resolve(
            &mut self,
            request: TypeExpressionRequest<'_>,
        ) -> Result<TypeExpressionOutcome, String> {
            assert_eq!(request.candidate_ends, &[13]);
            Ok(vec![TypeExpressionResolution {
                range: TextRange::new(request.remaining.start, 13),
                alternative_index: Some(0),
                resolution_id: None,
            }]
            .into())
        }
    }

    let source = "print (%string%) now";
    let pattern = parse(source);
    let input = "print one two now";
    let mapped = MappedSource::identity(input);
    let result = match_pattern_candidates(
        MatchInput::from_source(&mapped, TextRange::new(0, input.len())).unwrap(),
        &[candidate(source, &pattern, 0)],
        &mut Resolver,
        &mut NoopPatternMatchHooks,
        PatternMatcherConfig::default(),
    )
    .unwrap();

    assert!(result.selected.is_some());
}

#[test]
fn keeps_both_optional_tail_paths_for_type_boundaries() {
    struct Resolver;
    impl TypeExpressionResolver for Resolver {
        fn resolve(
            &mut self,
            request: TypeExpressionRequest<'_>,
        ) -> Result<TypeExpressionOutcome, String> {
            assert_eq!(request.candidate_ends, &[9, 13]);
            Ok(vec![TypeExpressionResolution {
                range: TextRange::new(request.remaining.start, 9),
                alternative_index: Some(0),
                resolution_id: None,
            }]
            .into())
        }
    }

    let source = "print %string%[ now]";
    let pattern = parse(source);
    let input = "print one now";
    let mapped = MappedSource::identity(input);
    let result = match_pattern_candidates(
        MatchInput::from_source(&mapped, TextRange::new(0, input.len())).unwrap(),
        &[candidate(source, &pattern, 0)],
        &mut Resolver,
        &mut NoopPatternMatchHooks,
        PatternMatcherConfig::default(),
    )
    .unwrap();

    assert!(result.selected.is_some());
}

#[test]
fn keeps_all_boundaries_before_a_dynamic_type_tail() {
    struct Resolver;
    impl TypeExpressionResolver for Resolver {
        fn resolve(
            &mut self,
            request: TypeExpressionRequest<'_>,
        ) -> Result<TypeExpressionOutcome, String> {
            let end = match request.remaining.start {
                6 => {
                    assert_eq!(request.candidate_ends, &[7, 8]);
                    7
                }
                7 => {
                    assert_eq!(request.candidate_ends, &[8]);
                    8
                }
                start => panic!("unexpected resolver start: {start}"),
            };
            Ok(vec![TypeExpressionResolution {
                range: TextRange::new(request.remaining.start, end),
                alternative_index: Some(0),
                resolution_id: None,
            }]
            .into())
        }
    }

    // Skript permits adjacent type elements, so the matcher cannot infer where the first one
    // ends. The stub above deliberately treats the bare `a` and `b` as string expressions;
    // this tests boundary exploration, not whether real Skript accepts those string literals.
    let source = "print %string%%string%";
    let pattern = parse(source);
    let input = "print ab";
    let mapped = MappedSource::identity(input);
    let result = match_pattern_candidates(
        MatchInput::from_source(&mapped, TextRange::new(0, input.len())).unwrap(),
        &[candidate(source, &pattern, 0)],
        &mut Resolver,
        &mut NoopPatternMatchHooks,
        PatternMatcherConfig::default(),
    )
    .unwrap();

    assert!(result.selected.is_some());
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

    // Skript's ParseTagPatternElement treats a numeric `:` tag as both a tag
    // and an XOR parse mark. EffExit relies on this legacy `1:loop` form.
    let source = "(section|1:loop|2:conditional)";
    let pattern = parse(source);
    let selected = match_one("loop", source, &pattern)
        .unwrap()
        .selected
        .unwrap();
    assert_eq!(selected.matched.tags[0].value, "1");
    assert_eq!(selected.matched.mark, 1);
    assert_eq!(selected.matched.marks[0].value, 1);

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
                pattern_index: 0,
                source: "other",
                parsed: &other,
            },
            MatchPattern {
                pattern_index: 1,
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
    let result = match_one("abcxyQ", "abc(def|xyz)", &pattern).unwrap();
    let failure = &result.primary_failure().unwrap().failure;
    assert_eq!(failure.span.mapped.virtual_range.start, 5);
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

#[test]
fn near_match_requires_a_literal_anchor_before_dynamic_elements() {
    let anchored_source = "teleport %entities% to %location%";
    let anchored = parse(anchored_source);
    let input = "teleport invalid";
    let mapped = MappedSource::identity(input);
    let result = match_pattern_candidates(
        MatchInput::from_source(&mapped, TextRange::new(0, input.len())).unwrap(),
        &[candidate(anchored_source, &anchored, 0)],
        &mut RejectTypeExpressions,
        &mut NoopPatternMatchHooks,
        PatternMatcherConfig::default(),
    )
    .unwrap();
    assert!(result.failures.primary().is_some());

    let generic_source = "<.+> if <.+>";
    let generic = parse(generic_source);
    let input = "not an effect";
    let mapped = MappedSource::identity(input);
    let result = match_pattern_candidates(
        MatchInput::from_source(&mapped, TextRange::new(0, input.len())).unwrap(),
        &[candidate(generic_source, &generic, 0)],
        &mut RejectTypeExpressions,
        &mut NoopPatternMatchHooks,
        PatternMatcherConfig::default(),
    )
    .unwrap();
    assert!(result.failures.primary().is_none());
}

#[test]
fn near_match_recognizes_literal_anchors_inside_a_leading_group() {
    let source = "(message|send [message[s]]) %objects% [to %audiences%]";
    let pattern = parse(source);
    let input = "send invalid";
    let mapped = MappedSource::identity(input);
    let result = match_pattern_candidates(
        MatchInput::from_source(&mapped, TextRange::new(0, input.len())).unwrap(),
        &[candidate(source, &pattern, 0)],
        &mut RejectTypeExpressions,
        &mut NoopPatternMatchHooks,
        PatternMatcherConfig::default(),
    )
    .unwrap();

    assert!(result.failures.primary().is_some());
    assert_eq!(result.failures.candidates.len(), 1);
}

#[test]
fn failed_type_span_stops_before_the_next_literal_separator() {
    let source = "[:force] teleport %entities% (to|%direction%) %location% [[while] retaining %-teleportflags%]";
    let pattern = parse(source);
    let input = "teleport all player to location(1, 2, 3)";
    let mapped = MappedSource::identity(input);
    let result = match_pattern_candidates(
        MatchInput::from_source(&mapped, TextRange::new(0, input.len())).unwrap(),
        &[candidate(source, &pattern, 0)],
        &mut RejectTypeExpressions,
        &mut NoopPatternMatchHooks,
        PatternMatcherConfig::default(),
    )
    .unwrap();

    let failure = result
        .failures
        .primary()
        .expect("teleport remains recognizable");
    assert_eq!(
        failure.trace.root_cause().failure.span.local_range,
        TextRange::new(9, 19)
    );
    assert_eq!(
        failure
            .trace
            .root_cause()
            .failure
            .span
            .local_range
            .slice(input),
        Some("all player")
    );
}

#[test]
fn recovery_collects_two_typed_capture_failures_without_selecting_candidate() {
    let source = "teleport %entities% to %location%";
    let pattern = parse(source);
    let input = "teleport a to location(b, 2, 3)";
    let mapped = MappedSource::identity(input);
    let result = match_pattern_candidates(
        MatchInput::from_source(&mapped, TextRange::new(0, input.len())).unwrap(),
        &[candidate(source, &pattern, 0)],
        &mut RejectTypeExpressions,
        &mut NoopPatternMatchHooks,
        PatternMatcherConfig {
            recover_type_expression_failures: true,
            ..PatternMatcherConfig::default()
        },
    )
    .unwrap();

    assert!(result.selected.is_none());
    assert_eq!(result.failures.candidates.len(), 1);
    let failure = result
        .failures
        .primary()
        .expect("recovered candidate failure");
    assert_eq!(
        failure.trace.root_cause().failure.span.local_range,
        TextRange::new(9, 10)
    );
    assert_eq!(failure.related.len(), 1);
    assert_eq!(
        failure.related[0].root_cause().failure.span.local_range,
        TextRange::new(14, 31)
    );
    assert_eq!(
        failure.related[0]
            .root_cause()
            .failure
            .span
            .local_range
            .slice(input),
        Some("location(b, 2, 3)")
    );
    assert!(
        failure
            .trace
            .root_cause()
            .failure
            .reasons
            .iter()
            .any(|reason| matches!(reason, PatternFailureReason::TypeExpression { .. }))
    );
    assert!(
        failure.related[0]
            .root_cause()
            .failure
            .reasons
            .iter()
            .any(|reason| matches!(reason, PatternFailureReason::TypeExpression { .. }))
    );
}

#[test]
fn successful_teleport_matching_is_unchanged_when_recovery_is_disabled() {
    let source = "teleport %entities% to %location%";
    let pattern = parse(source);
    let input = "teleport all players to location(1, 2, 3)";
    let mapped = MappedSource::identity(input);
    let result = match_pattern_candidates(
        MatchInput::from_source(&mapped, TextRange::new(0, input.len())).unwrap(),
        &[candidate(source, &pattern, 0)],
        &mut AcceptTypeExpressions,
        &mut NoopPatternMatchHooks,
        PatternMatcherConfig::default(),
    )
    .unwrap();

    assert!(result.selected.is_some());
    assert!(result.failures.primary().is_none());
}

#[test]
fn near_match_prefers_a_concrete_failed_capture_at_the_same_offset() {
    let vague_source = "teleport %livingentity% towards %location%";
    let vague = parse(vague_source);
    let concrete_source = "teleport %entities% to %location%";
    let concrete = parse(concrete_source);
    let input = "teleport no to somewhere";
    let mapped = MappedSource::identity(input);
    let result = match_pattern_candidates(
        MatchInput::from_source(&mapped, TextRange::new(0, input.len())).unwrap(),
        &[
            candidate(vague_source, &vague, 0),
            PatternCandidate {
                registration_id: "effect:test#1".to_owned(),
                registration_order: 1,
                ..candidate(concrete_source, &concrete, 1)
            },
        ],
        &mut RejectTypeExpressions,
        &mut NoopPatternMatchHooks,
        PatternMatcherConfig::default(),
    )
    .unwrap();

    let failure = result
        .failures
        .primary()
        .expect("anchored candidate expected");
    assert_eq!(failure.registration_id, "effect:test#1");
    assert_eq!(
        failure.trace.root_cause().failure.span.local_range,
        TextRange::new(9, 11)
    );
}

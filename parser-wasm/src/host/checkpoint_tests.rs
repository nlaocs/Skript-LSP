use super::*;

fn call(subscription: &str) -> HookCall {
    HookCall {
        component_id: "component".into(),
        subscription_id: subscription.into(),
        semantic_receipt: None,
    }
}

fn update(value: &str) -> ContextUpdate {
    ContextUpdate {
        syntax_context: 7,
        key: "branch".into(),
        value: Some(value.as_bytes().to_vec()),
    }
}

#[test]
fn checkpoints_restore_selected_calls_after_shorter_equal_and_longer_branches() {
    for other_count in 0..=3 {
        let mut effects = Arc::new(empty_effects());
        Arc::make_mut(&mut effects)
            .context_updates
            .push(update("base"));
        let mut calls = HookCallLog::from(vec![call("base")]);
        let mut failures = Vec::new();
        let base = HookEffectsCheckpoint::capture(&effects, &calls, &failures);

        Arc::make_mut(&mut effects)
            .context_updates
            .push(update("selected"));
        extend_hook_calls(&mut calls, vec![call("selected-1"), call("selected-2")]);
        let selected = HookEffectsCheckpoint::capture(&effects, &calls, &failures);
        let selected_clone = selected.clone();
        assert!(calls.shares_tail(&selected.calls));

        base.restore(&mut effects, &mut calls, &mut failures);
        assert_eq!(calls.suffix(0), vec![call("base")]);
        Arc::make_mut(&mut effects)
            .context_updates
            .push(update("other"));
        extend_hook_calls(&mut calls, vec![call("other"); other_count]);

        selected.restore(&mut effects, &mut calls, &mut failures);
        assert_eq!(
            calls.suffix(0),
            vec![call("base"), call("selected-1"), call("selected-2")]
        );
        assert_eq!(effects.context_updates.len(), 2);
        assert_eq!(effects.context_updates[1].value, Some(b"selected".to_vec()));
        extend_hook_calls(&mut calls, vec![call("later")]);
        selected_clone.restore(&mut effects, &mut calls, &mut failures);
        assert_eq!(
            calls.suffix(0),
            vec![call("base"), call("selected-1"), call("selected-2")]
        );
        assert_eq!(base.calls.suffix(0), vec![call("base")]);
    }
}

#[test]
fn empty_dispatch_keeps_shared_calls_and_nonempty_dispatch_detaches_in_order() {
    let mut calls = HookCallLog::from(vec![call("first")]);
    let saved = calls.clone();
    extend_hook_calls(&mut calls, Vec::new());
    assert!(calls.shares_tail(&saved));
    extend_hook_calls(&mut calls, vec![call("second"), call("third")]);
    assert!(!calls.shares_tail(&saved));
    assert_eq!(saved.suffix(0), vec![call("first")]);
    assert_eq!(
        calls.suffix(0),
        vec![call("first"), call("second"), call("third")]
    );
    let owned = calls.into_vec();
    assert_eq!(owned, vec![call("first"), call("second"), call("third")]);
    assert_eq!(saved.into_vec(), vec![call("first")]);
}

#[test]
fn restoring_calls_retains_new_failures_and_updated_fault_spans() {
    let mut effects = Arc::new(empty_effects());
    let mut calls = HookCallLog::from(vec![call("base")]);
    let failure = || ComponentFailure {
        affected_span: None,
        component_id: "faulty".into(),
        subscription_id: "failed".into(),
        error: HostError::UnavailableComponent {
            component_id: "faulty".into(),
            reason: "unavailable".into(),
        },
    };
    let mut failures = vec![failure()];
    let base = HookEffectsCheckpoint::capture(&effects, &calls, &failures);
    failures[0].affected_span = Some(MappedSpan {
        virtual_range: WitTextRange { start: 2, end: 5 },
        origins: Vec::new(),
    });
    failures.push(failure());
    extend_hook_calls(&mut calls, vec![call("discarded")]);
    base.restore(&mut effects, &mut calls, &mut failures);
    assert_eq!(calls.suffix(0), vec![call("base")]);
    assert_eq!(failures.len(), 2);
    let span = failures[0].affected_span.as_ref().unwrap();
    assert_eq!((span.virtual_range.start, span.virtual_range.end), (2, 5));
    assert_eq!(failures[1].subscription_id, "failed");
    assert!(matches!(
        failures[1].error,
        HostError::UnavailableComponent { .. }
    ));
}

#[test]
fn shared_write_followed_by_unique_appends_preserves_both_logs() {
    let mut calls = HookCallLog::from(vec![call("base")]);
    let saved = calls.clone();
    extend_hook_calls(&mut calls, vec![call("selected")]);
    assert!(!calls.shares_tail(&saved));
    for subscription in ["later-1", "later-2", "later-3"] {
        extend_hook_calls(&mut calls, vec![call(subscription)]);
    }
    assert_eq!(saved.suffix(0), vec![call("base")]);
    assert_eq!(
        calls.suffix(0),
        vec![
            call("base"),
            call("selected"),
            call("later-1"),
            call("later-2"),
            call("later-3")
        ]
    );
}

#[test]
fn append_preserves_prefix_until_the_last_log_owner_drops() {
    let mut calls = HookCallLog::from(vec![call("base")]);
    let observer = calls.observer();
    extend_hook_calls(&mut calls, vec![call("later")]);
    assert!(observer.upgrade().is_some());
    assert_eq!(calls.suffix(0), vec![call("base"), call("later")]);
    drop(calls);
    assert!(observer.upgrade().is_none());
}

fn with_environment(test: impl FnOnce(WasmExpressionEnvironment<'_>)) {
    let mut host = ParserHost::new(
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../artifacts/core-library.wasm"
        )),
        HostConfig {
            runtime_profile: RuntimeProfile {
                skript_version: Some("2.15.4".into()),
                ..RuntimeProfile::default()
            },
            ..HostConfig::default()
        },
    )
    .unwrap();
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/checkpoint.sk", 1)
        .unwrap();
    test(WasmExpressionEnvironment {
        hooks: WasmPatternHooks {
            host: &mut host,
            transaction: &transaction,
            dynamic_snapshot: None,
            matching_hooks_registered: false,
            context: InvocationContext {
                invocation_id: 1,
                subscription_id: String::new(),
                document_id: "file:///workspace/checkpoint.sk".into(),
                document_revision: 1,
                expansion: None,
                syntax_context: 0,
            },
            input: String::new(),
            frames: Vec::new(),
            scope_frames: Vec::new(),
            branch_states: Vec::new(),
            last_candidate: None,
            pending_fault: None,
            pending_cancellation: None,
            pending_unavailable: None,
            effects: Arc::new(empty_effects()),
            calls: HookCallLog::default(),
            failures: Vec::new(),
        },
        pending_leaf: None,
        pending_registered: None,
        expression_candidates: Vec::new(),
        semantic_candidates: Vec::new(),
        function_registry: None,
        structure_preparation: None,
    });
}

#[test]
fn deferred_call_suffix_applies_once_and_reapplies_after_rollback() {
    with_environment(|mut environment| {
        merge_shared_effects(&mut environment.hooks.effects, all_effects("prefix", 32));
        extend_hook_calls(&mut environment.hooks.calls, vec![call("prefix")]);
        let state = environment.hooks.transaction.savepoint().unwrap();
        let checkpoint = HookEffectsCheckpoint::capture(
            &environment.hooks.effects,
            &environment.hooks.calls,
            &environment.hooks.failures,
        );
        extend_hook_calls(
            &mut environment.hooks.calls,
            vec![call("suffix-1"), call("suffix-2")],
        );
        merge_shared_effects(&mut environment.hooks.effects, all_effects("suffix", 24));
        let deferred = environment.defer_effects(&state, &checkpoint).unwrap();
        assert_eq!(effect_lengths(&environment.hooks.effects), [1; 4]);
        assert_eq!(environment.hooks.calls.suffix(0), vec![call("prefix")]);
        assert_eq!(
            deferred
                .downcast_ref::<DeferredExpressionEffects>()
                .unwrap()
                .calls,
            vec![call("suffix-1"), call("suffix-2")]
        );
        for _ in 0..2 {
            environment.apply_expression_effects(&deferred).unwrap();
            assert_eq!(effect_lengths(&environment.hooks.effects), [2; 4]);
            assert_eq!(
                environment.hooks.calls.suffix(0),
                vec![call("prefix"), call("suffix-1"), call("suffix-2")]
            );
        }
        environment.hooks.transaction.rollback_to(&state).unwrap();
        checkpoint.restore(
            &mut environment.hooks.effects,
            &mut environment.hooks.calls,
            &mut environment.hooks.failures,
        );
        environment.apply_expression_effects(&deferred).unwrap();
        assert_eq!(effect_lengths(&environment.hooks.effects), [2; 4]);
        assert_eq!(environment.hooks.effects.diagnostics[1].message, "suffix");
        assert_eq!(
            environment.hooks.effects.context_updates[1].value,
            Some(b"suffix".to_vec())
        );
        assert_eq!(environment.hooks.effects.parse_requests[1].input, "suffix");
        assert_eq!(environment.hooks.effects.parse_results[1].nodes.len(), 24);
        assert_eq!(
            environment.hooks.calls.suffix(0),
            vec![call("prefix"), call("suffix-1"), call("suffix-2")]
        );
        let saved = environment.hooks.calls.clone();
        let (_, mut exported, _) = environment.into_parts();
        exported.push(call("outside"));
        assert_eq!(
            saved.suffix(0),
            vec![call("prefix"), call("suffix-1"), call("suffix-2")]
        );
    });
}

#[test]
fn checked_dispatch_drains_calls_once_and_retains_fault_markers() {
    with_environment(|mut environment| {
        let dispatch = |fault| DispatchResult {
            fault,
            decision: HookDecision::ContinueProcessing,
            payload: HookPayload::Parser(ParseRequest {
                request_id: 1,
                parser_id: "test".into(),
                input: String::new(),
                expected_types: Vec::new(),
                options: Vec::new(),
                span: MappedSpan {
                    virtual_range: WitTextRange { start: 0, end: 0 },
                    origins: Vec::new(),
                },
            }),
            effects: empty_effects(),
            calls: vec![call("dispatch")],
            failures: Vec::new(),
            available_parse_results: BTreeMap::new(),
        };
        let result = environment.hooks.checked_dispatch(dispatch(None)).unwrap();
        assert!(result.calls.is_empty());
        let saved = environment.hooks.calls.clone();
        extend_hook_calls(&mut environment.hooks.calls, result.calls);
        assert!(saved.shares_tail(&environment.hooks.calls));
        let fault = skript_parser::ExtensionFault {
            component_id: "component".into(),
            subscription_id: "dispatch".into(),
            phase: "parse".into(),
            target: "expression".into(),
            code: "test.fault".into(),
            message: "failed".into(),
        };
        let mut failed = dispatch(Some(fault.clone()));
        failed.failures.push(ComponentFailure {
            affected_span: None,
            component_id: fault.component_id.clone(),
            subscription_id: fault.subscription_id.clone(),
            error: HostError::ExtensionFault(fault.clone()),
        });
        assert!(environment.hooks.checked_dispatch(failed).is_err());
        assert_eq!(environment.hooks.pending_fault.as_ref(), Some(&fault));
        assert_eq!(environment.hooks.failures.len(), 1);
        assert_eq!(saved.suffix(0), vec![call("dispatch")]);
        let (_, exported, _) = environment.into_parts();
        assert_eq!(exported, vec![call("dispatch"), call("dispatch")]);
    });
}

#[test]
fn old_log_generations_live_until_their_last_checkpoint_is_dropped() {
    let effects = Arc::new(empty_effects());
    let mut calls = HookCallLog::from(vec![call(&"prefix".repeat(80)); 16]);
    let mut checkpoints = Vec::new();
    let mut generations = Vec::new();
    for index in 0..48 {
        checkpoints.push(HookEffectsCheckpoint::capture(&effects, &calls, &[]));
        let generation = calls.observer();
        // Appending shares this prefix with the next chunk and the checkpoint.
        extend_hook_calls(&mut calls, vec![call(&format!("branch-{index}"))]);
        assert!(generation.upgrade().is_some());
        generations.push(generation);
    }
    let retained = checkpoints[7].clone();
    let latest = calls.observer();
    drop(calls);
    assert!(latest.upgrade().is_none());
    for (index, checkpoint) in checkpoints.into_iter().enumerate().rev() {
        assert_eq!(checkpoint.calls.len(), 16 + index);
        assert_eq!(
            checkpoint.calls.suffix(0)[0].subscription_id,
            "prefix".repeat(80)
        );
        drop(checkpoint);
        assert_eq!(generations[index].upgrade().is_some(), index <= 7);
    }
    assert_eq!(retained.calls.len(), 23);
    drop(retained);
    assert!(
        generations
            .iter()
            .all(|generation| generation.upgrade().is_none())
    );
}

#[test]
fn branch_restore_retains_saved_generations_until_the_matcher_frame_finishes() {
    with_environment(|mut environment| {
        let hooks = &mut environment.hooks;
        extend_hook_calls(&mut hooks.calls, vec![call("base")]);
        hooks.begin_match_frame().unwrap();
        extend_hook_calls(&mut hooks.calls, vec![call("a")]);
        let branch_a = hooks.checkpoint_branch_state().unwrap().unwrap();
        let generation_a = hooks.calls.observer();
        extend_hook_calls(&mut hooks.calls, vec![call("b")]);
        hooks.checkpoint_branch_state().unwrap();
        let generation_b = hooks.calls.observer();
        hooks.restore_branch_state(branch_a).unwrap();
        assert_eq!(hooks.calls.suffix(0), vec![call("base"), call("a")]);
        assert!(generation_a.upgrade().is_some());
        assert!(generation_b.upgrade().is_some());
        hooks.finish_match_frame(false).unwrap();
        assert_eq!(hooks.calls.suffix(0), vec![call("base")]);
        assert!(hooks.frames.is_empty());
        assert!(hooks.branch_states.is_empty());
        assert!(generation_a.upgrade().is_none());
        assert!(generation_b.upgrade().is_none());
    });
}

#[test]
fn exported_candidate_state_keeps_its_log_alive_after_the_environment_is_dropped() {
    let mut exported = None;
    let mut generation = None;
    with_environment(|mut environment| {
        extend_hook_calls(&mut environment.hooks.calls, vec![call("selected")]);
        environment.hooks.last_candidate = Some(SavedPatternCandidate {
            restored: None,
            state: environment.hooks.transaction.savepoint().unwrap(),
            effects: HookEffectsCheckpoint::capture(
                &environment.hooks.effects,
                &environment.hooks.calls,
                &environment.hooks.failures,
            ),
        });
        generation = Some(environment.hooks.calls.observer());
        exported = environment.take_pattern_candidate_state();
        assert!(environment.hooks.last_candidate.is_none());
    });
    let generation = generation.unwrap();
    let exported = exported.unwrap();
    let retained = exported.clone();
    assert!(generation.upgrade().is_some());
    drop(exported);
    assert_eq!(
        retained
            .downcast_ref::<SavedPatternCandidate>()
            .unwrap()
            .effects
            .calls
            .suffix(0)[0],
        call("selected")
    );
    assert!(generation.upgrade().is_some());
    drop(retained);
    assert!(generation.upgrade().is_none());
}

struct CancelInsideFrame<'a, 'host> {
    hooks: &'a mut WasmPatternHooks<'host>,
    token: DocumentCancellationToken,
    close_transaction: bool,
    injected: bool,
    generation: Option<std::sync::Weak<hook_calls::Chunk>>,
}

impl PatternMatchHooks for CancelInsideFrame<'_, '_> {
    fn begin_match(&mut self) -> Result<(), String> {
        self.hooks.begin_match_frame()
    }

    fn cancellation_requested(&mut self) -> bool {
        if !self.injected && !self.hooks.frames.is_empty() {
            self.injected = true;
            extend_hook_calls(&mut self.hooks.calls, vec![call("branch")]);
            self.hooks
                .prepare_nested_scope(PatternHookScope::Pattern)
                .unwrap();
            self.hooks.checkpoint_branch_state().unwrap();
            self.generation = Some(self.hooks.calls.observer());
            extend_hook_calls(&mut self.hooks.calls, vec![call("later")]);
            assert!(self.generation.as_ref().unwrap().upgrade().is_some());
            self.token.cancel();
            if self.close_transaction {
                self.hooks.transaction.cancel().unwrap();
            }
        }
        self.hooks.cancellation_requested()
    }

    fn finish_match(&mut self, accepted: bool) -> Result<(), String> {
        self.hooks.finish_match_frame(accepted)
    }

    fn dispatch(&mut self, event: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
        self.hooks.dispatch(event)
    }
}

#[test]
fn cancellation_releases_branch_logs_and_cleanup_errors_release_them_on_environment_drop() {
    for close_transaction in [false, true] {
        with_environment(|mut environment| {
            let token = DocumentCancellationToken::new();
            environment.hooks.host.native_cancellation = Some(token.clone());
            extend_hook_calls(&mut environment.hooks.calls, vec![call("base")]);
            let source = MappedSource::identity("do");
            let rules =
                syntax_pattern_parser::syntax::PluralRules::from_json(include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../syntax-pattern-parser/tests/data/PluralRules-2.15.4.json"
                )))
                .unwrap();
            let parsed = syntax_pattern_parser::syntax::parse("do", &rules).unwrap();
            let candidate = PatternCandidate {
                kind: MatchSyntaxKind::Effect,
                definition_id: "test".into(),
                registration_id: "test.0".into(),
                priority: 0,
                registration_order: 0,
                resolved_order: None,
                patterns: vec![MatchPattern {
                    pattern_index: 0,
                    source: "do",
                    parsed: &parsed,
                }],
            };
            let mut cancelling = CancelInsideFrame {
                hooks: &mut environment.hooks,
                token,
                close_transaction,
                injected: false,
                generation: None,
            };
            let error = run_pattern_matcher(
                MatchInput::from_source(&source, ParserTextRange::new(0, 2)).unwrap(),
                &[candidate],
                &mut skript_parser::RejectTypeExpressions,
                &mut cancelling,
                PatternMatcherConfig::default(),
            )
            .unwrap_err();
            assert!(cancelling.injected);
            assert!(cancelling.token.is_cancelled());
            let generation = cancelling.generation.take().unwrap();
            drop(cancelling);
            if close_transaction {
                assert!(matches!(
                    error,
                    skript_parser::PatternMatchError::Hook { .. }
                ));
                assert!(skript_parser::parse_cancellation(&error).is_none());
                assert!(!environment.hooks.branch_states.is_empty());
                assert!(!environment.hooks.scope_frames.is_empty());
                assert!(generation.upgrade().is_some());
            } else {
                assert!(skript_parser::parse_cancellation(&error).is_some());
                assert_eq!(environment.hooks.calls.suffix(0), vec![call("base")]);
                assert!(environment.hooks.frames.is_empty());
                assert!(environment.hooks.branch_states.is_empty());
                assert!(environment.hooks.scope_frames.is_empty());
                assert!(generation.upgrade().is_none());
            }
            drop(environment);
            assert!(generation.upgrade().is_none());
        });
    }
}

fn arena(marker: &str, count: u64) -> ParseResult {
    let span = MappedSpan {
        virtual_range: WitTextRange { start: 0, end: 1 },
        origins: Vec::new(),
    };
    ParseResult {
        host_token: 0,
        request_id: 1,
        parser_id: "fixture.parser".into(),
        status: WitParseResultStatus::Success,
        roots: (0..count).collect(),
        nodes: (0..count)
            .map(|node_id| ParseResultNode {
                node_id,
                parser_id: "fixture.parser".into(),
                kind: "fixture".into(),
                status: WitParseResultStatus::Success,
                text: marker.repeat(128),
                span: span.clone(),
                expected_types: Vec::new(),
                summary: None,
                children: Vec::new(),
                function: None,
                attachments: Vec::new(),
                diagnostics: Vec::new(),
                metadata: Vec::new(),
            })
            .collect(),
        diagnostics: Vec::new(),
    }
}

fn all_effects(marker: &str, nodes: u64) -> HookEffects {
    let span = MappedSpan {
        virtual_range: WitTextRange { start: 0, end: 1 },
        origins: Vec::new(),
    };
    HookEffects {
        diagnostics: vec![Diagnostic {
            code: marker.into(),
            message: marker.into(),
            severity: DiagnosticSeverity::Information,
            span: span.clone(),
            related: Vec::new(),
        }],
        context_updates: vec![update(marker)],
        parse_requests: vec![ParseRequest {
            request_id: 1,
            parser_id: "fixture.parser".into(),
            input: marker.into(),
            expected_types: Vec::new(),
            span,
            options: Vec::new(),
        }],
        parse_results: vec![arena(marker, nodes)],
    }
}

fn effect_lengths(effects: &HookEffects) -> [usize; 4] {
    [
        effects.diagnostics.len(),
        effects.context_updates.len(),
        effects.parse_requests.len(),
        effects.parse_results.len(),
    ]
}

#[test]
fn empty_effect_merge_keeps_shared_prefix_and_each_nonempty_field_detaches() {
    for field in 0..4 {
        let mut effects = Arc::new(all_effects("base", 64));
        let saved = Arc::clone(&effects);
        merge_shared_effects(&mut effects, empty_effects());
        assert!(
            Arc::ptr_eq(&effects, &saved),
            "empty speculative work must not copy arenas"
        );
        let mut added = all_effects("added", 2);
        if field != 0 {
            added.diagnostics.clear();
        }
        if field != 1 {
            added.context_updates.clear();
        }
        if field != 2 {
            added.parse_requests.clear();
        }
        if field != 3 {
            added.parse_results.clear();
        }
        merge_shared_effects(&mut effects, added);
        assert!(!Arc::ptr_eq(&effects, &saved));
        let mut expected = [1; 4];
        expected[field] = 2;
        assert_eq!(effect_lengths(&effects), expected);
        assert_eq!(effect_lengths(&saved), [1; 4]);
        assert_eq!(saved.parse_results[0].nodes.len(), 64);
        assert_eq!(saved.parse_results[0].nodes[0].text, "base".repeat(128));
    }
}

#[test]
fn complete_effect_snapshots_restore_selected_arenas_after_other_branch_lengths() {
    for count in 0..=3 {
        let mut effects = Arc::new(all_effects("base", 64));
        let mut calls = HookCallLog::default();
        let mut failures = Vec::new();
        let base = HookEffectsCheckpoint::capture(&effects, &calls, &failures);
        merge_shared_effects(&mut effects, all_effects("selected", 48));
        let selected = HookEffectsCheckpoint::capture(&effects, &calls, &failures);
        let saved = selected.clone();
        base.restore(&mut effects, &mut calls, &mut failures);
        for _ in 0..count {
            merge_shared_effects(&mut effects, all_effects("other", 80));
        }
        selected.restore(&mut effects, &mut calls, &mut failures);
        assert_eq!(effect_lengths(&effects), [2; 4]);
        assert_eq!(effects.diagnostics[1].code, "selected");
        assert_eq!(effects.context_updates[1].value, Some(b"selected".to_vec()));
        assert_eq!(effects.parse_requests[1].input, "selected");
        assert_eq!(effects.parse_results[1].nodes.len(), 48);
        assert_eq!(
            effects.parse_results[1].nodes[0].text,
            "selected".repeat(128)
        );
        let mut exported = Arc::unwrap_or_clone(effects);
        exported.parse_results[1].nodes[0].text = "outside".into();
        assert_eq!(
            saved.effects.parse_results[1].nodes[0].text,
            "selected".repeat(128)
        );
        assert_eq!(base.effects.parse_results[0].nodes.len(), 64);
    }
}

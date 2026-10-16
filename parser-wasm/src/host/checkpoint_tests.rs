use super::*;

fn call(subscription: &str) -> HookCall {
    HookCall {
        component_id: "component".into(),
        subscription_id: subscription.into(),
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
        let mut effects = empty_effects();
        effects.context_updates.push(update("base"));
        let mut calls = Arc::new(vec![call("base")]);
        let mut failures = Vec::new();
        let base = HookEffectsCheckpoint::capture(&effects, &calls, &failures);

        effects.context_updates.push(update("selected"));
        extend_hook_calls(&mut calls, vec![call("selected-1"), call("selected-2")]);
        let selected = HookEffectsCheckpoint::capture(&effects, &calls, &failures);
        let selected_clone = selected.clone();
        assert!(Arc::ptr_eq(&calls, &selected.calls));

        base.restore(&mut effects, &mut calls, &mut failures);
        assert_eq!(*calls, vec![call("base")]);
        effects.context_updates.push(update("other"));
        extend_hook_calls(&mut calls, vec![call("other"); other_count]);

        selected.restore(&mut effects, &mut calls, &mut failures);
        assert_eq!(
            *calls,
            vec![call("base"), call("selected-1"), call("selected-2")]
        );
        assert_eq!(effects.context_updates.len(), 2);
        assert_eq!(effects.context_updates[1].value, Some(b"selected".to_vec()));
        extend_hook_calls(&mut calls, vec![call("later")]);
        selected_clone.restore(&mut effects, &mut calls, &mut failures);
        assert_eq!(
            *calls,
            vec![call("base"), call("selected-1"), call("selected-2")]
        );
        assert_eq!(*base.calls, vec![call("base")]);
    }
}

#[test]
fn empty_dispatch_keeps_shared_calls_and_nonempty_dispatch_detaches_in_order() {
    let mut calls = Arc::new(vec![call("first")]);
    let saved = Arc::clone(&calls);
    extend_hook_calls(&mut calls, Vec::new());
    assert!(Arc::ptr_eq(&calls, &saved));
    extend_hook_calls(&mut calls, vec![call("second"), call("third")]);
    assert!(!Arc::ptr_eq(&calls, &saved));
    assert_eq!(*saved, vec![call("first")]);
    assert_eq!(*calls, vec![call("first"), call("second"), call("third")]);
    let owned = Arc::unwrap_or_clone(calls);
    assert_eq!(owned, vec![call("first"), call("second"), call("third")]);
    assert_eq!(Arc::unwrap_or_clone(saved), vec![call("first")]);
}

#[test]
fn restoring_calls_retains_new_failures_and_updated_fault_spans() {
    let mut effects = empty_effects();
    let mut calls = Arc::new(vec![call("base")]);
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
    assert_eq!(*calls, vec![call("base")]);
    assert_eq!(failures.len(), 2);
    let span = failures[0].affected_span.as_ref().unwrap();
    assert_eq!((span.virtual_range.start, span.virtual_range.end), (2, 5));
    assert_eq!(failures[1].subscription_id, "failed");
    assert!(matches!(
        failures[1].error,
        HostError::UnavailableComponent { .. }
    ));
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
            effects: empty_effects(),
            calls: Arc::default(),
            failures: Vec::new(),
        },
        pending_leaf: None,
        pending_registered: None,
        expression_candidates: Vec::new(),
        semantic_candidates: Vec::new(),
        function_registry: None,
    });
}

#[test]
fn deferred_call_suffix_applies_once_and_reapplies_after_rollback() {
    with_environment(|mut environment| {
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
        let deferred = environment.defer_effects(&state, &checkpoint).unwrap();
        assert_eq!(*environment.hooks.calls, vec![call("prefix")]);
        assert_eq!(
            deferred
                .downcast_ref::<DeferredExpressionEffects>()
                .unwrap()
                .calls,
            vec![call("suffix-1"), call("suffix-2")]
        );
        for _ in 0..2 {
            environment.apply_expression_effects(&deferred).unwrap();
            assert_eq!(
                *environment.hooks.calls,
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
        assert_eq!(
            *environment.hooks.calls,
            vec![call("prefix"), call("suffix-1"), call("suffix-2")]
        );
        let saved = Arc::clone(&environment.hooks.calls);
        let (_, mut exported, _) = environment.into_parts();
        exported.push(call("outside"));
        assert_eq!(
            *saved,
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
        let saved = Arc::clone(&environment.hooks.calls);
        extend_hook_calls(&mut environment.hooks.calls, result.calls);
        assert!(Arc::ptr_eq(&saved, &environment.hooks.calls));
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
        assert_eq!(*saved, vec![call("dispatch")]);
        let (_, exported, _) = environment.into_parts();
        assert_eq!(exported, vec![call("dispatch"), call("dispatch")]);
    });
}

use super::*;

#[derive(Default)]
struct CancellingEnvironment {
    cancel_after: Option<usize>,
    probes: usize,
    cancelled: bool,
    pending_cancellation: bool,
    cancel_defaults: bool,
    fail_restore: bool,
    fail_finish: bool,
    finished: Vec<bool>,
}

impl PatternMatchEnvironment for CancellingEnvironment {
    fn cancellation_requested(&mut self) -> bool {
        self.probes += 1;
        if self.cancel_after.is_some_and(|limit| self.probes > limit) {
            self.cancelled = true;
        }
        self.cancelled
    }

    fn take_parse_cancellation(&mut self) -> Option<skript_parser::ParseCancelled> {
        std::mem::take(&mut self.pending_cancellation).then_some(skript_parser::ParseCancelled)
    }

    fn finish_pattern_match(&mut self, accepted: bool) -> Result<(), String> {
        self.finished.push(accepted);
        if self.fail_finish {
            Err("finish failed".into())
        } else {
            Ok(())
        }
    }

    fn checkpoint_pattern_branch(&mut self) -> Result<Option<u64>, String> {
        Ok(Some(0))
    }

    fn restore_pattern_branch(&mut self, _: u64) -> Result<(), String> {
        if self.cancelled && self.fail_restore {
            Err("restore failed".into())
        } else {
            Ok(())
        }
    }

    fn complete_typed_captures(
        &mut self,
        _: &PatternCandidate<'_>,
        _: &MatchPattern<'_>,
        _: &mut PatternMatch,
    ) -> Result<Option<FailureTrace>, String> {
        assert!(self.cancel_defaults);
        self.cancelled = true;
        self.pending_cancellation = true;
        Err(skript_parser::ParseCancelled.to_string())
    }

    fn resolve_type(
        &mut self,
        _: TypeExpressionRequest<'_>,
    ) -> Result<TypeExpressionOutcome, String> {
        Ok(TypeExpressionOutcome::default())
    }

    fn dispatch_hook(&mut self, _: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
        Ok(PatternHookControl::Continue)
    }
}

impl PatternMatchHooks for CancellingEnvironment {
    fn cancellation_requested(&mut self) -> bool {
        PatternMatchEnvironment::cancellation_requested(self)
    }

    fn take_parse_cancellation(&mut self) -> Option<skript_parser::ParseCancelled> {
        PatternMatchEnvironment::take_parse_cancellation(self)
    }

    fn finish_match(&mut self, accepted: bool) -> Result<(), String> {
        self.finish_pattern_match(accepted)
    }

    fn dispatch(&mut self, event: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
        self.dispatch_hook(event)
    }
}

#[test]
fn native_cancellation_stops_matcher_and_finalizes_rejected_work() {
    for fail_finish in [false, true] {
        let source = "(a|b|c) tail";
        let pattern = parse(source);
        let mapped = MappedSource::identity("a tail");
        let mut hooks = CancellingEnvironment {
            cancel_after: Some(3),
            fail_finish,
            ..Default::default()
        };
        let error = match_pattern_candidates(
            MatchInput::from_source(&mapped, TextRange::new(0, 6)).unwrap(),
            &[candidate(source, &pattern, 0)],
            &mut RejectTypeExpressions,
            &mut hooks,
            PatternMatcherConfig::default(),
        )
        .unwrap_err();
        assert_eq!(hooks.probes, 4);
        assert_eq!(hooks.finished, [false]);
        if fail_finish {
            assert!(
                matches!(error, PatternMatchError::Hook { ref message } if message == "finish failed")
            );
            assert!(skript_parser::parse_cancellation(&error).is_none());
        } else {
            assert!(skript_parser::parse_cancellation(&error).is_some());
        }
    }
}

#[test]
fn default_cancellation_preserves_a_later_cleanup_failure() {
    for fail_restore in [false, true] {
        let source = "do [%number%]";
        let pattern = parse(source);
        let mapped = MappedSource::identity("do");
        let mut environment = CancellingEnvironment {
            cancel_defaults: true,
            fail_restore,
            ..Default::default()
        };
        let error = match_pattern_candidates_with_environment(
            MatchInput::from_source(&mapped, TextRange::new(0, 2)).unwrap(),
            &[candidate(source, &pattern, 0)],
            &mut environment,
            PatternMatcherConfig::default(),
        )
        .unwrap_err();
        assert_eq!(environment.finished, [false]);
        assert!(environment.cancelled);
        assert!(!environment.pending_cancellation);
        if fail_restore {
            assert!(
                matches!(error, PatternMatchError::Hook { ref message } if message == "restore failed")
            );
            assert!(skript_parser::parse_cancellation(&error).is_none());
        } else {
            assert!(skript_parser::parse_cancellation(&error).is_some());
        }
    }
}

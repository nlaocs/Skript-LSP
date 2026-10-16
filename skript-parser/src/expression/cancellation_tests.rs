use super::*;

#[derive(Default)]
struct Environment {
    cancelled: bool,
    cancel_after_leaf: bool,
    probes_after_request: usize,
    leaves: usize,
}

impl PatternMatchEnvironment for Environment {
    fn cancellation_requested(&mut self) -> bool {
        if self.cancelled {
            self.probes_after_request += 1;
        }
        self.cancelled
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

impl ExpressionParseEnvironment for Environment {
    fn state_revision(&self) -> Result<u64, String> {
        Ok(0)
    }

    fn parse_expression_leaf(
        &mut self,
        request: ExpressionLeafRequest<'_>,
    ) -> Result<ExpressionLeafParse, String> {
        self.leaves += 1;
        self.cancelled |= self.cancel_after_leaf;
        Ok(vec![ExpressionLeafCandidate {
            effects: None,
            parser_id: "test.literal".into(),
            kind: ExpressionLeafKind::Literal,
            timing: ExpressionLeafTiming::BeforeRegistered,
            range: request.remaining,
            return_type: Some(ClassName("java.lang.Object".into())),
            multiplicity: Some(Multiplicity::Single),
            children: Vec::new(),
            function: None,
            public_data: Vec::new(),
            metadata: BTreeMap::new(),
        }]
        .into())
    }
}

#[test]
fn cancellation_precedes_memo_hits_and_clears_the_session_memo() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4");
    let catalog = ssg::load(path).unwrap().into_catalog();
    let source = MappedSource::identity("x");
    let mut environment = Environment::default();
    let mut session = ExpressionSession::new(
        &catalog,
        None,
        &source,
        &mut environment,
        ExpressionParseContext::default(),
        ExpressionParserConfig::default(),
    );
    let range = TextRange::new(0, 1);
    let first = session
        .parse_complete_range(range, &[], true, true, 0)
        .unwrap();
    assert!(first.selected.is_some());
    assert!(!session.memo.is_empty());
    let leaves = session.environment.leaves;
    session.environment.cancelled = true;
    let error = session
        .parse_complete_range(range, &[], true, true, 0)
        .unwrap_err();
    assert!(crate::parse_cancellation(&error).is_some());
    assert_eq!(session.environment.leaves, leaves);
    assert!(session.memo.is_empty());
    assert!(session.active.is_empty());
}

#[test]
fn cancellation_during_recursive_work_is_not_memoized() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4");
    let catalog = ssg::load(path).unwrap().into_catalog();
    let source = MappedSource::identity("(x)");
    let mut environment = Environment {
        cancel_after_leaf: true,
        ..Default::default()
    };
    let mut session = ExpressionSession::new(
        &catalog,
        None,
        &source,
        &mut environment,
        ExpressionParseContext::default(),
        ExpressionParserConfig::default(),
    );
    // With registered expressions disabled, the inner literal seeds memo before
    // the next native seed-growth pass observes the request from its callback.
    let error = session
        .parse_complete_range(TextRange::new(0, 3), &[], true, false, 0)
        .unwrap_err();
    assert!(crate::parse_cancellation(&error).is_some());
    assert_eq!(session.environment.leaves, 1);
    assert!(session.environment.probes_after_request > 0);
    assert!(session.memo.is_empty());
    assert!(session.active.is_empty());
}

#[test]
fn routed_nested_errors_and_string_forwarding_keep_cancellation_typed() {
    let error = crate::ConditionParseError::Expression(crate::ParseCancelled.into());
    let error = nested_expression_error(error);
    assert!(crate::parse_cancellation(&error).is_some());
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4");
    let catalog = ssg::load(path).unwrap().into_catalog();
    let source = MappedSource::identity("x");
    let mut environment = Environment::default();
    let mut session = ExpressionSession::new(
        &catalog,
        None,
        &source,
        &mut environment,
        ExpressionParseContext::default(),
        ExpressionParserConfig::default(),
    );
    let message = session.forward_environment_error(error);
    let restored = session.environment_error(message);
    assert!(crate::parse_cancellation(&restored).is_some());
    let cleanup = session.environment_error("cleanup failed".into());
    assert!(crate::parse_cancellation(&cleanup).is_none());
}

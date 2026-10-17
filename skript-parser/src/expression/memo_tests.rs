use super::*;
use std::{cell::Cell, sync::OnceLock};

fn source_catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4");
        ssg::load(path).unwrap().into_catalog()
    })
}

fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let base = source_catalog();
        // Isolate the session memo from registered-language semantics. The
        // callback below supplies valid leaves through the production interface.
        Catalog::new(syntaxes::CatalogParts {
            syntaxes: Vec::new(),
            converters: Vec::new(),
            comparators: Vec::new(),
            event_values: Vec::new(),
            properties: Vec::new(),
            operators: Vec::new(),
            operations: BTreeMap::new(),
            differences: Vec::new(),
            classes: Vec::new(),
            aliases: base.aliases().clone(),
            plural_rules: base.plural_rules().clone(),
            language: BTreeMap::new(),
        })
    })
}

#[derive(Default)]
struct Environment {
    leaf_calls: usize,
    leaf_finishes: Vec<bool>,
    revision_reads: Cell<usize>,
    revision: u64,
    revision_error: bool,
    leaf_error: bool,
    no_match: bool,
    emit_effects: bool,
    single_candidate: bool,
    only_atoms: bool,
    cancel_after_outer_leaf: bool,
    cancelled: bool,
    registered_calls: usize,
    registered_error: bool,
    applied: Vec<u64>,
}

impl PatternMatchEnvironment for Environment {
    fn cancellation_requested(&mut self) -> bool {
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
    fn can_resolve_registered_expression(&self, _: RegisteredSyntaxIdentity<'_>) -> bool {
        self.registered_error
    }

    fn resolve_registered_expression(
        &mut self,
        _: RegisteredExpressionRequest<'_>,
    ) -> Result<RegisteredExpressionDecision, String> {
        self.registered_calls += 1;
        if self.registered_error {
            Err("memo registered resolver failed".into())
        } else {
            Ok(RegisteredExpressionDecision::UseDeclared)
        }
    }

    fn state_revision(&self) -> Result<u64, String> {
        self.revision_reads.set(self.revision_reads.get() + 1);
        if self.revision_error {
            Err("memo revision unavailable".into())
        } else {
            Ok(self.revision)
        }
    }

    fn parse_expression_leaf(
        &mut self,
        request: ExpressionLeafRequest<'_>,
    ) -> Result<ExpressionLeafParse, String> {
        self.leaf_calls += 1;
        self.cancelled |= self.cancel_after_outer_leaf && request.depth == 0;
        if self.leaf_error {
            return Err("memo leaf failed".into());
        }
        if self.no_match {
            let diagnostic = |code: &str| crate::SemanticDiagnostic {
                code: code.into(),
                message: format!("diagnostic {code}"),
                severity: crate::SemanticDiagnosticSeverity::Error,
                span: request.span.mapped.clone(),
                related: vec![crate::SemanticRelatedSpan {
                    message: "related origin".into(),
                    span: request.span.mapped.clone(),
                }],
            };
            return Ok(ExpressionLeafParse {
                candidates: Vec::new(),
                failure: Some(
                    FailureTrace::leaf(crate::PatternFailure {
                        span: request.span.clone(),
                        reasons: vec![crate::PatternFailureReason::TypeParserUnresolved {
                            definition_id: "type:memo".into(),
                            registration_id: "type:memo#0".into(),
                            parser_class: None,
                            reason: "memo fixture rejected the source".into(),
                            required_provider: None,
                        }],
                    })
                    .with_semantic_diagnostics(vec![diagnostic("memo.child")])
                    .with_parent(crate::FailureFrame {
                        kind: crate::MatchSyntaxKind::Expression,
                        definition_id: "expression:memo".into(),
                        registration_id: "expression:memo#0".into(),
                        pattern_index: 3,
                        pattern: "%strings%".into(),
                        element_path: vec![crate::PatternPathSegment::Element(0)],
                        pattern_span: None,
                        input_span: request.span.clone(),
                        role: crate::FailureFrameRole::TypeExpressionCapture,
                    })
                    .with_semantic_diagnostics(vec![diagnostic("memo.parent")]),
                ),
            });
        }
        let mut candidates = Vec::new();
        for &end in request.candidate_ends {
            let leaf_range = TextRange::new(request.remaining.start, end);
            if self.only_atoms && leaf_range.slice(request.input) != Some("x") {
                continue;
            }
            for index in 0..if self.single_candidate { 1 } else { 2 } {
                let parser_id = if index == 0 {
                    "memo.primary"
                } else {
                    "memo.alternative"
                };
                candidates.push(ExpressionLeafCandidate {
                    effects: self
                        .emit_effects
                        .then(|| ExpressionEffects::new(self.leaf_calls as u64 * 10 + index)),
                    parser_id: parser_id.into(),
                    kind: if request.allow_literals {
                        ExpressionLeafKind::Literal
                    } else {
                        ExpressionLeafKind::Custom
                    },
                    timing: ExpressionLeafTiming::BeforeRegistered,
                    range: leaf_range,
                    return_type: Some(request.expected_types.first().map_or_else(
                        || ClassName("java.lang.String".into()),
                        |expected| expected.class_name.clone(),
                    )),
                    multiplicity: Some(if index == 0 {
                        Multiplicity::Single
                    } else {
                        Multiplicity::Multiple
                    }),
                    children: Vec::new(),
                    function: None,
                    public_data: Vec::new(),
                    metadata: BTreeMap::from([
                        (
                            "source".into(),
                            leaf_range.slice(request.input).unwrap().into(),
                        ),
                        (
                            "request".into(),
                            format!(
                                "{:?}/{:?}/{}/{}/{}/{:?}/{}",
                                request.remaining,
                                request.expected_types,
                                request.allow_literals,
                                request.allow_expressions,
                                request.time,
                                request.context,
                                self.revision,
                            ),
                        ),
                    ]),
                });
            }
        }
        Ok(candidates.into())
    }

    fn finish_expression_leaf(&mut self, accepted: bool) -> Result<(), String> {
        self.leaf_finishes.push(accepted);
        Ok(())
    }

    fn apply_expression_effects(&mut self, effects: &ExpressionEffects) -> Result<(), String> {
        self.applied.push(*effects.downcast_ref::<u64>().unwrap());
        Ok(())
    }
}

#[derive(Clone)]
struct Request {
    range: TextRange,
    ends: Vec<usize>,
    types: Vec<ExpressionExpectedType>,
    literals: bool,
    expressions: bool,
    time: i32,
    lists: bool,
}

impl Request {
    fn new(range: TextRange) -> Self {
        Self {
            range,
            ends: vec![range.end],
            types: Vec::new(),
            literals: true,
            expressions: false,
            time: 0,
            lists: true,
        }
    }

    fn parse(
        &self,
        session: &mut ExpressionSession<'_, Environment>,
        depth: usize,
    ) -> Result<PrefixParse, ExpressionParseError> {
        session.parse_prefixes_mode(
            self.range,
            &self.ends,
            &self.types,
            self.literals,
            self.expressions,
            self.time,
            depth,
            self.lists,
        )
    }
}

fn new_session<'a>(
    source: &'a MappedSource,
    environment: &'a mut Environment,
    config: ExpressionParserConfig,
) -> ExpressionSession<'a, Environment> {
    ExpressionSession::new(
        catalog(),
        None,
        source,
        environment,
        ExpressionParseContext::default(),
        config,
    )
}

#[test]
fn same_session_hit_preserves_selected_tree_alternatives_types_and_mapped_spans() {
    let source = MappedSource::new(
        "prefix(é)suffix",
        "(é)",
        crate::ExpansionGraph::default(),
        vec![crate::SourceMapSegment::new(
            TextRange::new(0, 4),
            crate::SourceOrigin::exact(TextRange::new(6, 10), None),
        )],
    )
    .unwrap();
    let mut environment = Environment::default();
    let mut session = new_session(&source, &mut environment, ExpressionParserConfig::default());
    let range = TextRange::new(0, source.virtual_source().len());
    let cold = session
        .parse_complete_range(range, &[], true, false, 0)
        .unwrap();
    let selected = &cold.selected.as_ref().unwrap().node;
    assert_eq!(selected.kind, ExpressionNodeKind::Grouped);
    assert_eq!(selected.children.len(), 1);
    assert_eq!(
        selected.children[0].kind,
        ExpressionNodeKind::Literal {
            parser_id: "memo.primary".into()
        }
    );
    assert_eq!(
        selected.children[0].return_type,
        Some(ClassName("java.lang.String".into()))
    );
    assert_eq!(
        selected.children[0].multiplicity,
        Some(Multiplicity::Single)
    );
    assert_eq!(selected.children[0].span.local_range, TextRange::new(1, 3));
    assert_eq!(
        selected.children[0]
            .span
            .mapped
            .primary_origin()
            .unwrap()
            .original_range,
        TextRange::new(7, 9)
    );
    assert_eq!(cold.alternatives.len(), 1);
    assert_eq!(
        cold.alternatives[0].node.children[0].multiplicity,
        Some(Multiplicity::Multiple)
    );
    assert!(cold.failure.is_none());
    let calls = session.environment.leaf_calls;
    let finishes = session.environment.leaf_finishes.clone();
    let reads = session.environment.revision_reads.get();
    let entries = session.memo.len();
    assert!(calls > 0 && entries > 0);
    let hit = session
        .parse_complete_range(range, &[], true, false, 0)
        .unwrap();
    assert_eq!(cold, hit);
    assert_eq!(session.environment.leaf_calls, calls);
    assert_eq!(session.environment.leaf_finishes, finishes);
    assert_eq!(session.environment.revision_reads.get(), reads + 1);
    assert_eq!(session.memo.len(), entries);
    assert!(session.active.is_empty());
    let mut fresh_environment = Environment::default();
    let fresh = new_session(
        &source,
        &mut fresh_environment,
        ExpressionParserConfig::default(),
    )
    .parse_complete_range(range, &[], true, false, 0)
    .unwrap();
    assert_eq!(hit, fresh);
}

#[test]
fn negative_hits_keep_the_complete_failure_trace_and_do_not_dispatch_again() {
    let source = MappedSource::identity("é");
    let mut environment = Environment {
        no_match: true,
        ..Default::default()
    };
    let mut session = new_session(&source, &mut environment, ExpressionParserConfig::default());
    let range = TextRange::new(0, 2);
    let cold = session
        .parse_complete_range(range, &[], true, false, 0)
        .unwrap();
    assert!(cold.selected.is_none());
    let failure = cold.failure.as_ref().unwrap();
    assert_eq!(failure.kind, ExpressionFailureKind::ExpectedExpression);
    // The aggregate diagnostic uses an insertion point; the detailed trace
    // retains the provider's complete failing source range.
    assert_eq!(failure.span.local_range, TextRange::empty(range.start));
    let trace = failure.trace.as_ref().unwrap();
    assert_eq!(trace.failure.span.local_range, range);
    assert_eq!(trace.root_cause().failure.span.local_range, range);
    assert_eq!(trace.frame.as_ref().unwrap().input_span.local_range, range);
    assert_eq!(
        trace.frame.as_ref().unwrap().registration_id,
        "expression:memo#0"
    );
    assert!(trace.cause.is_some());
    assert_eq!(
        trace
            .semantic_diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.code.as_str())
            .collect::<Vec<_>>(),
        vec!["memo.child", "memo.parent"]
    );
    for diagnostic in trace.semantic_diagnostics() {
        assert_eq!(diagnostic.span.virtual_range, range);
        assert_eq!(diagnostic.related.len(), 1);
        assert_eq!(diagnostic.related[0].span.virtual_range, range);
    }
    assert!(
        matches!(&trace.root_cause().failure.reasons[0], crate::PatternFailureReason::TypeParserUnresolved { reason, .. } if reason == "memo fixture rejected the source")
    );
    let calls = session.environment.leaf_calls;
    let hit = session
        .parse_complete_range(range, &[], true, false, 0)
        .unwrap();
    assert_eq!(cold, hit);
    assert_eq!(session.environment.leaf_calls, calls);
    assert_eq!(session.environment.leaf_finishes, vec![false]);
    assert!(session.active.is_empty());
}

#[test]
fn hits_retain_candidate_owned_effects_without_repeating_provider_work() {
    let source = MappedSource::identity("x");
    let mut environment = Environment {
        emit_effects: true,
        ..Default::default()
    };
    let mut session = new_session(&source, &mut environment, ExpressionParserConfig::default());
    let request = Request::new(TextRange::new(0, 1));
    let cold = request.parse(&mut session, 0).unwrap();
    assert_eq!(cold.candidates.len(), 2);
    assert!(session.environment.applied.is_empty());
    let calls = session.environment.leaf_calls;
    let hit = request.parse(&mut session, 0).unwrap();
    assert_eq!(cold.candidates, hit.candidates);
    assert_eq!(cold.failure, hit.failure);
    assert_eq!(session.environment.leaf_calls, calls);
    assert!(session.environment.applied.is_empty());
    session.select_expression(&hit.candidates[0].node).unwrap();
    assert_eq!(session.environment.applied, vec![10]);
    assert_eq!(
        *hit.candidates[1]
            .node
            .effects
            .as_ref()
            .unwrap()
            .downcast_ref::<u64>()
            .unwrap(),
        11
    );
    // Native selection forwards only the selected token. StateStore delta
    // idempotency and transaction ownership are separate host contracts.
}

fn section_frame() -> crate::SectionScopeFrame {
    crate::SectionScopeFrame {
        scope_id: 73,
        parent_scope_id: None,
        raw_node_id: crate::RawNodeId::new(9),
        kind: crate::SectionScopeKind::Loop,
        definition_id: "section:memo".into(),
        registration_id: "section:memo#0".into(),
        element_class: None,
        pattern_index: 0,
        pattern: "loop".into(),
        source: "loop".into(),
        addon_name: None,
        addon_version: None,
        owner_component_id: None,
        loop_section: true,
        effect_section: false,
        section_expression: false,
        captures: Vec::new(),
        metadata: BTreeMap::new(),
    }
}

#[test]
fn every_request_dependency_separates_memo_entries_in_one_session() {
    for case in [
        "range",
        "ends",
        "end-order",
        "type",
        "type-order",
        "plural",
        "literals",
        "expressions",
        "time",
        "syntax-context",
        "events",
        "sections",
        "values",
        "revision",
    ] {
        let source = MappedSource::identity("éx");
        let mut environment = Environment::default();
        let mut session = new_session(&source, &mut environment, ExpressionParserConfig::default());
        let mut base = Request::new(TextRange::new(0, 3));
        base.types = vec![
            ExpressionExpectedType {
                class_name: ClassName("java.lang.String".into()),
                plural: true,
            },
            ExpressionExpectedType {
                class_name: ClassName("java.lang.Number".into()),
                plural: true,
            },
        ];
        if case == "plural" {
            base.types.truncate(1);
        }
        if case == "end-order" {
            base.ends = vec![2, 3];
        }
        if case == "literals" {
            base.expressions = true;
        }
        let cold = base.parse(&mut session, 0).unwrap();
        assert!(!cold.candidates.is_empty(), "{case}");
        let calls = session.environment.leaf_calls;
        let mut changed = base.clone();
        match case {
            "range" => changed.range.start = 2,
            "ends" => changed.ends = vec![2, 3],
            "end-order" => changed.ends.reverse(),
            "type" => changed.types = vec![base.types[1].clone()],
            "type-order" => changed.types.reverse(),
            "plural" => changed.types[0].plural = false,
            "literals" => changed.literals = false,
            "expressions" => changed.expressions = true,
            "time" => changed.time = -1,
            "syntax-context" => session.context.syntax_context = 73,
            "events" => session.context.event_classes = vec![ClassName("memo.Event".into())],
            "sections" => session.context.section_stack.push(section_frame()),
            "values" => {
                session
                    .context
                    .values
                    .insert("memo".into(), "changed".into());
            }
            "revision" => session.environment.revision = 17,
            _ => unreachable!(),
        }
        let changed_result = changed.parse(&mut session, 0).unwrap();
        if case == "plural" {
            assert_eq!(changed_result.candidates.len(), 1);
            assert_eq!(
                changed_result.candidates[0].node.multiplicity,
                Some(Multiplicity::Single)
            );
        }
        assert!(
            session.environment.leaf_calls > calls,
            "{case} reused a stale memo entry"
        );
        let changed_calls = session.environment.leaf_calls;
        let hit = changed.parse(&mut session, 0).unwrap();
        assert_eq!(changed_result.candidates, hit.candidates, "{case}");
        assert_eq!(changed_result.failure, hit.failure, "{case}");
        assert_eq!(session.environment.leaf_calls, changed_calls, "{case}");
        session.replace_context(ExpressionParseContext::default());
        session.environment.revision = 0;
        let restored = base.parse(&mut session, 0).unwrap();
        assert_eq!(cold.candidates, restored.candidates, "{case}");
        assert_eq!(cold.failure, restored.failure, "{case}");
        assert_eq!(session.environment.leaf_calls, changed_calls, "{case}");
        assert!(session.active.is_empty(), "{case}");
    }
}

#[test]
fn list_mode_reuses_only_results_with_the_same_list_policy() {
    let source = MappedSource::identity("x and x");
    let mut environment = Environment {
        only_atoms: true,
        single_candidate: true,
        ..Default::default()
    };
    let mut session = new_session(&source, &mut environment, ExpressionParserConfig::default());
    let mut request = Request::new(TextRange::new(0, 7));
    request.types = vec![ExpressionExpectedType {
        class_name: ClassName("java.lang.String".into()),
        plural: true,
    }];
    request.expressions = true;
    request.lists = false;
    assert!(
        request
            .parse(&mut session, 0)
            .unwrap()
            .candidates
            .is_empty()
    );
    request.lists = true;
    let list = request.parse(&mut session, 0).unwrap();
    assert_eq!(list.candidates.len(), 1);
    let node = &list.candidates[0].node;
    assert!(matches!(node.kind, ExpressionNodeKind::List { .. }));
    assert_eq!(node.multiplicity, Some(Multiplicity::Multiple));
    assert_eq!(node.children.len(), 2);
    assert_eq!(node.children[0].span.local_range, TextRange::new(0, 1));
    assert_eq!(node.children[1].span.local_range, TextRange::new(6, 7));
    let calls = session.environment.leaf_calls;
    assert_eq!(
        request.parse(&mut session, 0).unwrap().candidates,
        list.candidates
    );
    request.lists = false;
    assert!(
        request
            .parse(&mut session, 0)
            .unwrap()
            .candidates
            .is_empty()
    );
    assert_eq!(session.environment.leaf_calls, calls);
    assert!(session.active.is_empty());
}

#[test]
fn fresh_sessions_do_not_inherit_source_results_or_deferred_effect_tokens() {
    let mut results = Vec::new();
    for text in ["x", "y"] {
        let source = MappedSource::identity(text);
        let mut environment = Environment {
            emit_effects: true,
            single_candidate: true,
            ..Default::default()
        };
        let mut session = new_session(&source, &mut environment, ExpressionParserConfig::default());
        assert!(session.memo.is_empty());
        let result = Request::new(TextRange::new(0, 1))
            .parse(&mut session, 0)
            .unwrap();
        assert_eq!(session.environment.leaf_calls, 1);
        assert_eq!(
            result.candidates[0]
                .node
                .metadata
                .get("source")
                .map(String::as_str),
            Some(text)
        );
        assert!(session.environment.applied.is_empty());
        results.push(result.candidates[0].node.clone());
    }
    assert_ne!(results[0].effects, results[1].effects);
    // This confirms fresh-session isolation; it is not used as evidence of a hit.
}

#[test]
fn memo_capacity_accepts_hits_at_the_limit_and_keeps_old_entries_after_a_miss() {
    for limit in [0, 1, 2] {
        let source = MappedSource::identity("x");
        let mut environment = Environment {
            single_candidate: true,
            ..Default::default()
        };
        let mut session = new_session(
            &source,
            &mut environment,
            ExpressionParserConfig {
                max_memo_entries: limit,
                ..Default::default()
            },
        );
        let request = Request::new(TextRange::new(0, 1));
        for index in 0..limit {
            session.context.syntax_context = index as u64;
            assert_eq!(request.parse(&mut session, 0).unwrap().candidates.len(), 1);
            let calls = session.environment.leaf_calls;
            request.parse(&mut session, 0).unwrap();
            assert_eq!(session.environment.leaf_calls, calls);
            assert_eq!(session.memo.len(), index + 1);
        }
        session.context.syntax_context = limit as u64;
        assert!(
            matches!(request.parse(&mut session, 0), Err(ExpressionParseError::MemoLimit { limit: actual }) if actual == limit)
        );
        assert!(session.active.is_empty());
        assert_eq!(session.memo.len(), limit);
        assert_eq!(session.environment.leaf_calls, limit);
        if limit > 0 {
            session.context.syntax_context = 0;
            assert_eq!(request.parse(&mut session, 0).unwrap().candidates.len(), 1);
            assert_eq!(session.environment.leaf_calls, limit);
        }
    }
}

#[test]
fn nested_parsing_keeps_memo_entries_within_the_configured_limit() {
    for limit in [0, 1, 2] {
        let source = MappedSource::identity("(x)");
        let mut environment = Environment {
            single_candidate: true,
            ..Default::default()
        };
        let mut session = new_session(
            &source,
            &mut environment,
            ExpressionParserConfig {
                max_memo_entries: limit,
                ..Default::default()
            },
        );
        let result = Request::new(TextRange::new(0, 3)).parse(&mut session, 0);
        if limit < 2 {
            assert!(
                matches!(result, Err(ExpressionParseError::MemoLimit { limit: actual }) if actual == limit),
                "limit={limit}, entries={}: {result:?}",
                session.memo.len()
            );
        } else {
            assert_eq!(result.unwrap().candidates.len(), 1);
        }
        assert!(
            session.memo.len() <= limit,
            "limit={limit}, entries={}",
            session.memo.len()
        );
        assert!(session.active.is_empty());
        if limit == 1 {
            // If this fixture ran the child before rejecting its parent, that
            // child's completed memo must remain a real hit. Reserving capacity
            // before any child work is also allowed by this test.
            let child_ran = session.environment.leaf_finishes.contains(&true);
            let calls = session.environment.leaf_calls;
            assert_eq!(
                Request::new(TextRange::new(1, 2))
                    .parse(&mut session, 0)
                    .unwrap()
                    .candidates
                    .len(),
                1
            );
            if child_ran {
                assert_eq!(
                    session.environment.leaf_calls, calls,
                    "the completed child memo was lost after its parent's limit error"
                );
            }
            assert!(session.memo.len() <= limit);
            assert!(session.active.is_empty());
        }
    }
}

#[test]
fn cancellation_after_a_parent_leaf_precedes_an_exhausted_memo_budget() {
    // Keep a real, nonempty parent prefix after the grouped child. The test does
    // not depend on dispatching leaf callbacks with no candidate boundaries.
    let source = MappedSource::identity("(x)y");
    let mut environment = Environment {
        single_candidate: true,
        cancel_after_outer_leaf: true,
        ..Default::default()
    };
    let mut session = new_session(
        &source,
        &mut environment,
        ExpressionParserConfig {
            max_memo_entries: 1,
            ..Default::default()
        },
    );
    let mut request = Request::new(TextRange::new(0, 4));
    request.ends = vec![3, 4];
    let error = request.parse(&mut session, 0).unwrap_err();
    assert!(crate::parse_cancellation(&error).is_some(), "{error:?}");
    assert!(session.environment.cancelled);
    assert!(session.environment.leaf_calls >= 2);
    assert!(session.memo.is_empty());
    assert!(session.active.is_empty());
    assert!(session.environment.applied.is_empty());
    session.environment.cancelled = false;
    session.environment.cancel_after_outer_leaf = false;
    assert_eq!(
        Request::new(TextRange::new(1, 2))
            .parse(&mut session, 0)
            .unwrap()
            .candidates
            .len(),
        1
    );
}

#[test]
fn candidate_budget_counts_work_across_misses_but_does_not_charge_cached_candidates() {
    for limit in [0, 1, 2] {
        let source = MappedSource::identity("x");
        let mut environment = Environment {
            single_candidate: true,
            ..Default::default()
        };
        let mut session = new_session(
            &source,
            &mut environment,
            ExpressionParserConfig {
                max_candidates: limit,
                ..Default::default()
            },
        );
        let request = Request::new(TextRange::new(0, 1));
        for index in 0..limit {
            session.context.syntax_context = index as u64;
            assert_eq!(request.parse(&mut session, 0).unwrap().candidates.len(), 1);
            let calls = session.environment.leaf_calls;
            let charged = session.candidates_seen;
            request.parse(&mut session, 0).unwrap();
            assert_eq!(session.environment.leaf_calls, calls);
            assert_eq!(
                session.candidates_seen, charged,
                "a hit must not spend candidate budget"
            );
        }
        session.context.syntax_context = limit as u64;
        for _ in 0..2 {
            assert!(
                matches!(request.parse(&mut session, 0), Err(ExpressionParseError::CandidateLimit { limit: actual }) if actual == limit)
            );
            assert!(session.active.is_empty());
            assert_eq!(session.memo.len(), limit);
        }
        assert_eq!(session.environment.leaf_calls, limit + 2);
        if limit > 0 {
            session.context.syntax_context = 0;
            request.parse(&mut session, 0).unwrap();
            assert_eq!(session.environment.leaf_calls, limit + 2);
        }
    }
}

#[test]
fn depth_guard_precedes_a_cached_entry_and_an_aborted_parent_keeps_the_child_memo() {
    let source = MappedSource::identity("(x)");
    let mut environment = Environment {
        single_candidate: true,
        ..Default::default()
    };
    let mut session = new_session(
        &source,
        &mut environment,
        ExpressionParserConfig {
            max_depth: 0,
            ..Default::default()
        },
    );
    let child = Request::new(TextRange::new(1, 2));
    assert_eq!(child.parse(&mut session, 0).unwrap().candidates.len(), 1);
    let calls = session.environment.leaf_calls;
    assert!(matches!(
        child.parse(&mut session, 1),
        Err(ExpressionParseError::DepthLimit { limit: 0 })
    ));
    assert!(matches!(
        Request::new(TextRange::new(0, 3)).parse(&mut session, 0),
        Err(ExpressionParseError::DepthLimit { limit: 0 })
    ));
    assert_eq!(session.environment.leaf_calls, calls);
    assert!(session.active.is_empty());
    assert_eq!(session.memo.len(), 1);
    child.parse(&mut session, 0).unwrap();
    assert_eq!(session.environment.leaf_calls, calls);
}

#[test]
fn revision_read_failure_cannot_hit_and_leaf_errors_do_not_become_negative_entries() {
    let source = MappedSource::identity("x");
    let mut environment = Environment {
        single_candidate: true,
        ..Default::default()
    };
    let mut session = new_session(&source, &mut environment, ExpressionParserConfig::default());
    let request = Request::new(TextRange::new(0, 1));
    let cold = request.parse(&mut session, 0).unwrap();
    let calls = session.environment.leaf_calls;
    session.environment.revision_error = true;
    assert!(
        matches!(request.parse(&mut session, 0), Err(ExpressionParseError::Environment { message }) if message == "memo revision unavailable")
    );
    assert_eq!(session.environment.leaf_calls, calls);
    assert!(session.active.is_empty());
    session.environment.revision_error = false;
    assert_eq!(
        request.parse(&mut session, 0).unwrap().candidates,
        cold.candidates
    );
    assert_eq!(session.environment.leaf_calls, calls);

    session.context.syntax_context = 73;
    session.environment.leaf_error = true;
    assert!(
        matches!(request.parse(&mut session, 0), Err(ExpressionParseError::Environment { message }) if message == "memo leaf failed")
    );
    assert!(session.active.is_empty());
    assert_eq!(
        session.memo.len(),
        1,
        "failed work must not replace the completed entry"
    );
    session.environment.leaf_error = false;
    assert_eq!(request.parse(&mut session, 0).unwrap().candidates.len(), 1);
    assert_eq!(session.environment.leaf_calls, calls + 2);
    assert!(session.active.is_empty());
}

#[test]
fn a_seed_growth_error_removes_the_partially_completed_memo_entry() {
    let source = MappedSource::identity("x in upper case");
    let mut environment = Environment {
        single_candidate: true,
        only_atoms: true,
        ..Default::default()
    };
    let mut session = ExpressionSession::new(
        source_catalog(),
        None,
        &source,
        &mut environment,
        ExpressionParseContext::default(),
        ExpressionParserConfig::default(),
    );
    let mut request = Request::new(TextRange::new(0, source.virtual_source().len()));
    request.expressions = true;
    let completed = request.parse(&mut session, 0).unwrap();
    let registered = completed
        .candidates
        .iter()
        .find(|candidate| {
            matches!(
                &candidate.node.kind,
                ExpressionNodeKind::Registered { element_class, pattern_index: 0, .. }
                    if element_class.as_str() == "ch.njol.skript.expressions.ExprStringCase"
            )
        })
        .expect("the real left-recursive Case Text registration must grow from x");
    assert_eq!(registered.node.children.len(), 1);
    assert_eq!(
        registered.node.children[0].span.local_range,
        TextRange::new(0, 1)
    );
    assert_eq!(
        registered.node.children[0].kind,
        ExpressionNodeKind::Literal {
            parser_id: "memo.primary".into()
        }
    );
    session.environment.revision = 17;
    session.environment.registered_error = true;
    let failed_key = MemoKey {
        range: request.range,
        candidate_ends: request.ends.clone(),
        expected_type_id: session.expected_type_id(&request.types),
        allow_literals: request.literals,
        allow_expressions: request.expressions,
        time: request.time,
        allow_lists: request.lists,
        context: session.context.clone(),
        state_revision: 17,
    };
    // This real left-recursive registration grows from a literal seed. Fault
    // the semantic resolver, without depending on empty matcher callbacks.
    assert!(
        matches!(request.parse(&mut session, 0), Err(ExpressionParseError::Environment { message }) if message == "memo registered resolver failed")
    );
    assert!(session.environment.leaf_calls > 0);
    assert!(session.environment.registered_calls > 0);
    assert!(!session.memo.contains_key(&failed_key));
    assert!(session.active.is_empty());
    session.environment.revision = 0;
    session.environment.registered_error = false;
    let calls = session.environment.leaf_calls;
    let hit = request.parse(&mut session, 0).unwrap();
    assert_eq!(hit.candidates, completed.candidates);
    assert_eq!(hit.failure, completed.failure);
    assert_eq!(session.environment.leaf_calls, calls);
}

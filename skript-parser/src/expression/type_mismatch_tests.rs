use super::*;
use std::sync::OnceLock;

fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4");
        let base = ssg::load(path).unwrap().into_catalog();
        Catalog::new(syntaxes::CatalogParts {
            syntaxes: base
                .syntaxes()
                .iter()
                .filter(|syntax| matches!(syntax, syntaxes::Syntax::Type(_)))
                .cloned()
                .collect(),
            converters: base.converters().to_vec(),
            comparators: Vec::new(),
            event_values: Vec::new(),
            properties: Vec::new(),
            operators: Vec::new(),
            operations: BTreeMap::new(),
            differences: Vec::new(),
            classes: base.classes().to_vec(),
            aliases: base.aliases().clone(),
            plural_rules: base.plural_rules().clone(),
            language: BTreeMap::new(),
        })
    })
}

#[derive(Default)]
struct Environment {
    leaf_calls: usize,
    registered_calls: usize,
    finishes: Vec<bool>,
    allow_regex: bool,
    cancelled: bool,
    nested_failure: Option<FailureTrace>,
    nested_failure_expected: Option<&'static str>,
    functions: Vec<crate::FunctionDefinition>,
    effect_count: usize,
    semantic_checkpoints: Vec<usize>,
    cancel_after_leaf: bool,
}

impl PatternMatchEnvironment for Environment {
    fn allows_regex_pattern(
        &mut self,
        _: crate::MatchSyntaxKind,
        _: &str,
        _: usize,
    ) -> Result<bool, String> {
        Ok(self.allow_regex)
    }
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
    fn lookup_functions(
        &mut self,
        request: crate::FunctionLookupRequest<'_>,
    ) -> Result<Vec<crate::FunctionDefinition>, String> {
        Ok(self
            .functions
            .iter()
            .filter(|definition| definition.name == request.name)
            .cloned()
            .collect())
    }
    fn begin_semantic_candidate(&mut self) -> Result<(), String> {
        self.semantic_checkpoints.push(self.effect_count);
        Ok(())
    }
    fn finish_semantic_candidate(&mut self, accepted: bool) -> Result<(), String> {
        let before = self
            .semantic_checkpoints
            .pop()
            .ok_or("unbalanced semantic probe")?;
        if !accepted {
            self.effect_count = before;
        }
        Ok(())
    }
    fn state_revision(&self) -> Result<u64, String> {
        Ok(0)
    }
    fn can_resolve_registered_expression(&self, _: RegisteredSyntaxIdentity<'_>) -> bool {
        true
    }
    fn resolve_registered_expression(
        &mut self,
        _: RegisteredExpressionRequest<'_>,
    ) -> Result<RegisteredExpressionDecision, String> {
        self.registered_calls += 1;
        Ok(RegisteredExpressionDecision::Resolved {
            return_type: Some(ClassName("java.lang.String".into())),
            possible_return_types: vec![ClassName("java.lang.String".into())],
            possible_return_types_state: PossibleReturnTypesState::Complete,
            multiplicity: Some(Multiplicity::Single),
            public_data: Vec::new(),
            metadata: BTreeMap::new(),
        })
    }
    fn parse_expression_leaf(
        &mut self,
        request: ExpressionLeafRequest<'_>,
    ) -> Result<ExpressionLeafParse, String> {
        self.leaf_calls += 1;
        assert!(
            !request.expected_types.is_empty(),
            "diagnostics must not reparse without the expected type"
        );
        if let Some(trace) = &self.nested_failure
            && self.nested_failure_expected.is_none_or(|expected| {
                request
                    .expected_types
                    .iter()
                    .any(|ty| ty.class_name.as_str() == expected)
            })
        {
            return Ok(ExpressionLeafParse {
                candidates: Vec::new(),
                failure: Some(trace.clone()),
            });
        }
        let Some((range, return_type)) = request.candidate_ends.iter().rev().find_map(|end| {
            let range = TextRange::new(request.remaining.start, *end);
            match range.slice(request.input) {
                Some("known text") => Some((range, "java.lang.String")),
                Some("player") => Some((range, "org.bukkit.entity.Player")),
                _ => None,
            }
        }) else {
            return Ok(ExpressionLeafParse::default());
        };
        self.effect_count += 1;
        if self.cancel_after_leaf {
            self.cancelled = true;
        }
        Ok(vec![ExpressionLeafCandidate {
            effects: None,
            parser_id: "test.known-text".into(),
            kind: ExpressionLeafKind::Literal,
            timing: ExpressionLeafTiming::BeforeRegistered,
            range,
            return_type: Some(ClassName(return_type.into())),
            multiplicity: Some(Multiplicity::Single),
            children: Vec::new(),
            function: None,
            public_data: Vec::new(),
            metadata: BTreeMap::new(),
        }]
        .into())
    }
    fn finish_expression_leaf(&mut self, accepted: bool) -> Result<(), String> {
        self.finishes.push(accepted);
        Ok(())
    }
}

fn expected_location() -> Vec<ExpressionExpectedType> {
    vec![ExpressionExpectedType {
        class_name: ClassName("org.bukkit.Location".into()),
        plural: false,
    }]
}

fn assert_mismatch(trace: &FailureTrace) {
    assert!(
        matches!(&trace.root_cause().failure.reasons[..], [PatternFailureReason::TypeMismatch { expected, actual }]
        if expected == &["org.bukkit.Location"] && actual == &["java.lang.String"])
    );
}

#[test]
fn type_mismatch_preserves_a_rejected_known_leaf_without_an_extra_parse() {
    let source = MappedSource::identity("known text");
    let mut environment = Environment::default();
    let result = parse_expression(
        catalog(),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, 10),
            expected_types: expected_location(),
            context: ExpressionParseContext::default(),
        },
        &mut environment,
        ExpressionParserConfig {
            root_mode: ExpressionRootMode::LiteralsOnly,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(result.selected.is_none());
    assert_mismatch(result.failure.as_ref().unwrap().trace.as_ref().unwrap());
    assert_eq!(environment.leaf_calls, 1);
    assert_eq!(environment.finishes, [false]);
    assert_eq!(environment.registered_calls, 0);
}

#[test]
fn type_mismatch_direct_resolution_keeps_slot_expectation_and_candidate_identity() {
    use syntaxes::{
        DynamicPattern, DynamicSyntaxDefinition, DynamicSyntaxId, RankedSyntaxCandidate,
        SyntaxCandidateSource,
    };
    let source = MappedSource::identity("resolved text");
    let id = DynamicSyntaxId::new("test.type-evidence", "text");
    let definition = DynamicSyntaxDefinition {
        id: id.clone(),
        kind: SyntaxKind::Expression,
        patterns: vec![DynamicPattern {
            source: "resolved text".into(),
            parsed: syntax_pattern_parser::syntax::parse("resolved text", catalog().plural_rules())
                .unwrap(),
        }],
        priority: 0,
        before: Vec::new(),
        after: Vec::new(),
        return_type: Some("java.lang.Object".into()),
        return_multiplicity: Some(DynamicMultiplicity::Single),
        structure_node_type: None,
        structure_body_mode: None,
        entry_validator: None,
        handler: "test.resolve".into(),
        metadata: BTreeMap::new(),
        component_load_order: 0,
        declaration_order: 0,
    };
    let snapshot = DynamicSyntaxSnapshot {
        document_id: "file:///evidence.sk".into(),
        document_revision: 1,
        registry_revision: 1,
        definitions: BTreeMap::from([(id.clone(), definition)]),
        overrides: BTreeMap::new(),
        candidates: vec![RankedSyntaxCandidate {
            source: SyntaxCandidateSource::Dynamic(id),
            kind: SyntaxKind::Expression,
            overrides: Vec::new(),
        }],
    };
    // An incomplete competing parser can resolve its own child at this range.
    // Its nested expectation must not replace this complete resolution's slot.
    let range = TextRange::new(0, source.virtual_source().len());
    let span = MatchSpan {
        local_range: range,
        mapped: source.map_range(range).unwrap(),
    };
    let speculative = FailureTrace::leaf(PatternFailure {
        span: span.clone(),
        reasons: vec![PatternFailureReason::TypeMismatch {
            expected: vec!["org.bukkit.entity.Entity".into()],
            actual: vec!["java.lang.String".into()],
        }],
    })
    .with_parent(FailureFrame {
        kind: crate::MatchSyntaxKind::Expression,
        definition_id: "test.inner-text".into(),
        registration_id: "test.inner-text:0".into(),
        pattern_index: 0,
        pattern: "resolved text".into(),
        element_path: Vec::new(),
        pattern_span: None,
        input_span: span.clone(),
        role: FailureFrameRole::SemanticCandidate,
    })
    .with_parent(FailureFrame {
        kind: crate::MatchSyntaxKind::Expression,
        definition_id: "test.speculative".into(),
        registration_id: "test.speculative:0".into(),
        pattern_index: 0,
        pattern: "%entities% later".into(),
        element_path: Vec::new(),
        pattern_span: None,
        input_span: span,
        role: FailureFrameRole::TypeExpressionCapture,
    });
    let mut environment = Environment {
        nested_failure: Some(speculative.clone()),
        ..Default::default()
    };
    let result = parse_expression_with_snapshot(
        catalog(),
        Some(&snapshot),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, 13),
            expected_types: expected_location(),
            context: ExpressionParseContext::default(),
        },
        &mut environment,
        ExpressionParserConfig::default(),
    )
    .unwrap();
    assert!(result.selected.is_none());
    let trace = result.failure.as_ref().unwrap().trace.as_ref().unwrap();
    assert_mismatch(trace);
    assert_eq!(
        choose_failure_trace(Some(speculative.clone()), Some(trace.clone())),
        Some(speculative),
        "the generic ranking previously kept this earlier same-span nested rejection"
    );
    assert_eq!(
        trace.frame.as_ref().unwrap().registration_id,
        "dynamic:test.type-evidence/text"
    );
    assert_eq!(
        trace.root_cause().failure.span.mapped.virtual_range,
        TextRange::new(0, 13)
    );
    assert_eq!(environment.registered_calls, 1);
}

#[test]
fn type_mismatch_requires_complete_known_types_and_respects_conversions() {
    let source = MappedSource::identity("value");
    let mut environment = Environment::default();
    let session = ExpressionSession::new(
        catalog(),
        None,
        &source,
        &mut environment,
        ExpressionParseContext::default(),
        ExpressionParserConfig::default(),
    );
    let string = ClassName("java.lang.String".into());
    let player = ClassName("org.bukkit.entity.Player".into());
    let object = ClassName("java.lang.Object".into());
    let span = session.map_range(TextRange::new(0, 5)).unwrap();
    assert!(catalog().can_convert(player.as_str(), "org.bukkit.Location"));
    assert!(!catalog().can_convert(string.as_str(), "org.bukkit.Location"));
    assert!(
        session
            .type_mismatch_trace(
                span.clone(),
                Some(&player),
                &[player.clone()],
                PossibleReturnTypesState::Complete,
                &expected_location()
            )
            .is_none()
    );
    for state in [
        PossibleReturnTypesState::Partial,
        PossibleReturnTypesState::Unresolved,
    ] {
        assert!(
            session
                .type_mismatch_trace(
                    span.clone(),
                    Some(&string),
                    &[string.clone()],
                    state,
                    &expected_location()
                )
                .is_none()
        );
    }
    assert!(
        session
            .type_mismatch_trace(
                span.clone(),
                Some(&object),
                &[object.clone()],
                PossibleReturnTypesState::Complete,
                &expected_location()
            )
            .is_none()
    );
    assert!(
        session
            .type_mismatch_trace(
                span.clone(),
                None,
                &[],
                PossibleReturnTypesState::Complete,
                &expected_location()
            )
            .is_none()
    );
    assert!(
        session
            .type_mismatch_trace(
                span.clone(),
                Some(&ClassName("unavailable.Type".into())),
                &[],
                PossibleReturnTypesState::Complete,
                &expected_location()
            )
            .is_none()
    );
    assert!(
        session
            .type_mismatch_trace(
                span,
                Some(&string),
                &[string.clone()],
                PossibleReturnTypesState::Complete,
                &[ExpressionExpectedType {
                    class_name: ClassName("unavailable.Expected".into()),
                    plural: false
                }]
            )
            .is_none()
    );
}

#[test]
fn type_mismatch_evidence_stays_contained_and_does_not_hide_later_input_errors() {
    let source = MappedSource::identity("teleport player to player's name extra");
    let mismatch = FailureTrace::leaf(PatternFailure {
        span: MatchSpan {
            local_range: TextRange::new(19, 32),
            mapped: source.map_range(TextRange::new(19, 32)).unwrap(),
        },
        reasons: vec![PatternFailureReason::TypeMismatch {
            expected: vec!["org.bukkit.Location".into()],
            actual: vec!["java.lang.String".into()],
        }],
    });
    let trailing = |range| {
        FailureTrace::leaf(PatternFailure {
            span: MatchSpan {
                local_range: range,
                mapped: source.map_range(range).unwrap(),
            },
            reasons: vec![PatternFailureReason::TrailingInput],
        })
    };
    assert_eq!(
        choose_failure_trace(
            Some(trailing(TextRange::new(25, 32))),
            Some(mismatch.clone())
        ),
        Some(mismatch.clone())
    );
    assert_eq!(
        choose_failure_trace(
            Some(mismatch.clone()),
            Some(trailing(TextRange::new(25, 32)))
        ),
        Some(mismatch.clone())
    );
    let later = trailing(TextRange::new(33, 38));
    assert_eq!(
        choose_failure_trace(Some(mismatch), Some(later.clone())),
        Some(later)
    );
}

#[test]
fn type_mismatch_from_a_resolved_property_reaches_its_containing_effect_capture() {
    use syntaxes::{
        DynamicPattern, DynamicSyntaxDefinition, DynamicSyntaxId, RankedSyntaxCandidate,
        SyntaxCandidateSource,
    };
    let definitions = [
        ("name", SyntaxKind::Expression, "%objects%'[s] name[s]"),
        (
            "teleport",
            SyntaxKind::Effect,
            "teleport %entities% to %location%",
        ),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (id, kind, pattern))| {
        let id = DynamicSyntaxId::new("test.type-evidence", id);
        let definition = DynamicSyntaxDefinition {
            id: id.clone(),
            kind,
            patterns: vec![DynamicPattern {
                source: pattern.into(),
                parsed: syntax_pattern_parser::syntax::parse(pattern, catalog().plural_rules())
                    .unwrap(),
            }],
            priority: 0,
            before: Vec::new(),
            after: Vec::new(),
            return_type: (kind == SyntaxKind::Expression).then(|| "java.lang.Object".into()),
            return_multiplicity: Some(DynamicMultiplicity::Single),
            structure_node_type: None,
            structure_body_mode: None,
            entry_validator: None,
            handler: "test.resolve".into(),
            metadata: BTreeMap::new(),
            component_load_order: 0,
            declaration_order: index as u64,
        };
        (id, definition)
    })
    .collect::<BTreeMap<_, _>>();
    let candidates = definitions
        .iter()
        .map(|(id, definition)| RankedSyntaxCandidate {
            source: SyntaxCandidateSource::Dynamic(id.clone()),
            kind: definition.kind,
            overrides: Vec::new(),
        })
        .collect();
    let snapshot = DynamicSyntaxSnapshot {
        document_id: "file:///evidence.sk".into(),
        document_revision: 1,
        registry_revision: 1,
        definitions,
        overrides: BTreeMap::new(),
        candidates,
    };
    let source = MappedSource::identity("teleport player to player's name");
    let tree = crate::parse_raw_tree(&source, crate::RawTreeOptions::for_skript_version(2, 15));
    let mut environment = Environment::default();
    let result = crate::parse_effect_with_snapshot(
        catalog(),
        Some(&snapshot),
        crate::EffectParseRequest {
            source: &source,
            node: tree.get(tree.roots[0]).unwrap(),
            context: ExpressionParseContext::default(),
        },
        &mut environment,
        crate::EffectParserConfig::default(),
    )
    .unwrap();
    assert!(result.selected.is_none());
    let unknown = result.unknown.unwrap();
    let trace = &unknown.failures.primary().unwrap().matched.trace;
    assert_mismatch(trace);
    assert_eq!(
        trace
            .root_cause()
            .failure
            .span
            .local_range
            .slice(source.virtual_source()),
        Some("player's name")
    );
    assert_eq!(
        trace.frame.as_ref().unwrap().registration_id,
        "dynamic:test.type-evidence/teleport"
    );
    assert!(environment.registered_calls > 0);
}

#[test]
fn type_mismatch_from_one_type_alternative_does_not_blame_an_accepted_union() {
    // The matching literal makes `known text` a legal capture boundary.
    // A missing anchor would skip the leaf entirely and make this test vacuous.
    let source = MappedSource::identity("display known text now nope");
    let pattern = "display %location/string% now <[0-9]+>";
    let parsed = syntax_pattern_parser::syntax::parse(pattern, catalog().plural_rules()).unwrap();
    let candidate = PatternCandidate {
        kind: crate::MatchSyntaxKind::Effect,
        definition_id: "test.union".into(),
        registration_id: "test.union:0".into(),
        priority: 0,
        registration_order: 0,
        resolved_order: None,
        patterns: vec![MatchPattern {
            pattern_index: 0,
            source: pattern,
            parsed: &parsed,
        }],
    };
    // Regex matching is an explicit environment capability, disabled by default.
    let mut environment = Environment {
        allow_regex: true,
        ..Default::default()
    };
    let mut session = ExpressionSession::new(
        catalog(),
        None,
        &source,
        &mut environment,
        ExpressionParseContext::default(),
        ExpressionParserConfig::default(),
    );
    let result = session
        .match_candidates_at_depth(
            TextRange::new(0, source.virtual_source().len()),
            &[candidate],
            0,
        )
        .unwrap();
    assert!(
        result.selected.is_none(),
        "the later regex rejects the whole syntax"
    );
    let failure = result
        .primary_failure()
        .expect("enabled native matcher must report the later regex failure");
    assert!(
        !failure
            .root_cause()
            .failure
            .reasons
            .iter()
            .any(|reason| matches!(reason, PatternFailureReason::TypeMismatch { .. })),
        "String was accepted by another type alternative: {failure:?}"
    );
    assert!(
        failure.root_cause().failure.reasons.iter().any(|reason| {
            matches!(reason, PatternFailureReason::Regex { pattern } if pattern == "[0-9]+")
        }),
        "the later regex failure must survive: {failure:?}"
    );
    assert_eq!(
        failure.root_cause().failure.span.mapped.virtual_range.start,
        source.virtual_source().find("nope").unwrap()
    );
    // This callback certifies native leaf validation, not whole-pattern commit.
    assert!(environment.finishes.contains(&true));
}

#[test]
fn type_mismatch_for_a_rejected_union_lists_all_known_expectations() {
    for (grammar, expected) in [
        (
            "%location/entity%",
            Some(vec!["org.bukkit.Location", "org.bukkit.entity.Entity"]),
        ),
        ("%location/unavailabletype%", None),
    ] {
        let source = MappedSource::identity("known text");
        let parsed =
            syntax_pattern_parser::syntax::parse(grammar, catalog().plural_rules()).unwrap();
        let PatternElement::TypeExpr(expression) = &parsed.elements[0].value else {
            panic!("typed capture");
        };
        let mut environment = Environment::default();
        let mut session = ExpressionSession::new(
            catalog(),
            None,
            &source,
            &mut environment,
            ExpressionParseContext::default(),
            ExpressionParserConfig::default(),
        );
        session.frame_starts.push(0);
        session.frame_depths.push(0);
        let result = session
            .resolve_type_inner(TypeExpressionRequest {
                input: source.virtual_source(),
                expression,
                pattern_span: parsed.elements[0].span,
                remaining: TextRange::new(0, 10),
                candidate_ends: &[10],
            })
            .unwrap();
        assert!(result.resolutions.is_empty());
        if let Some(expected) = expected {
            assert!(
                matches!(&result.failure.unwrap().root_cause().failure.reasons[..], [PatternFailureReason::TypeMismatch { expected: found, actual }]
                if found == &expected && actual == &["java.lang.String"])
            );
        } else {
            assert!(
                result.failure.is_none(),
                "unknown alternative is not a definite type incompatibility"
            );
        }
    }
}

#[test]
fn type_mismatch_union_prefers_direct_later_alternative_over_nested_speculation() {
    let source = MappedSource::identity("known text");
    let range = TextRange::new(0, source.virtual_source().len());
    let span = MatchSpan {
        local_range: range,
        mapped: source.map_range(range).unwrap(),
    };
    let capture = FailureFrame {
        kind: crate::MatchSyntaxKind::Expression,
        definition_id: "test.speculative".into(),
        registration_id: "test.speculative:0".into(),
        pattern_index: 0,
        pattern: "%entities% later".into(),
        element_path: Vec::new(),
        pattern_span: None,
        input_span: span.clone(),
        role: FailureFrameRole::TypeExpressionCapture,
    };
    let mut semantic = capture.clone();
    semantic.role = FailureFrameRole::SemanticCandidate;
    let nested = FailureTrace::leaf(PatternFailure {
        span,
        reasons: vec![PatternFailureReason::TypeMismatch {
            expected: vec!["org.bukkit.entity.Entity".into()],
            actual: vec!["java.lang.String".into()],
        }],
    })
    .with_parent(semantic)
    .with_parent(capture);
    let parsed =
        syntax_pattern_parser::syntax::parse("%location/number%", catalog().plural_rules())
            .unwrap();
    let PatternElement::TypeExpr(expression) = &parsed.elements[0].value else {
        panic!("typed capture");
    };
    let mut environment = Environment {
        nested_failure: Some(nested),
        nested_failure_expected: Some("org.bukkit.Location"),
        ..Default::default()
    };
    let mut session = ExpressionSession::new(
        catalog(),
        None,
        &source,
        &mut environment,
        ExpressionParseContext::default(),
        ExpressionParserConfig::default(),
    );
    session.frame_starts.push(0);
    session.frame_depths.push(0);
    let result = session
        .resolve_type_inner(TypeExpressionRequest {
            input: source.virtual_source(),
            expression,
            pattern_span: parsed.elements[0].span,
            remaining: range,
            candidate_ends: &[range.end],
        })
        .unwrap();
    assert!(result.resolutions.is_empty());
    let trace = result.failure.unwrap();
    assert!(matches!(
        &trace.root_cause().failure.reasons[..],
        [PatternFailureReason::TypeMismatch { expected, actual }]
            if expected == &["org.bukkit.Location", "java.lang.Number"]
                && actual == &["java.lang.String"]
    ));
    assert_eq!(trace.root_cause().failure.span.mapped.virtual_range, range);
    assert_eq!(environment.leaf_calls, 2);
}

#[test]
fn type_mismatch_in_a_nested_capture_keeps_its_own_expected_type() {
    let source = MappedSource::identity("nested value");
    let range = TextRange::new(0, source.virtual_source().len());
    let span = MatchSpan {
        local_range: range,
        mapped: source.map_range(range).unwrap(),
    };
    let nested = FailureTrace::leaf(PatternFailure {
        span: span.clone(),
        reasons: vec![PatternFailureReason::TypeMismatch {
            expected: vec!["org.bukkit.entity.Entity".into()],
            actual: vec!["java.lang.String".into()],
        }],
    })
    .with_parent(FailureFrame {
        kind: crate::MatchSyntaxKind::Expression,
        definition_id: "test.nested".into(),
        registration_id: "test.nested:0".into(),
        pattern_index: 0,
        pattern: "nested %entity%".into(),
        element_path: Vec::new(),
        pattern_span: None,
        input_span: span,
        role: FailureFrameRole::TypeExpressionCapture,
    });
    let parsed =
        syntax_pattern_parser::syntax::parse("%location/number%", catalog().plural_rules())
            .unwrap();
    let PatternElement::TypeExpr(expression) = &parsed.elements[0].value else {
        panic!("typed capture");
    };
    let mut environment = Environment {
        nested_failure: Some(nested),
        ..Default::default()
    };
    let mut session = ExpressionSession::new(
        catalog(),
        None,
        &source,
        &mut environment,
        ExpressionParseContext::default(),
        ExpressionParserConfig::default(),
    );
    session.frame_starts.push(0);
    session.frame_depths.push(0);
    let result = session
        .resolve_type_inner(TypeExpressionRequest {
            input: source.virtual_source(),
            expression,
            pattern_span: parsed.elements[0].span,
            remaining: range,
            candidate_ends: &[range.end],
        })
        .unwrap();
    assert!(result.resolutions.is_empty());
    let trace = result.failure.unwrap();
    assert!(
        matches!(&trace.root_cause().failure.reasons[..], [PatternFailureReason::TypeMismatch { expected, actual }]
        if expected == &["org.bukkit.entity.Entity"] && actual == &["java.lang.String"])
    );
}

fn diagnostic_function(
    name: &str,
    parameter: Option<&str>,
    returns: Option<&str>,
) -> crate::FunctionDefinition {
    crate::FunctionDefinition {
        parser_id: "test.function".into(),
        name: name.into(),
        definition_id: format!("function:{name}:{parameter:?}:{returns:?}"),
        registration_id: format!("function:{name}:{parameter:?}:{returns:?}:0"),
        registration_order: 0,
        return_type: returns.map(|class| ClassName(class.into())),
        return_type_is_single: true,
        parameters: parameter
            .map(|class| {
                vec![crate::FunctionParameterDefinition {
                    name: "value".into(),
                    parameter_type: ClassName(class.into()),
                    single: true,
                    optional: false,
                }]
            })
            .unwrap_or_default(),
        metadata: BTreeMap::new(),
    }
}

#[test]
fn type_mismatch_function_returns_use_typed_binding_without_adopting_probe_effects() {
    for text in ["typed(known text)", "(typed(known text))"] {
        let source = MappedSource::identity(text);
        let mut environment = Environment {
            functions: vec![diagnostic_function(
                "typed",
                Some("java.lang.String"),
                Some("java.lang.String"),
            )],
            ..Default::default()
        };
        let parsed = parse_expression(
            catalog(),
            ExpressionParseRequest {
                source: &source,
                range: TextRange::new(0, text.len()),
                expected_types: expected_location(),
                context: Default::default(),
            },
            &mut environment,
            Default::default(),
        )
        .unwrap();
        assert!(parsed.selected.is_none());
        let trace = parsed.failure.as_ref().unwrap().trace.as_ref().unwrap();
        assert_mismatch(trace);
        assert_eq!(
            trace.root_cause().failure.span.local_range.slice(text),
            Some("typed(known text)")
        );
        assert_eq!(
            environment.effect_count, 0,
            "diagnostic argument initialization must roll back"
        );
        assert!(environment.semantic_checkpoints.is_empty());
    }
}

#[test]
fn type_mismatch_function_probe_keeps_ambiguity_unknown_plurality_and_bad_arguments_separate() {
    let mut plural = diagnostic_function(
        "typed",
        Some("org.bukkit.entity.LivingEntity"),
        Some("org.bukkit.Location"),
    );
    plural.return_type_is_single = false;
    let scenarios = [
        (
            "ambiguous",
            vec![
                diagnostic_function(
                    "typed",
                    Some("org.bukkit.entity.Entity"),
                    Some("java.lang.String"),
                ),
                diagnostic_function("typed", Some("java.lang.Object"), Some("java.lang.Number")),
            ],
            "typed(player)",
        ),
        (
            "multiplicity competitor",
            vec![
                diagnostic_function(
                    "typed",
                    Some("org.bukkit.entity.Entity"),
                    Some("java.lang.String"),
                ),
                plural,
            ],
            "typed(player)",
        ),
        (
            "unknown result",
            vec![diagnostic_function(
                "typed",
                Some("java.lang.String"),
                Some("java.lang.Object"),
            )],
            "typed(known text)",
        ),
        (
            "void",
            vec![diagnostic_function("typed", None, None)],
            "typed()",
        ),
        (
            "wrong arguments",
            vec![diagnostic_function(
                "typed",
                Some("org.bukkit.entity.Player"),
                Some("java.lang.String"),
            )],
            "typed(known text)",
        ),
        (
            "unfinished call",
            vec![diagnostic_function("typed", None, Some("java.lang.String"))],
            "typed(",
        ),
    ];
    for (case, functions, text) in scenarios {
        let source = MappedSource::identity(text);
        let mut environment = Environment {
            functions,
            ..Default::default()
        };
        let result = parse_expression(
            catalog(),
            ExpressionParseRequest {
                source: &source,
                range: TextRange::new(0, text.len()),
                expected_types: expected_location(),
                context: Default::default(),
            },
            &mut environment,
            Default::default(),
        )
        .unwrap();
        assert!(result.selected.is_none(), "{case}");
        if let Some(trace) = result.failure.and_then(|failure| failure.trace) {
            assert!(
                !trace
                    .root_cause()
                    .failure
                    .reasons
                    .iter()
                    .any(|reason| matches!(reason, PatternFailureReason::TypeMismatch { .. })),
                "{case}: {trace:?}"
            );
        }
        assert_eq!(environment.effect_count, 0, "{case}");
        assert!(environment.semantic_checkpoints.is_empty(), "{case}");
    }
}

#[test]
fn type_mismatch_function_probe_does_not_change_compatible_selection_and_cleans_up_on_cancel() {
    let text = "typed(known text)";
    let source = MappedSource::identity(text);
    let definition = diagnostic_function(
        "typed",
        Some("java.lang.String"),
        Some("org.bukkit.Location"),
    );
    let mut environment = Environment {
        functions: vec![definition],
        ..Default::default()
    };
    let result = parse_expression(
        catalog(),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: expected_location(),
            context: Default::default(),
        },
        &mut environment,
        Default::default(),
    )
    .unwrap();
    assert!(result.selected.is_some());
    assert!(result.failure.is_none());
    let mut environment = Environment {
        functions: vec![diagnostic_function(
            "typed",
            Some("java.lang.String"),
            Some("java.lang.String"),
        )],
        cancel_after_leaf: true,
        ..Default::default()
    };
    let error = parse_expression(
        catalog(),
        ExpressionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            expected_types: expected_location(),
            context: Default::default(),
        },
        &mut environment,
        Default::default(),
    )
    .unwrap_err();
    assert!(crate::parse_cancellation(&error).is_some());
    assert_eq!(environment.effect_count, 0);
    assert!(environment.semantic_checkpoints.is_empty());
}

#[test]
fn type_mismatch_function_parameter_does_not_inherit_outer_union() {
    let text = "typed(known text)";
    let source = MappedSource::identity(text);
    let range = TextRange::new(0, text.len());
    let pattern =
        syntax_pattern_parser::syntax::parse("%location/entity%", catalog().plural_rules())
            .unwrap();
    let PatternElement::TypeExpr(expression) = &pattern.elements[0].value else {
        panic!("typed capture");
    };
    let mut environment = Environment {
        functions: vec![diagnostic_function(
            "typed",
            Some("java.lang.Number"),
            Some("org.bukkit.Location"),
        )],
        ..Default::default()
    };
    // Use the actual typed-slot resolver: the public parse_expression entry
    // alone does not run the outer-union expected-type rewriting being tested.
    let result = {
        let mut session = ExpressionSession::new(
            catalog(),
            None,
            &source,
            &mut environment,
            ExpressionParseContext::default(),
            ExpressionParserConfig::default(),
        );
        session.frame_starts.push(0);
        session.frame_depths.push(0);
        session
            .resolve_type_inner(TypeExpressionRequest {
                input: source.virtual_source(),
                expression,
                pattern_span: pattern.elements[0].span,
                remaining: range,
                candidate_ends: &[range.end],
            })
            .unwrap()
    };
    assert!(result.resolutions.is_empty());
    let trace = result
        .failure
        .expect("the Number parameter must reject String");
    let cause = trace.root_cause();
    assert!(
        cause.failure.reasons.iter().any(|reason| matches!(
            reason,
            PatternFailureReason::TypeMismatch { expected, actual }
                if expected == &vec!["java.lang.Number".to_owned()]
                    && actual == &vec!["java.lang.String".to_owned()]
        )),
        "{trace:#?}"
    );
    assert_eq!(cause.failure.span.local_range, TextRange::new(6, 16));
    assert_eq!(environment.effect_count, 0);
    assert!(environment.semantic_checkpoints.is_empty());
}

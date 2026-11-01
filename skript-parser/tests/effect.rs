use skript_parser::{
    EffectParseRequest, EffectParserConfig, EffectSemanticDecision, EffectSemanticRequest,
    ExpressionEffects, ExpressionLeafCandidate, ExpressionLeafKind, ExpressionLeafParse, ExpressionLeafRequest,
    ExpressionLeafTiming, ExpressionParseContext, ExpressionParseEnvironment, FailureTrace,
    MappedSource, MatchSyntaxKind, NoopExpressionEnvironment, ParsedCaptureValue,
    PatternFailureReason, PatternHookControl, PatternHookEvent, PatternHookScope,
    PatternHookTiming, PatternMatchEnvironment, RawTreeOptions, RegisteredCaptureBinding,
    RegisteredSyntaxIdentity, TextRange, TypeExpressionOutcome, TypeExpressionRequest,
    parse_effect, parse_effect_with_snapshot, parse_raw_tree,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use syntaxes::{
    Catalog, CatalogParts, ClassName, DynamicPattern, DynamicSyntaxDefinition, DynamicSyntaxId,
    DynamicSyntaxSnapshot, Multiplicity, RankedSyntaxCandidate, Syntax, SyntaxCandidateSource,
    SyntaxKind,
};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn effect_fixture() -> Catalog {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let source = snapshot.catalog();
    let syntaxes = source
        .syntaxes()
        .iter()
        .filter(|syntax| match syntax {
            Syntax::Type(value) => matches!(value.code_name.as_str(), "string" | "object"),
            Syntax::Effect(value) => value.common.patterns.iter().any(|pattern| {
                matches!(
                    pattern.source.as_str(),
                    "dummy effect registered through wrapper"
                        | "run dummy fixture effect [with %-string%]"
                )
            }),
            _ => false,
        })
        .cloned()
        .collect();
    catalog_with_syntaxes(source, syntaxes)
}

fn catalog_with_syntaxes(source: &Catalog, syntaxes: Vec<Syntax>) -> Catalog {
    Catalog::new(CatalogParts {
        syntaxes,
        converters: Vec::new(),
        comparators: Vec::new(),
        event_values: Vec::new(),
        properties: Vec::new(),
        operators: Vec::new(),
        operations: BTreeMap::new(),
        differences: Vec::new(),
        classes: Vec::new(),
        aliases: source.aliases().clone(),
        plural_rules: source.plural_rules().clone(),
        language: source
            .language_entries()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect(),
    })
}

fn effect_section_fixture(effect_section: bool, include_effect: bool) -> Catalog {
    let snapshot = ssg::load(fixture()).expect("schema 3 fixture must load");
    let source = snapshot.catalog();
    let effect = source
        .effects()
        .find(|effect| {
            effect
                .common
                .patterns
                .iter()
                .any(|pattern| pattern.source == "dummy effect registered through wrapper")
        })
        .expect("dummy Effect exists")
        .clone();
    let mut section = source
        .sections()
        .find(|section| {
            section
                .common
                .element_class
                .as_str()
                .ends_with(".EffSecShoot")
        })
        .expect("Skript EffectSection exists")
        .clone();
    section.common.patterns = effect.common.patterns.clone();
    section.effect_section = effect_section;

    let mut syntaxes = vec![Syntax::Section(section)];
    if include_effect {
        syntaxes.push(Syntax::Effect(effect));
    }
    catalog_with_syntaxes(source, syntaxes)
}

fn simple_node(source: &MappedSource) -> skript_parser::RawNode {
    let tree = parse_raw_tree(source, RawTreeOptions::for_skript_version(2, 15));
    tree.get(tree.roots[0]).expect("one root node").clone()
}

fn event_context(event_classes: &[&str]) -> ExpressionParseContext {
    ExpressionParseContext {
        event_classes: event_classes
            .iter()
            .map(|event| syntaxes::ClassName((*event).to_owned()))
            .collect(),
        ..ExpressionParseContext::default()
    }
}

fn event_restriction_reason(trace: &FailureTrace) -> Option<(&[String], &[String])> {
    if let Some(PatternFailureReason::EventRestricted { supported, current }) = trace
        .failure
        .reasons
        .iter()
        .find(|reason| matches!(reason, PatternFailureReason::EventRestricted { .. }))
    {
        return Some((supported.as_slice(), current.as_slice()));
    }
    trace.cause.as_deref().and_then(event_restriction_reason)
}

#[test]
fn parses_real_effect_without_placeholders_and_ignores_trailing_comment() {
    let catalog = effect_fixture();
    let source = MappedSource::identity(
        "dummy effect registered through wrapper # retained trailing comment",
    );
    let node = simple_node(&source);
    let result = parse_effect(
        &catalog,
        EffectParseRequest {
            source: &source,
            node: &node,
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        EffectParserConfig::default(),
    )
    .expect("fixture Effect must parse");

    let selected = result.selected.expect("Effect must be selected");
    assert_eq!(
        selected.matched.pattern,
        "dummy effect registered through wrapper"
    );
    assert!(
        selected
            .matched
            .registration_id
            .starts_with("effect:skriptdummyaddon:")
    );
    assert!(selected.parsed_captures.is_empty());
    assert!(result.unknown.is_none());
}

#[test]
fn effect_section_precedes_an_ordinary_effect_with_the_same_pattern() {
    let catalog = effect_section_fixture(true, true);
    let source = MappedSource::identity("dummy effect registered through wrapper");
    let node = simple_node(&source);
    let result = parse_effect(
        &catalog,
        EffectParseRequest {
            source: &source,
            node: &node,
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        EffectParserConfig::default(),
    )
    .expect("EffectSection must parse as an Effect");

    let selected = result.selected.expect("EffectSection must be selected");
    assert_eq!(selected.matched.kind, MatchSyntaxKind::Section);
    assert!(
        selected
            .matched
            .definition_id
            .starts_with("section:skript:")
    );
    assert!(
        selected
            .matched
            .registration_id
            .starts_with("section:skript:")
    );
    assert_eq!(
        selected.matched.pattern,
        "dummy effect registered through wrapper"
    );
}

#[test]
fn ordinary_sections_do_not_become_effect_candidates() {
    let catalog = effect_section_fixture(false, false);
    let source = MappedSource::identity("dummy effect registered through wrapper");
    let node = simple_node(&source);
    let result = parse_effect(
        &catalog,
        EffectParseRequest {
            source: &source,
            node: &node,
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        EffectParserConfig::default(),
    )
    .expect("ordinary Section exclusion is a valid parse result");

    assert!(result.selected.is_none());
    assert!(result.unknown.is_some());
}

#[derive(Default)]
struct RejectEffectEnvironment;

impl PatternMatchEnvironment for RejectEffectEnvironment {
    fn resolve_type(
        &mut self,
        _request: TypeExpressionRequest<'_>,
    ) -> Result<TypeExpressionOutcome, String> {
        Ok(TypeExpressionOutcome::default())
    }

    fn dispatch_hook(
        &mut self,
        _event: PatternHookEvent<'_>,
    ) -> Result<PatternHookControl, String> {
        Ok(PatternHookControl::Continue)
    }
}

impl ExpressionParseEnvironment for RejectEffectEnvironment {
    fn parse_expression_leaf(
        &mut self,
        _request: ExpressionLeafRequest<'_>,
    ) -> Result<ExpressionLeafParse, String> {
        Ok(ExpressionLeafParse::default())
    }

    fn resolve_effect_candidate(
        &mut self,
        request: EffectSemanticRequest<'_>,
    ) -> Result<EffectSemanticDecision, String> {
        assert_eq!(request.input, "dummy effect registered through wrapper");
        Ok(EffectSemanticDecision::Reject {
            reason: "rejected by test semantics".to_owned(),
            diagnostics: Vec::new(),
        })
    }

    fn state_revision(&self) -> Result<u64, String> {
        Ok(0)
    }
}

#[test]
fn semantic_environment_can_reject_a_structurally_matched_effect() {
    let catalog = effect_fixture();
    let source = MappedSource::identity("dummy effect registered through wrapper");
    let node = simple_node(&source);
    let result = parse_effect(
        &catalog,
        EffectParseRequest {
            source: &source,
            node: &node,
            context: ExpressionParseContext::default(),
        },
        &mut RejectEffectEnvironment,
        EffectParserConfig::default(),
    )
    .expect("semantic rejection is a recoverable parse result");

    assert!(result.selected.is_none());
    let unknown = result
        .unknown
        .expect("rejected Effect must remain diagnosable");
    let failure = unknown
        .failures
        .primary()
        .expect("semantic rejection must be ranked");
    assert!(failure.matched.trace.root_cause().failure.reasons.iter().any(
        |reason| matches!(reason, PatternFailureReason::HookRejected { reason } if reason == "rejected by test semantics")
    ));
}

#[derive(Default)]
struct StringLiteralEnvironment;

impl PatternMatchEnvironment for StringLiteralEnvironment {
    fn resolve_type(
        &mut self,
        _request: TypeExpressionRequest<'_>,
    ) -> Result<TypeExpressionOutcome, String> {
        Ok(TypeExpressionOutcome::default())
    }

    fn dispatch_hook(
        &mut self,
        _event: PatternHookEvent<'_>,
    ) -> Result<PatternHookControl, String> {
        Ok(PatternHookControl::Continue)
    }
}

impl ExpressionParseEnvironment for StringLiteralEnvironment {
    fn parse_expression_leaf(
        &mut self,
        request: ExpressionLeafRequest<'_>,
    ) -> Result<ExpressionLeafParse, String> {
        let Some(remaining) = request.remaining.slice(request.input) else {
            return Ok(ExpressionLeafParse::default());
        };
        if !remaining.starts_with('"') {
            return Ok(ExpressionLeafParse::default());
        }
        Ok(request
            .candidate_ends
            .iter()
            .copied()
            .filter(|end| {
                TextRange::new(request.remaining.start, *end)
                    .slice(request.input)
                    .is_some_and(|value| value.len() >= 2 && value.ends_with('"'))
            })
            .map(|end| ExpressionLeafCandidate {
                function: None,
                effects: None,
                parser_id: "test.string-literal".to_owned(),
                kind: ExpressionLeafKind::Literal,
                timing: ExpressionLeafTiming::AfterRegistered,
                range: TextRange::new(request.remaining.start, end),
                return_type: Some(ClassName("java.lang.String".to_owned())),
                multiplicity: Some(Multiplicity::Single),
                children: Vec::new(),
                public_data: Vec::new(),
                metadata: BTreeMap::new(),
            })
            .collect::<Vec<_>>()
            .into())
    }

    fn state_revision(&self) -> Result<u64, String> {
        Ok(0)
    }
}

#[test]
fn parses_expression_placeholder_with_the_shared_recursive_session() {
    let catalog = effect_fixture();
    let source = MappedSource::identity("run dummy fixture effect with \"metadata\"");
    let node = simple_node(&source);
    let result = parse_effect(
        &catalog,
        EffectParseRequest {
            source: &source,
            node: &node,
            context: ExpressionParseContext::default(),
        },
        &mut StringLiteralEnvironment,
        EffectParserConfig::default(),
    )
    .expect("Effect with a string Expression must parse");

    let selected = result.selected.expect("Effect must be selected");
    assert_eq!(
        selected.matched.pattern,
        "run dummy fixture effect [with %-string%]"
    );
    assert_eq!(selected.parsed_captures.len(), 1);
    let Some(ParsedCaptureValue::Expression(expression)) =
        selected.parsed_captures[0].result.value.as_ref()
    else {
        panic!("typed Effect capture must retain its Expression value");
    };
    assert_eq!(
        expression.span.local_range.slice(source.virtual_source()),
        Some("\"metadata\"")
    );
}

#[test]
fn parses_dynamic_effect_and_retains_its_handler_metadata() {
    let catalog = effect_fixture();
    let id = DynamicSyntaxId::new("test.dynamic", "effect");
    let pattern = "invoke dynamic %string%";
    let definition = DynamicSyntaxDefinition {
        id: id.clone(),
        kind: SyntaxKind::Effect,
        patterns: vec![DynamicPattern {
            source: pattern.to_owned(),
            parsed: syntax_pattern_parser::syntax::parse(pattern, catalog.plural_rules()).unwrap(),
        }],
        priority: -10,
        before: Vec::new(),
        after: Vec::new(),
        return_type: None,
        return_multiplicity: None,
        structure_node_type: None,
        structure_body_mode: None,
        entry_validator: None,
        handler: "effect.handle".to_owned(),
        metadata: BTreeMap::from([("owner".to_owned(), "fixture".to_owned())]),
        component_load_order: 1,
        declaration_order: 0,
    };
    let dynamic = DynamicSyntaxSnapshot {
        document_id: "file:///workspace/test.sk".to_owned(),
        document_revision: 1,
        registry_revision: 1,
        definitions: BTreeMap::from([(id.clone(), definition)]),
        overrides: BTreeMap::new(),
        candidates: vec![RankedSyntaxCandidate {
            source: SyntaxCandidateSource::Dynamic(id),
            kind: SyntaxKind::Effect,
            overrides: Vec::new(),
        }],
    };
    let source = MappedSource::identity("invoke dynamic \"value\"");
    let node = simple_node(&source);
    let result = parse_effect_with_snapshot(
        &catalog,
        Some(&dynamic),
        EffectParseRequest {
            source: &source,
            node: &node,
            context: ExpressionParseContext::default(),
        },
        &mut StringLiteralEnvironment,
        EffectParserConfig::default(),
    )
    .expect("dynamic Effect must parse");

    let selected = result.selected.expect("dynamic Effect must be selected");
    assert_eq!(
        selected.matched.registration_id,
        "dynamic:test.dynamic/effect"
    );
    assert_eq!(selected.handler.as_deref(), Some("effect.handle"));
    assert_eq!(
        selected.metadata.get("owner").map(String::as_str),
        Some("fixture")
    );
}

#[derive(Default)]
struct ScopedExpressionCaptureEnvironment {
    nested_context_seen: bool,
    nested_parent_context_seen: bool,
    effect_context_leaked: bool,
    effect_parent_context_seen: bool,
    adopted: Vec<usize>,
    checkpoints: Vec<Vec<usize>>,
    reject_effect: bool,
    fail_effect: bool,
    cancel_after_adoption: bool,
    cancelled: bool,
}

impl PatternMatchEnvironment for ScopedExpressionCaptureEnvironment {
    fn cancellation_requested(&mut self) -> bool { self.cancelled }
    fn allows_regex_pattern(
        &mut self,
        _kind: MatchSyntaxKind,
        _registration_id: &str,
        _pattern_index: usize,
    ) -> Result<bool, String> {
        Ok(true)
    }

    fn resolve_type(
        &mut self,
        _request: TypeExpressionRequest<'_>,
    ) -> Result<TypeExpressionOutcome, String> {
        Ok(TypeExpressionOutcome::default())
    }

    fn dispatch_hook(
        &mut self,
        _event: PatternHookEvent<'_>,
    ) -> Result<PatternHookControl, String> {
        Ok(PatternHookControl::Continue)
    }
}

impl ExpressionParseEnvironment for ScopedExpressionCaptureEnvironment {
    fn parse_expression_leaf(
        &mut self,
        request: ExpressionLeafRequest<'_>,
    ) -> Result<ExpressionLeafParse, String> {
        let local_context = request
            .context
            .values
            .get("fixture.input-source")
            .is_some_and(|value| value == "true");
        let candidates = request
            .candidate_ends
            .iter()
            .filter_map(|end| {
                let range = TextRange::new(request.remaining.start, *end);
                let text = range.slice(request.input)?;
                let (parser_id, kind, return_type) = if request.allow_literals
                    && text.len() >= 2
                    && text.starts_with('"')
                    && text.ends_with('"')
                {
                    (
                        "test.string-literal",
                        ExpressionLeafKind::Literal,
                        "java.lang.String",
                    )
                } else if request.allow_expressions && text == "input" && local_context {
                    self.nested_context_seen = true;
                    self.nested_parent_context_seen = request
                        .context
                        .event_classes
                        .iter()
                        .any(|event| event.as_str() == "fixture.Event");
                    ("test.input", ExpressionLeafKind::Custom, "java.lang.Object")
                } else {
                    return None;
                };
                Some(ExpressionLeafCandidate {
                    function: None,
                    effects: (text == "input").then(|| ExpressionEffects::new(1usize)),
                    parser_id: parser_id.to_owned(),
                    kind,
                    timing: if matches!(kind, ExpressionLeafKind::Literal) {
                        ExpressionLeafTiming::AfterRegistered
                    } else {
                        ExpressionLeafTiming::BeforeRegistered
                    },
                    range,
                    return_type: Some(ClassName(return_type.to_owned())),
                    multiplicity: Some(Multiplicity::Single),
                    children: Vec::new(),
                    public_data: Vec::new(),
                    metadata: BTreeMap::new(),
                })
            })
            .collect::<Vec<_>>();
        Ok(candidates.into())
    }

    fn registered_capture_bindings(
        &self,
        syntax: RegisteredSyntaxIdentity<'_>,
    ) -> Result<Vec<RegisteredCaptureBinding>, String> {
        if syntax.dynamic_handler != Some("effect.scoped-expression") {
            return Ok(Vec::new());
        }
        Ok(vec![RegisteredCaptureBinding {
            capture_index: 1,
            parser_id: "host.expression".to_owned(),
            required: true,
            options: BTreeMap::from([
                ("parse.mode".to_owned(), "expressions-only".to_owned()),
                (
                    "context.value.fixture.input-source".to_owned(),
                    "true".to_owned(),
                ),
            ]),
        }])
    }

    fn resolve_effect_candidate(
        &mut self,
        request: EffectSemanticRequest<'_>,
    ) -> Result<EffectSemanticDecision, String> {
        self.effect_context_leaked |= request.context.values.contains_key("fixture.input-source");
        self.effect_parent_context_seen |= request
            .context
            .event_classes
            .iter()
            .any(|event| event.as_str() == "fixture.Event");
        if self.fail_effect { return Err("owned test Effect failure".into()); }
        Ok(if self.reject_effect { EffectSemanticDecision::Reject {
            reason: "owned test Effect rejection".into(), diagnostics: Vec::new(),
        } } else { EffectSemanticDecision::UseCandidate })
    }

    fn apply_expression_effects(&mut self, effects: &ExpressionEffects) -> Result<(), String> {
        self.adopted.push(*effects.downcast_ref::<usize>().ok_or("invalid owned marker")?);
        if self.cancel_after_adoption { self.cancelled = true; }
        Ok(())
    }

    fn begin_semantic_candidate(&mut self) -> Result<(), String> {
        self.checkpoints.push(self.adopted.clone());
        Ok(())
    }

    fn finish_semantic_candidate(&mut self, accepted: bool) -> Result<(), String> {
        let previous = self.checkpoints.pop().ok_or("missing owned checkpoint")?;
        if !accepted { self.adopted = previous; }
        Ok(())
    }

    fn state_revision(&self) -> Result<u64, String> {
        Ok(0)
    }
}

#[test]
fn effect_expression_capture_uses_local_context_without_leaking_it() {
    let catalog = effect_fixture();
    let id = DynamicSyntaxId::new("test.dynamic", "scoped-expression-effect");
    // The omitted nullable `%-string%` still owns capture slot 0, so the
    // matched regex must retain its registration-defined slot 1.
    let pattern = "scoped [%-string% ]using <.+>";
    let definition = DynamicSyntaxDefinition {
        id: id.clone(),
        kind: SyntaxKind::Effect,
        patterns: vec![DynamicPattern {
            source: pattern.to_owned(),
            parsed: syntax_pattern_parser::syntax::parse(pattern, catalog.plural_rules()).unwrap(),
        }],
        priority: 0,
        before: Vec::new(),
        after: Vec::new(),
        return_type: None,
        return_multiplicity: None,
        structure_node_type: None,
        structure_body_mode: None,
        entry_validator: None,
        handler: "effect.scoped-expression".to_owned(),
        metadata: BTreeMap::new(),
        component_load_order: 1,
        declaration_order: 0,
    };
    let dynamic = DynamicSyntaxSnapshot {
        document_id: "file:///workspace/test.sk".to_owned(),
        document_revision: 1,
        registry_revision: 1,
        definitions: BTreeMap::from([(id.clone(), definition)]),
        overrides: BTreeMap::new(),
        candidates: vec![RankedSyntaxCandidate {
            source: SyntaxCandidateSource::Dynamic(id),
            kind: SyntaxKind::Effect,
            overrides: Vec::new(),
        }],
    };
    let source = MappedSource::identity("scoped using input");
    let node = simple_node(&source);
    let mut environment = ScopedExpressionCaptureEnvironment::default();
    let result = parse_effect_with_snapshot(
        &catalog,
        Some(&dynamic),
        EffectParseRequest {
            source: &source,
            node: &node,
            context: event_context(&["fixture.Event"]),
        },
        &mut environment,
        EffectParserConfig::default(),
    )
    .expect("the scoped Effect must parse");

    let selected = result.selected.expect("the scoped Effect must be selected");
    let mapping = selected
        .parsed_captures
        .iter()
        .find(|capture| capture.capture_index == 1)
        .expect("the regex mapping must retain its parsed Expression");
    let Some(ParsedCaptureValue::Expression(expression)) = mapping.result.value.as_ref() else {
        panic!("the regex mapping must be routed through host.expression");
    };
    assert_eq!(
        expression.return_type.as_ref().unwrap().as_str(),
        "java.lang.Object"
    );
    assert_eq!(
        mapping
            .binding
            .options
            .get("parse.mode")
            .map(String::as_str),
        Some("expressions-only")
    );
    assert!(environment.nested_context_seen);
    assert!(environment.nested_parent_context_seen);
    assert!(!environment.effect_context_leaked);
    assert!(environment.effect_parent_context_seen);
    assert_eq!(environment.adopted, vec![1], "retain the selected regex child's deferred effects");
    assert!(environment.checkpoints.is_empty());

    for mode in ["reject", "fail", "cancel"] {
        let mut environment=ScopedExpressionCaptureEnvironment {
            reject_effect: mode == "reject", fail_effect: mode == "fail",
            cancel_after_adoption: mode == "cancel", ..Default::default()
        };
        let result=parse_effect_with_snapshot(&catalog,Some(&dynamic),EffectParseRequest {
            source:&source,node:&node,context:event_context(&["fixture.Event"]),
        },&mut environment,EffectParserConfig::default());
        if mode == "reject" { assert!(result.unwrap().selected.is_none()); }
        else { assert!(result.is_err(), "{mode} must stop the outer candidate"); }
        assert!(environment.adopted.is_empty(), "{mode} must roll back adopted child state");
        assert!(environment.checkpoints.is_empty(), "{mode} must close candidate transactions");
    }
}

#[test]
fn retains_later_fully_matched_effects_as_alternatives() {
    let catalog = effect_fixture();
    let pattern = "ambiguous dynamic";
    let parsed = syntax_pattern_parser::syntax::parse(pattern, catalog.plural_rules()).unwrap();
    let first_id = DynamicSyntaxId::new("test.dynamic", "first");
    let second_id = DynamicSyntaxId::new("test.dynamic", "second");
    let definition = |id: DynamicSyntaxId, declaration_order| DynamicSyntaxDefinition {
        id,
        kind: SyntaxKind::Effect,
        patterns: vec![DynamicPattern {
            source: pattern.to_owned(),
            parsed: parsed.clone(),
        }],
        priority: 0,
        before: Vec::new(),
        after: Vec::new(),
        return_type: None,
        return_multiplicity: None,
        structure_node_type: None,
        structure_body_mode: None,
        entry_validator: None,
        handler: "effect.handle".to_owned(),
        metadata: BTreeMap::new(),
        component_load_order: 1,
        declaration_order,
    };
    let dynamic = DynamicSyntaxSnapshot {
        document_id: "file:///workspace/test.sk".to_owned(),
        document_revision: 1,
        registry_revision: 1,
        definitions: BTreeMap::from([
            (first_id.clone(), definition(first_id.clone(), 0)),
            (second_id.clone(), definition(second_id.clone(), 1)),
        ]),
        overrides: BTreeMap::new(),
        candidates: vec![
            RankedSyntaxCandidate {
                source: SyntaxCandidateSource::Dynamic(first_id),
                kind: SyntaxKind::Effect,
                overrides: Vec::new(),
            },
            RankedSyntaxCandidate {
                source: SyntaxCandidateSource::Dynamic(second_id),
                kind: SyntaxKind::Effect,
                overrides: Vec::new(),
            },
        ],
    };
    let source = MappedSource::identity(pattern);
    let node = simple_node(&source);
    let result = parse_effect_with_snapshot(
        &catalog,
        Some(&dynamic),
        EffectParseRequest {
            source: &source,
            node: &node,
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        EffectParserConfig::default(),
    )
    .unwrap();

    assert_eq!(
        result.selected.unwrap().matched.registration_id,
        "dynamic:test.dynamic/first"
    );
    assert_eq!(result.alternatives.len(), 1);
    assert_eq!(
        result.alternatives[0].matched.registration_id,
        "dynamic:test.dynamic/second"
    );
}

#[test]
fn unknown_effect_retains_exact_code_and_farthest_failure() {
    let catalog = effect_fixture();
    let source = MappedSource::identity("run dummy fixture effect with nope # source trivia");
    let node = simple_node(&source);
    let result = parse_effect(
        &catalog,
        EffectParseRequest {
            source: &source,
            node: &node,
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        EffectParserConfig::default(),
    )
    .expect("unknown Effect is a recoverable parse result");

    let unknown = result.unknown.expect("unknown node must be retained");
    assert_eq!(unknown.source, "run dummy fixture effect with nope");
    let best = unknown
        .failures
        .primary()
        .expect("the Effect registration must remain recognizable");
    assert!(
        best.matched
            .registration_id
            .starts_with("effect:skriptdummyaddon:")
    );
    assert_eq!(
        best.matched
            .trace
            .root_cause()
            .failure
            .span
            .local_range
            .slice(source.virtual_source()),
        Some("nope")
    );
    assert!(
        best.matched
            .trace
            .root_cause()
            .failure
            .reasons
            .iter()
            .any(|reason| matches!(
                reason,
                skript_parser::PatternFailureReason::TypeExpression { expected }
                    if expected == &["string".to_owned()]
            ))
    );
    let failure = unknown
        .failures
        .fallback
        .expect("matcher must retain a farthest failure");
    assert!(failure.failure.span.mapped.virtual_range.start > 0);
    assert!(!failure.failure.reasons.is_empty());
}

#[derive(Default)]
struct SyntheticEffectEnvironment;

impl PatternMatchEnvironment for SyntheticEffectEnvironment {
    fn may_override_pattern(
        &self,
        kind: MatchSyntaxKind,
        _registration_id: &str,
        _pattern_index: usize,
    ) -> bool {
        kind == MatchSyntaxKind::Effect
    }

    fn resolve_type(
        &mut self,
        _request: TypeExpressionRequest<'_>,
    ) -> Result<TypeExpressionOutcome, String> {
        Ok(TypeExpressionOutcome::default())
    }

    fn dispatch_hook(&mut self, event: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
        if event.scope == PatternHookScope::Definition && event.timing == PatternHookTiming::Before
        {
            Ok(PatternHookControl::Match(event.input_range))
        } else {
            Ok(PatternHookControl::Continue)
        }
    }
}

impl ExpressionParseEnvironment for SyntheticEffectEnvironment {
    fn parse_expression_leaf(
        &mut self,
        _request: ExpressionLeafRequest<'_>,
    ) -> Result<ExpressionLeafParse, String> {
        Ok(ExpressionLeafParse::default())
    }

    fn state_revision(&self) -> Result<u64, String> {
        Ok(0)
    }
}

#[test]
fn synthetic_effect_hook_bypasses_static_pattern_prefilters() {
    let catalog = effect_fixture();
    let source = MappedSource::identity("synthetic override");
    let node = simple_node(&source);
    let result = parse_effect(
        &catalog,
        EffectParseRequest {
            source: &source,
            node: &node,
            context: ExpressionParseContext::default(),
        },
        &mut SyntheticEffectEnvironment,
        EffectParserConfig::default(),
    )
    .expect("matching hooks may synthesize an Effect before native matching");

    assert!(result.selected.is_some());
    assert!(result.unknown.is_none());
}

#[test]
fn event_restricted_effect_uses_the_shared_event_context_check() {
    let catalog = ssg::load(fixture())
        .expect("schema 3 fixture must load")
        .into_catalog();
    let text = "make egg hatch";
    let source = MappedSource::identity(text);
    let node = simple_node(&source);

    let allowed = parse_effect(
        &catalog,
        EffectParseRequest {
            source: &source,
            node: &node,
            context: event_context(&["org.bukkit.event.player.PlayerEggThrowEvent"]),
        },
        &mut NoopExpressionEnvironment,
        EffectParserConfig::default(),
    )
    .expect("matching Event context must allow the restricted Effect");
    assert!(allowed.selected.is_some());

    let rejected = parse_effect(
        &catalog,
        EffectParseRequest {
            source: &source,
            node: &node,
            context: ExpressionParseContext::default(),
        },
        &mut NoopExpressionEnvironment,
        EffectParserConfig::default(),
    )
    .expect("event mismatch must be a recoverable Effect failure");
    let unknown = rejected
        .unknown
        .expect("restricted Effect must be rejected");
    let reason = unknown
        .failures
        .candidates
        .iter()
        .find_map(|candidate| event_restriction_reason(&candidate.matched.trace))
        .or_else(|| {
            unknown
                .failures
                .fallback
                .as_ref()
                .and_then(event_restriction_reason)
        })
        .expect("Effect failure must retain the EventRestricted reason");
    assert_eq!(reason.0, ["org.bukkit.event.player.PlayerEggThrowEvent"]);
    assert!(reason.1.is_empty());
}

#[test]
fn nested_restricted_expression_failure_retains_supported_and_current_events() {
    let catalog = ssg::load(fixture())
        .expect("schema 3 fixture must load")
        .into_catalog();
    let text = "send final damage";
    let source = MappedSource::identity(text);
    let node = simple_node(&source);

    for (events, expected_current) in [
        (Vec::new(), Vec::<String>::new()),
        (
            vec!["org.bukkit.event.player.PlayerEggThrowEvent"],
            vec!["org.bukkit.event.player.PlayerEggThrowEvent".to_owned()],
        ),
    ] {
        let event_names = events.as_slice();
        let result = parse_effect(
            &catalog,
            EffectParseRequest {
                source: &source,
                node: &node,
                context: event_context(event_names),
            },
            &mut NoopExpressionEnvironment,
            EffectParserConfig::default(),
        )
        .expect("nested Expression mismatch must be recoverable");
        let unknown = result
            .unknown
            .expect("send final damage must fail outside a damage Event");
        let reason = unknown
            .failures
            .candidates
            .iter()
            .find_map(|candidate| event_restriction_reason(&candidate.matched.trace))
            .or_else(|| {
                unknown
                    .failures
                    .fallback
                    .as_ref()
                    .and_then(event_restriction_reason)
            })
            .expect("nested Expression failure must retain EventRestricted");
        assert_eq!(
            reason.0,
            ["org.bukkit.event.entity.EntityDamageEvent".to_owned()]
        );
        assert_eq!(reason.1, expected_current);
    }
}

mod effect_adoption_scope {
    use super::*;

    #[derive(Default)]
    struct Environment {
        inner: ScopedExpressionCaptureEnvironment,
        reject_first: bool,
        semantic_calls: usize,
    }
    impl PatternMatchEnvironment for Environment {
        fn cancellation_requested(&mut self) -> bool { self.inner.cancelled }
        fn resolve_type(&mut self, request: TypeExpressionRequest<'_>) -> Result<TypeExpressionOutcome, String> {
            self.inner.resolve_type(request)
        }
        fn dispatch_hook(&mut self, event: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
            self.inner.dispatch_hook(event)
        }
    }
    impl ExpressionParseEnvironment for Environment {
        fn state_revision(&self) -> Result<u64, String> { Ok(0) }
        fn parse_expression_leaf(&mut self, request: ExpressionLeafRequest<'_>) -> Result<ExpressionLeafParse, String> {
            // Reuse the existing owned "input" leaf. This local context is only
            // for the fixture leaf; it is not a regex capture binding or effect.
            let mut context = request.context.clone();
            context.values.insert("fixture.input-source".into(), "true".into());
            self.inner.parse_expression_leaf(ExpressionLeafRequest { context: &context, ..request })
        }
        fn resolve_effect_candidate(&mut self, request: EffectSemanticRequest<'_>) -> Result<EffectSemanticDecision, String> {
            self.semantic_calls += 1;
            let previous = self.inner.reject_effect;
            if self.reject_first { self.inner.reject_effect = self.semantic_calls == 1; }
            let result = self.inner.resolve_effect_candidate(request);
            self.inner.reject_effect = previous;
            result
        }
        fn apply_expression_effects(&mut self, effects: &ExpressionEffects) -> Result<(), String> {
            self.inner.apply_expression_effects(effects)
        }
        fn begin_semantic_candidate(&mut self) -> Result<(), String> {
            self.inner.begin_semantic_candidate()
        }
        fn finish_semantic_candidate(&mut self, accepted: bool) -> Result<(), String> {
            self.inner.finish_semantic_candidate(accepted)
        }
    }

    fn snapshot(catalog: &Catalog) -> DynamicSyntaxSnapshot {
        let effect_id = DynamicSyntaxId::new("test.effect-adoption", "effect");
        let root_id = DynamicSyntaxId::new("test.effect-adoption", "root");
        let definition = |id: DynamicSyntaxId, kind, pattern: &str, declaration_order| DynamicSyntaxDefinition {
            id, kind,
            patterns: vec![DynamicPattern {
                source: pattern.into(),
                parsed: syntax_pattern_parser::syntax::parse(pattern, catalog.plural_rules()).unwrap(),
            }],
            priority: 0, before: vec![], after: vec![], return_type: None, return_multiplicity: None,
            structure_node_type: (kind == SyntaxKind::Structure).then_some(syntaxes::NodeType::Section),
            structure_body_mode: (kind == SyntaxKind::Structure).then_some(syntaxes::DynamicStructureBodyMode::Trigger),
            entry_validator: None, handler: "test.effect-adoption".into(), metadata: BTreeMap::new(),
            component_load_order: 1, declaration_order,
        };
        DynamicSyntaxSnapshot {
            document_id: "file:///workspace/effect-adoption.sk".into(), document_revision: 1, registry_revision: 1,
            definitions: BTreeMap::from([
                (effect_id.clone(), definition(effect_id.clone(), SyntaxKind::Effect, "typed %objects%", 0)),
                (root_id.clone(), definition(root_id.clone(), SyntaxKind::Structure, "root", 1)),
            ]),
            overrides: BTreeMap::new(),
            candidates: vec![
                RankedSyntaxCandidate { source: SyntaxCandidateSource::Dynamic(effect_id), kind: SyntaxKind::Effect, overrides: vec![] },
                RankedSyntaxCandidate { source: SyntaxCandidateSource::Dynamic(root_id), kind: SyntaxKind::Structure, overrides: vec![] },
            ],
        }
    }

    #[test]
    fn rejected_typed_capture_effect_rolls_back_matching_state_and_keeps_its_trace() {
        let catalog = effect_fixture();
        let dynamic = snapshot(&catalog);
        let source = MappedSource::identity("typed input");
        let node = simple_node(&source);
        let mut environment = Environment::default();
        environment.inner.reject_effect = true;
        let result = parse_effect_with_snapshot(&catalog, Some(&dynamic), EffectParseRequest {
            source: &source, node: &node, context: event_context(&["fixture.Event"]),
        }, &mut environment, EffectParserConfig::default()).unwrap();
        assert!(result.selected.is_none());
        let unknown = result.unknown.expect("semantic rejection remains diagnosable");
        assert_eq!(unknown.source, "typed input");
        assert!(unknown.failures.primary().unwrap().matched.trace.root_cause().failure.reasons.iter().any(|reason|
            matches!(reason, PatternFailureReason::HookRejected { reason } if reason == "owned test Effect rejection")));
        assert!(environment.semantic_calls > 0, "exercise semantic rejection after a structural match");
        assert!(environment.inner.nested_context_seen, "the matching RHS actually parsed");
        assert!(environment.inner.adopted.is_empty(), "a rejected Effect cannot retain its matching RHS effect");
        assert!(environment.inner.checkpoints.is_empty());
    }

    #[test]
    fn errored_or_cancelled_typed_capture_effect_unwinds_matching_adoption() {
        let catalog = effect_fixture();
        let dynamic = snapshot(&catalog);
        let source = MappedSource::identity("typed input");
        let node = simple_node(&source);
        for mode in ["error", "cancel"] {
            let mut environment = Environment::default();
            environment.inner.fail_effect = mode == "error";
            environment.inner.cancel_after_adoption = mode == "cancel";
            let error = parse_effect_with_snapshot(&catalog, Some(&dynamic), EffectParseRequest {
                source: &source, node: &node, context: event_context(&["fixture.Event"]),
            }, &mut environment, EffectParserConfig::default()).expect_err(mode);
            if mode == "error" { assert!(error.to_string().contains("owned test Effect failure")); }
            assert!(environment.inner.nested_context_seen);
            assert!(environment.inner.adopted.is_empty(), "{mode} cannot retain matching RHS state");
            assert!(environment.inner.checkpoints.is_empty(), "{mode} closes every nested scope");
        }
    }

    #[test]
    fn selected_typed_capture_effect_preserves_its_matching_adoption() {
        let catalog = effect_fixture();
        let dynamic = snapshot(&catalog);
        let source = MappedSource::identity("typed input");
        let node = simple_node(&source);
        let mut environment = Environment::default();
        let result = parse_effect_with_snapshot(&catalog, Some(&dynamic), EffectParseRequest {
            source: &source, node: &node, context: event_context(&["fixture.Event"]),
        }, &mut environment, EffectParserConfig::default()).unwrap();
        assert!(result.selected.is_some());
        assert_eq!(environment.inner.adopted, vec![1]);
        assert!(environment.inner.checkpoints.is_empty());
    }

    #[test]
    fn failed_then_selected_effect_in_one_session_reacquires_a_current_capture() {
        let catalog = effect_fixture();
        let dynamic = snapshot(&catalog);
        let source = MappedSource::identity("root:\n    typed input\n    typed input\n");
        let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
        let mut environment = Environment { reject_first: true, ..Default::default() };
        let document = skript_parser::parse_structures_with_snapshot(&catalog, Some(&dynamic), skript_parser::StructureParseRequest {
            source: &source, tree: &tree, context: event_context(&["fixture.Event"]),
        }, &mut environment, skript_parser::StructureParserConfig::default()).unwrap();
        let selected = document.roots.iter().find_map(|node| match node {
            skript_parser::StructureDocumentNode::Structure(matches) => matches.selected.as_ref(),
            _ => None,
        }).expect("the containing native trigger remains available");
        let skript_parser::StructureBody::Trigger(body) = &selected.body else { panic!("native trigger") };
        assert_eq!(body.len(), 2);
        let skript_parser::SectionBodyNode::Effect(first) = &body[0] else { panic!("first Effect") };
        let skript_parser::SectionBodyNode::Effect(second) = &body[1] else { panic!("second Effect") };
        assert!(first.selected.is_none() && first.unknown.is_some());
        assert!(second.selected.is_some(), "failed capture cleanup must not poison the next statement");
        assert!(!document.diagnostics.is_empty(), "the first rejection remains visible");
        assert_eq!(environment.semantic_calls, 2);
        assert_eq!(environment.inner.adopted, vec![1], "retain only the selected second statement's RHS");
        assert!(environment.inner.checkpoints.is_empty());
    }
}

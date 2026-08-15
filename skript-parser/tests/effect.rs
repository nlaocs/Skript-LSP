use skript_parser::{
    EffectParseRequest, EffectParserConfig, ExpressionLeafCandidate, ExpressionLeafKind,
    ExpressionLeafRequest, ExpressionParseContext, ExpressionParseEnvironment, MappedSource,
    MatchSyntaxKind, NoopExpressionEnvironment, ParsedCaptureValue, PatternHookControl,
    PatternHookEvent, PatternHookScope, PatternHookTiming, PatternMatchEnvironment, RawTreeOptions,
    TextRange, TypeExpressionOutcome, TypeExpressionRequest, parse_effect,
    parse_effect_with_snapshot, parse_raw_tree,
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
    })
}

fn simple_node(source: &MappedSource) -> skript_parser::RawNode {
    let tree = parse_raw_tree(source, RawTreeOptions::for_skript_version(2, 15));
    tree.get(tree.roots[0]).expect("one root node").clone()
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
    ) -> Result<Vec<ExpressionLeafCandidate>, String> {
        let Some(remaining) = request.remaining.slice(request.input) else {
            return Ok(Vec::new());
        };
        if !remaining.starts_with('"') {
            return Ok(Vec::new());
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
                parser_id: "test.string-literal".to_owned(),
                kind: ExpressionLeafKind::Literal,
                range: TextRange::new(request.remaining.start, end),
                return_type: Some(ClassName("java.lang.String".to_owned())),
                multiplicity: Some(Multiplicity::Single),
                children: Vec::new(),
                metadata: BTreeMap::new(),
            })
            .collect())
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
    fn may_override_pattern(&self, kind: MatchSyntaxKind, _registration_id: &str) -> bool {
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
    ) -> Result<Vec<ExpressionLeafCandidate>, String> {
        Ok(Vec::new())
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

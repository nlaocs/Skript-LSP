use skript_parser::{
    ConditionNodeKind, ConditionParseRequest, ConditionParserConfig, ExpressionLeafCandidate,
    ExpressionLeafKind, ExpressionLeafParse, ExpressionLeafRequest, ExpressionLeafTiming,
    ExpressionParseContext, ExpressionParseEnvironment, FailureTrace, MappedSource,
    PatternFailureReason, PatternHookControl, PatternHookEvent, PatternMatchEnvironment, TextRange,
    TypeExpressionOutcome, TypeExpressionRequest, parse_condition,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use syntaxes::{ClassName, Multiplicity};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn event_context(event_classes: &[&str]) -> ExpressionParseContext {
    ExpressionParseContext {
        event_classes: event_classes
            .iter()
            .map(|event| ClassName((*event).to_owned()))
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
fn parses_registered_condition_in_catalog_order() {
    let snapshot = ssg::load(fixture()).expect("schema 4 fixture must load");
    let source = MappedSource::identity("dummy fixture condition");
    let result = parse_condition(
        snapshot.catalog(),
        ConditionParseRequest {
            source: &source,
            range: TextRange::new(0, source.virtual_source().len()),
            context: ExpressionParseContext::default(),
        },
        &mut LiteralEnvironment,
        ConditionParserConfig::default(),
    )
    .expect("registered Condition must parse");

    let selected = result.selected.expect("Condition must be selected");
    let ConditionNodeKind::Registered {
        registration_id,
        pattern_index,
        ..
    } = selected.node.kind
    else {
        panic!("registered Condition expected");
    };
    assert!(registration_id.starts_with("condition:skriptdummyaddon:"));
    assert_eq!(pattern_index, 0);
    assert_eq!(selected.node.span.local_range, TextRange::new(0, 23));
    assert!(selected.node.expressions.is_empty());
    assert!(result.unknown.is_none());
}

#[test]
fn parses_typed_capture_and_preserves_nested_groups() {
    let snapshot = ssg::load(fixture()).expect("schema 4 fixture must load");
    let text = "((dummy fixture condition with \"hello\"))";
    let source = MappedSource::identity(text);
    let result = parse_condition(
        snapshot.catalog(),
        ConditionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            context: ExpressionParseContext::default(),
        },
        &mut LiteralEnvironment,
        ConditionParserConfig::default(),
    )
    .expect("grouped Condition must parse");

    let outer = result.selected.expect("Condition must be selected").node;
    assert!(matches!(outer.kind, ConditionNodeKind::Grouped));
    assert_eq!(outer.span.local_range, TextRange::new(0, text.len()));
    let inner = &outer.children[0];
    assert!(matches!(inner.kind, ConditionNodeKind::Grouped));
    assert_eq!(inner.span.local_range, TextRange::new(1, text.len() - 1));
    let registered = &inner.children[0];
    assert!(matches!(
        registered.kind,
        ConditionNodeKind::Registered { .. }
    ));
    assert_eq!(
        registered.span.local_range,
        TextRange::new(2, text.len() - 2)
    );
    assert_eq!(registered.expressions.len(), 1);
    assert_eq!(
        registered.expressions[0].span.local_range.slice(text),
        Some("\"hello\"")
    );
}

#[test]
fn returns_source_preserving_unknown_condition() {
    let snapshot = ssg::load(fixture()).expect("schema 4 fixture must load");
    let source = MappedSource::identity("not a registered condition");
    let result = parse_condition(
        snapshot.catalog(),
        ConditionParseRequest {
            source: &source,
            range: TextRange::new(0, source.virtual_source().len()),
            context: ExpressionParseContext::default(),
        },
        &mut LiteralEnvironment,
        ConditionParserConfig::default(),
    )
    .expect("unknown Condition is recoverable");

    assert!(result.selected.is_none());
    let unknown = result.unknown.expect("unknown source must be retained");
    assert_eq!(unknown.source, "not a registered condition");
    assert_eq!(unknown.span.local_range, TextRange::new(0, 26));
}

#[test]
fn event_restricted_condition_uses_the_shared_event_context_check() {
    let catalog = ssg::load(fixture())
        .expect("schema 3 fixture must load")
        .into_catalog();
    let text = "the egg will hatch";
    let source = MappedSource::identity(text);

    let allowed = parse_condition(
        &catalog,
        ConditionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            context: event_context(&["org.bukkit.event.player.PlayerEggThrowEvent"]),
        },
        &mut LiteralEnvironment,
        ConditionParserConfig::default(),
    )
    .expect("matching Event context must allow the restricted Condition");
    assert!(allowed.selected.is_some());

    let rejected = parse_condition(
        &catalog,
        ConditionParseRequest {
            source: &source,
            range: TextRange::new(0, text.len()),
            context: ExpressionParseContext::default(),
        },
        &mut LiteralEnvironment,
        ConditionParserConfig::default(),
    )
    .expect("event mismatch must be a recoverable Condition failure");
    let unknown = rejected
        .unknown
        .expect("restricted Condition must be rejected");
    let failure = unknown.failure.expect("Condition failure must be retained");
    let reason = event_restriction_reason(&failure)
        .expect("Condition failure must retain the EventRestricted reason");
    assert_eq!(reason.0, ["org.bukkit.event.player.PlayerEggThrowEvent"]);
    assert!(reason.1.is_empty());
}

struct LiteralEnvironment;

impl PatternMatchEnvironment for LiteralEnvironment {
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

impl ExpressionParseEnvironment for LiteralEnvironment {
    fn parse_expression_leaf(
        &mut self,
        request: ExpressionLeafRequest<'_>,
    ) -> Result<ExpressionLeafParse, String> {
        let candidate = request.candidate_ends.iter().rev().find_map(|end| {
            let range = TextRange::new(request.remaining.start, *end);
            let text = range.slice(request.input)?;
            (text.starts_with('"') && text.ends_with('"')).then_some(range)
        });
        Ok(candidate
            .map(|range| ExpressionLeafCandidate {
                parser_id: "test.string".to_owned(),
                kind: ExpressionLeafKind::Literal,
                timing: ExpressionLeafTiming::AfterRegistered,
                range,
                return_type: Some(ClassName("java.lang.String".to_owned())),
                multiplicity: Some(Multiplicity::Single),
                children: Vec::new(),
                public_data: Vec::new(),
                metadata: BTreeMap::new(),
            })
            .into_iter()
            .collect::<Vec<_>>()
            .into())
    }

    fn state_revision(&self) -> Result<u64, String> {
        Ok(0)
    }
}

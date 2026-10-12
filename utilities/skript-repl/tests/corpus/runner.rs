//! Curated native regressions, not an upstream final-object-graph oracle.

use super::{DOCUMENT_URI, PROJECT_URI, SkriptSession, invocation_context};
use serde::Deserialize;
use serde_json::{Value, json};
use skript_parser::{
    CandidateMatch, EffectCandidate, EffectParseRequest, EffectParserConfig,
    ExpressionExpectedType, ExpressionNode, ExpressionNodeKind, ExpressionParseRequest,
    ExpressionParserConfig, FailureTrace, MatchSpan, ParsedCapture, ParsedCaptureValue,
    PatternCapture, PatternFailureReason, RawNodeKind, TextRange,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use syntaxes::{Catalog, ClassName};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    format: u32,
    fixtures: Vec<Fixture>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    id: String,
    path: String,
    schema: u32,
    snapshot_id: String,
    collection_evidence: String,
    converters: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    fixture: String,
    mode: String,
    input: String,
    event: Option<String>,
    source: Value,
    upstream_final_graph: String,
}

fn corpus() -> Corpus {
    serde_json::from_str(include_str!("cases.json")).expect("curated corpus must deserialize")
}

fn fixture_path(fixture: &Fixture) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(&fixture.path)
}

#[test]
fn snapshot_and_case_provenance() {
    let corpus = corpus();
    assert_eq!(corpus.format, 1);
    let mut ids = BTreeSet::new();
    for fixture in &corpus.fixtures {
        let snapshot = ssg::load(fixture_path(fixture)).expect("complete fixture must validate");
        assert_eq!(snapshot.manifest().schema_version, fixture.schema);
        assert_eq!(snapshot.manifest().snapshot_id, fixture.snapshot_id);
        assert!(!fixture.collection_evidence.is_empty());
        if let Some(count) = fixture.converters {
            assert_eq!(snapshot.catalog().converters().len(), count);
        }
        for case in corpus
            .cases
            .iter()
            .filter(|case| case.fixture == fixture.id)
        {
            assert!(ids.insert(&case.id), "duplicate case {}", case.id);
            assert!(!case.input.trim().is_empty());
            assert_eq!(case.upstream_final_graph, "unknown");
            if let Some(id) = case.source["registration_id"].as_str() {
                let owner = snapshot
                    .catalog()
                    .syntax_by_registration_id(id)
                    .into_iter()
                    .filter_map(|syntax| syntax.common())
                    .find(|common| common.definition_id.as_str() == case.source["definition_id"])
                    .expect("documentation owner must resolve separately from selected syntax");
                let index = case.source["example_index"].as_u64().unwrap() as usize;
                assert_eq!(
                    owner.documentation.examples[index],
                    case.source["raw_example"]
                );
                if case.source["adaptation"].is_null() {
                    assert_eq!(case.input, owner.documentation.examples[index]);
                }
            }
        }
    }
    assert_eq!(
        ids.len(),
        corpus.cases.len(),
        "every case needs a known fixture"
    );
}

#[test]
fn native_corpus_regressions() {
    let expected: BTreeMap<String, Value> =
        serde_json::from_str(include_str!("expected.json")).unwrap();
    let corpus = corpus();
    let upstream = upstream();
    assert_eq!(
        expected.keys().collect::<BTreeSet<_>>(),
        corpus.cases.iter().map(|case| &case.id).collect()
    );
    let mut failures = Vec::new();
    for case in &corpus.cases {
        match run_case(&corpus, case) {
            Ok(actual) => {
                if expected[&case.id]["status"] == "incomplete"
                    && actual["status"] == "matchedClean"
                {
                    eprintln!(
                        "{} improved; no upstream certification is inferred",
                        case.id
                    );
                } else if actual != expected[&case.id] {
                    failures.push(format!(
                        "{} ({:?}, event={:?}):\nexpected {}\nactual {}",
                        case.id, case.input, case.event, expected[&case.id], actual
                    ));
                }
                failures.extend(check_upstream(case, &actual, &upstream));
            }
            Err(error) => failures.push(format!(
                "{} ({:?}, event={:?}): {error}",
                case.id, case.input, case.event
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "{} regression/execution failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn upstream() -> Value {
    serde_json::from_str(include_str!("upstream.json"))
        .expect("observed upstream fields must deserialize")
}

#[test]
fn upstream_evidence_has_matching_artifacts_and_actual_returned_roots() {
    let corpus = corpus();
    let evidence = upstream();
    let fixture = corpus
        .fixtures
        .iter()
        .find(|fixture| fixture.snapshot_id == evidence["snapshot_id"])
        .unwrap();
    let snapshot = ssg::load(fixture_path(fixture)).unwrap();
    for (plugin, hash) in [
        ("Skript", "skript_jar_sha256"),
        ("SkriptSyntaxGenerator", "ssg_jar_sha256"),
    ] {
        let actual = snapshot
            .manifest()
            .plugins
            .iter()
            .find(|entry| entry.name == plugin)
            .unwrap();
        assert_eq!(actual.jar_sha256.as_deref(), evidence[hash].as_str());
    }
    let graphs: Vec<Value> = serde_json::from_str(include_str!("upstream-graphs.json")).unwrap();
    let observations = evidence["observations"].as_array().unwrap();
    assert_eq!(graphs.len(), observations.len());
    for observed in observations {
        let case = corpus
            .cases
            .iter()
            .find(|case| case.id == observed["case_id"])
            .unwrap();
        assert_eq!(case.fixture, fixture.id);
        assert_eq!(observed["coverage"], "partial observed fields only");
        let graph = &graphs[observed["graph_index"].as_u64().unwrap() as usize];
        assert_eq!(graph["valid"], true);
        assert!(graph["validationException"].is_null());
        assert_eq!(graph["returnedLists"].as_array().unwrap().len(), 1);
        assert_eq!(graph["returnedLists"][0].as_array().unwrap().len(), 1);
        let input = if observed["expression_wrapper"] == true {
            format!("on load:\n\tsend {} to console", case.input)
        } else {
            format!("{}:\n\t{}", case.event.as_deref().unwrap(), case.input)
        };
        assert_eq!(graph["source"], input);
        for difference in observed["known_differences"].as_array().unwrap() {
            assert_ne!(difference["expected"], difference["current_native"]);
            assert!(!difference["reason"].as_str().unwrap().is_empty());
        }
    }
}

fn observed_value(case: &Case, actual: &Value, check: &Value) -> Result<Value, String> {
    let path = check["path"].as_str().ok_or("missing observed path")?;
    let value = actual
        .pointer(path)
        .ok_or_else(|| format!("missing native field {path}"))?;
    match check["mode"].as_str() {
        Some("value") => Ok(value.clone()),
        Some("length") => value
            .as_array()
            .map(|values| json!(values.len()))
            .ok_or_else(|| format!("{path} is not an array")),
        Some("sourceNumber") => {
            let start = value[0].as_u64().ok_or("invalid source start")? as usize;
            let end = value[1].as_u64().ok_or("invalid source end")? as usize;
            let source = case
                .input
                .get(start..end)
                .ok_or("invalid UTF-8 source span")?;
            source
                .parse::<f64>()
                .map(|number| json!(number))
                .map_err(|error| format!("invalid fixed numeric source {source:?}: {error}"))
        }
        mode => Err(format!("unknown observed check mode {mode:?}")),
    }
}

fn check_upstream(case: &Case, actual: &Value, evidence: &Value) -> Vec<String> {
    let Some(observed) = evidence["observations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|observed| observed["case_id"] == case.id)
    else {
        return Vec::new();
    };
    let mut failures = Vec::new();
    for check in observed["checks"].as_array().unwrap() {
        match observed_value(case, actual, check) {
            Ok(value) if value == check["expected"] => {}
            result => failures.push(format!(
                "{} upstream {}: expected {}, got {result:?}",
                case.id, check["path"], check["expected"]
            )),
        }
    }
    for check in observed["known_differences"].as_array().unwrap() {
        match observed_value(case, actual, check) {
            Ok(value) if value == check["expected"] => eprintln!(
                "{} {} now agrees with the observed upstream value",
                case.id, check["path"]
            ),
            Ok(value) if value == check["current_native"] => eprintln!(
                "known difference {} {}: native {} / upstream {} ({})",
                case.id, check["path"], value, check["expected"], check["reason"]
            ),
            result => failures.push(format!(
                "{} {} changed beyond the exact documented difference: {result:?}",
                case.id, check["path"]
            )),
        }
    }
    eprintln!(
        "{}: {} observed upstream checks, {} known difference fields, {} unknown scopes; whole graph agreement is not certified",
        case.id,
        observed["checks"].as_array().unwrap().len(),
        observed["known_differences"].as_array().unwrap().len(),
        observed["unknown"].as_array().unwrap().len()
    );
    failures
}

#[test]
fn upstream_checks_reject_new_identity_slot_and_argument_differences() {
    let corpus = corpus();
    let evidence = upstream();
    let expected: BTreeMap<String, Value> =
        serde_json::from_str(include_str!("expected.json")).unwrap();
    for (suffix, path, value) in [
        (
            "short-teleport",
            "/root/selected/identity/class",
            json!("wrong.Effect"),
        ),
        ("short-teleport", "/root/captures/3/state", json!("Default")),
        (
            "named-function-arguments",
            "/root/function/arguments/0/childStart",
            json!(1),
        ),
        (
            "nested-function",
            "/root/children/0/function/name",
            json!("wrong_function"),
        ),
        (
            "research-teleport",
            "/root/captures/2/resolved/children/3/multiplicity",
            json!("Multiple"),
        ),
    ] {
        let case = corpus
            .cases
            .iter()
            .find(|case| case.id == format!("skript-2.16.2-java25/{suffix}"))
            .unwrap();
        let mut changed = expected[&case.id].clone();
        *changed.pointer_mut(path).unwrap() = value;
        assert!(
            !check_upstream(case, &changed, &evidence).is_empty(),
            "{path} must not fall under a broad known-difference exemption"
        );
    }
}

#[test]
fn projection_detects_identity_slots_arguments_types_and_defaults() {
    let corpus = corpus();
    let fixture = corpus
        .fixtures
        .iter()
        .find(|fixture| fixture.id == "skript-2.16.2-java25")
        .unwrap();
    let mut session = SkriptSession::load(fixture_path(fixture)).unwrap();
    session.select_event_header("on join").unwrap();
    let transaction = session.context_transaction.as_ref().unwrap().clone();
    let document = transaction.document_id().unwrap();
    let revision = transaction.document_revision().unwrap();
    let source = skript_parser::MappedSource::identity("send log(base: 2, n: 8)");
    let tree = skript_parser::parse_raw_tree(&source, session.raw_tree_options());
    let result = session
        .host
        .parse_effect_in_parse(
            &transaction,
            invocation_context(100, &document, revision),
            EffectParseRequest {
                source: &source,
                node: tree.get(tree.roots[0]).unwrap(),
                context: session.active_parser_context(),
            },
            EffectParserConfig::default(),
        )
        .unwrap();
    assert!(result.failures.is_empty());
    let selected = result.matches.selected.unwrap();
    let expected = project_effect(&selected, &session.catalog).unwrap();
    let mutations: &[(&str, fn(&mut EffectCandidate))] = &[
        ("definition", |effect| {
            effect.matched.definition_id.push_str("-changed")
        }),
        ("registration", |effect| {
            effect.matched.registration_id.push_str("-changed")
        }),
        ("pattern", |effect| effect.matched.pattern_index += 1),
        ("capture slot", |effect| {
            match &mut effect.matched.matched.captures[0] {
                PatternCapture::TypeExpression { capture_index, .. } => *capture_index += 1,
                _ => panic!("typed slot expected"),
            }
        }),
        ("default/null", |effect| {
            match &mut effect.matched.matched.captures[1] {
                PatternCapture::TypeExpression { state, .. } => {
                    *state = skript_parser::TypeCaptureState::Null
                }
                _ => panic!("typed default slot expected"),
            }
        }),
        ("argument ownership", |effect| {
            first_expression(effect)
                .function
                .as_mut()
                .unwrap()
                .arguments[0]
                .child_start = 1
        }),
        ("child ordering", |effect| {
            first_expression(effect).children.swap(0, 1)
        }),
        ("omitted argument", |effect| {
            first_expression(effect)
                .function
                .as_mut()
                .unwrap()
                .arguments[1]
                .omitted = true
        }),
        ("return type", |effect| {
            first_expression(effect).return_type = Some(ClassName("java.lang.String".to_owned()))
        }),
        ("multiplicity", |effect| {
            first_expression(effect).multiplicity = Some(syntaxes::Multiplicity::Multiple)
        }),
        ("possible types", |effect| {
            first_expression(effect).possible_return_types_state =
                syntaxes::PossibleReturnTypesState::Unresolved
        }),
        ("parse mark", |effect| effect.matched.matched.mark += 1),
    ];
    for (name, mutate) in mutations {
        let mut changed = selected.clone();
        mutate(&mut changed);
        assert_ne!(
            project_effect(&changed, &session.catalog).unwrap(),
            expected,
            "{name} regression must be detected"
        );
    }
    transaction.cancel().unwrap();
}

fn first_expression(effect: &mut EffectCandidate) -> &mut ExpressionNode {
    match effect.parsed_captures[0].result.value.as_mut().unwrap() {
        ParsedCaptureValue::Expression(node) => node,
        _ => panic!("Expression capture expected"),
    }
}

#[test]
#[ignore = "writes native baselines for explicit human review; does not establish upstream compatibility"]
fn record_baselines() {
    let corpus = corpus();
    let mut actual = BTreeMap::new();
    let mut failures = Vec::new();
    for case in &corpus.cases {
        match run_case(&corpus, case) {
            Ok(result) => {
                actual.insert(&case.id, result);
            }
            Err(error) => failures.push(format!("{}: {error}", case.id)),
        }
    }
    assert!(
        failures.is_empty(),
        "no baseline written after execution failure: {}",
        failures.join("\n")
    );
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/expected.json");
    std::fs::write(path, serde_json::to_string_pretty(&actual).unwrap() + "\n").unwrap();
}

fn run_case(corpus: &Corpus, case: &Case) -> Result<Value, String> {
    let fixture = corpus
        .fixtures
        .iter()
        .find(|fixture| fixture.id == case.fixture)
        .ok_or_else(|| format!("unknown fixture {}", case.fixture))?;
    // shortcut: fixed trusted cases use parser budgets, not a hard wall timeout;
    // isolate workers before expanding this runner to an untrusted/full corpus.
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn_scoped(scope, || analyze(fixture, case))
            .map_err(|error| error.to_string())?
            .join()
            .map_err(|_| {
                "parser worker panicked; subsequent cases continue with a fresh session".to_owned()
            })?
    })
}

fn analyze(fixture: &Fixture, case: &Case) -> Result<Value, String> {
    let mut session =
        SkriptSession::load(fixture_path(fixture)).map_err(|error| error.to_string())?;
    if let Some(header) = &case.event {
        let context = session
            .select_event_header(header)
            .map_err(|error| error.to_string())?;
        if !context.component_failures.is_empty() || !context.diagnostics.is_empty() {
            return Err("event setup returned failures or diagnostics".to_owned());
        }
    }
    let parser_context = session.active_parser_context();
    let transaction = match &session.context_transaction {
        Some(transaction) => transaction.clone(),
        None => session
            .host
            .begin_parse(PROJECT_URI, DOCUMENT_URI, 1)
            .map_err(|error| error.to_string())?,
    };
    let document = transaction
        .document_id()
        .map_err(|error| error.to_string())?;
    let revision = transaction
        .document_revision()
        .map_err(|error| error.to_string())?;
    let invocation = invocation_context(100, &document, revision);
    let source = skript_parser::MappedSource::identity(case.input.as_str());
    let analyzed = (|| {
        let (root, failure, diagnostics) = match case.mode.as_str() {
            "expression" => {
                let result = session
                    .host
                    .parse_expression_in_parse(
                        &transaction,
                        invocation,
                        ExpressionParseRequest {
                            source: &source,
                            range: TextRange::new(0, case.input.len()),
                            expected_types: vec![ExpressionExpectedType {
                                class_name: ClassName("java.lang.Object".to_owned()),
                                plural: true,
                            }],
                            context: parser_context.clone(),
                        },
                        ExpressionParserConfig::default(),
                    )
                    .map_err(|error| error.to_string())?;
                if !result.failures.is_empty() {
                    return Err(format!("component failures: {:?}", result.failures));
                }
                let root = result
                    .matches
                    .selected
                    .as_ref()
                    .map(|candidate| project_expression(&candidate.node, &session.catalog))
                    .transpose()?;
                let failure = result.matches.failure.as_ref().map(|failure| {
                    json!({
                        "kind": format!("{:?}", failure.kind), "span": span(&failure.span),
                        "trace": failure.trace.as_ref().map(project_failure),
                    })
                });
                (root, failure, diagnostics(&result.effects))
            }
            "effect" => {
                let tree = skript_parser::parse_raw_tree(&source, session.raw_tree_options());
                if !tree.diagnostics.is_empty() || tree.roots.len() != 1 {
                    return Err("case must be one clean simple line".to_owned());
                }
                let node = tree.get(tree.roots[0]).unwrap();
                if node.kind != RawNodeKind::Simple {
                    return Err("case is not an Effect line".to_owned());
                }
                let result = session
                    .host
                    .parse_effect_in_parse(
                        &transaction,
                        invocation,
                        EffectParseRequest {
                            source: &source,
                            node,
                            context: parser_context.clone(),
                        },
                        EffectParserConfig::default(),
                    )
                    .map_err(|error| error.to_string())?;
                if !result.failures.is_empty() {
                    return Err(format!("component failures: {:?}", result.failures));
                }
                let root = result
                    .matches
                    .selected
                    .as_ref()
                    .map(|candidate| project_effect(candidate, &session.catalog))
                    .transpose()?;
                let failure = result.matches.unknown.as_ref().map(|unknown| {
                    let primary = unknown.failures.primary();
                    json!({"recognized": primary.map(|candidate| json!({
                        "definition": candidate.matched.definition_id, "registration": candidate.matched.registration_id,
                        "patternIndex": candidate.matched.pattern_index, "pattern": candidate.matched.pattern,
                    })), "trace": primary.map(|candidate| &candidate.matched.trace)
                        .or(unknown.failures.fallback.as_ref()).map(project_failure)})
                });
                (root, failure, diagnostics(&result.effects))
            }
            mode => return Err(format!("unsupported mode {mode}")),
        };
        let status = if root.is_some() {
            if diagnostics.as_array().unwrap().is_empty() {
                "matchedClean"
            } else {
                "matchedWithDiagnostics"
            }
        } else if failure
            .as_ref()
            .and_then(|failure| failure.pointer("/trace/eventRestricted"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            "eventRestricted"
        } else if failure
            .as_ref()
            .is_some_and(|failure| !failure["recognized"].is_null() || !failure["trace"].is_null())
        {
            "incomplete"
        } else {
            "unknown"
        };
        Ok(
            json!({"status": status, "root": root, "failure": failure, "diagnostics": diagnostics,
            "context": session.event_context.as_ref().map(|event| json!({"definition": event.definition_id,
                "registration": event.registration_id, "patternIndex": event.pattern_index,
                "events": event.reference_events})),
            "comparison": {"nativeSelected": "completeForProjectedFields", "upstreamFinalGraph": "unknown",
                "conversionPath": "unknown", "simplifiedImplementationClass": "unknown"}}),
        )
    })();
    transaction.cancel().map_err(|error| error.to_string())?;
    analyzed
}

fn diagnostics(effects: &parser_wasm::host::HookEffects) -> Value {
    json!(
        effects
            .diagnostics
            .iter()
            .map(|diagnostic| json!({"code": diagnostic.code,
        "severity": format!("{:?}", diagnostic.severity), "message": diagnostic.message}))
            .collect::<Vec<_>>()
    )
}

fn span(span: &MatchSpan) -> Value {
    json!([span.local_range.start, span.local_range.end])
}

fn project_failure(trace: &FailureTrace) -> Value {
    let cause = trace.root_cause();
    json!({"span": span(&cause.failure.span),
        "reasons": cause.failure.reasons.iter().map(|reason| format!("{reason:?}")).collect::<Vec<_>>(),
        "eventRestricted": cause.failure.reasons.iter().any(|reason| matches!(reason, PatternFailureReason::EventRestricted { .. }))})
}

fn registered(catalog: &Catalog, definition: &str, registration: &str, index: usize) -> Value {
    let common = catalog
        .syntax_by_registration_id(registration)
        .into_iter()
        .filter_map(|syntax| syntax.common())
        .find(|common| common.definition_id.as_str() == definition);
    json!({"definition": definition, "registration": registration, "patternIndex": index,
        "class": common.map(|common| common.element_class.as_str()),
        "pattern": common.and_then(|common| common.patterns.get(index)).map(|pattern| &pattern.source)})
}

fn candidate(candidate: &CandidateMatch, catalog: &Catalog) -> Value {
    json!({"kind": format!("{:?}", candidate.kind),
        "identity": registered(catalog, &candidate.definition_id, &candidate.registration_id, candidate.pattern_index),
        "pattern": candidate.pattern, "mark": candidate.matched.mark,
        "tags": candidate.matched.tags.iter().map(|tag| &tag.value).collect::<Vec<_>>()})
}

fn project_effect(effect: &EffectCandidate, catalog: &Catalog) -> Result<Value, String> {
    Ok(
        json!({"selected": candidate(&effect.matched, catalog), "span": span(&effect.matched.matched.span),
        "captures": project_captures(&effect.matched.matched.captures, &effect.parsed_captures, catalog)?}),
    )
}

fn project_captures(
    raw: &[PatternCapture],
    parsed: &[ParsedCapture],
    catalog: &Catalog,
) -> Result<Vec<Value>, String> {
    raw.iter().map(|capture| {
        let index = capture.capture_index();
        let resolved = parsed.iter().find(|parsed| parsed.capture_index == index);
        let value = resolved.and_then(|parsed| parsed.result.value.as_ref()).map(|value| match value {
            ParsedCaptureValue::Expression(node) => project_expression(node, catalog),
            ParsedCaptureValue::Effect(effect) => project_effect(effect, catalog),
            ParsedCaptureValue::Raw(text) => Ok(json!({"raw": text})),
            other => Err(format!("unprojected capture kind {other:?}; coverage cannot be certified")),
        }).transpose()?;
        let mut projected = match capture {
            PatternCapture::TypeExpression { state, pattern_span, expression, span: input_span, alternative_index, value, .. } => json!({
                "index": index, "state": format!("{state:?}"), "patternSpan": [pattern_span.start, pattern_span.end],
                "typeExpression": format!("{expression:?}"), "span": span(input_span), "source": value,
                "alternativeIndex": alternative_index,
            }),
            PatternCapture::Regex { pattern_span, span: input_span, value, groups, .. } => json!({
                "index": index, "state": "regex", "patternSpan": [pattern_span.start, pattern_span.end],
                "span": span(input_span), "source": value,
                "groups": groups.iter().map(|group| json!({"index": group.index, "source": group.value, "span": group.span.as_ref().map(span)})).collect::<Vec<_>>(),
            }),
        };
        projected["resolved"] = value.unwrap_or(Value::Null);
        projected["parser"] = json!(resolved.map(|parsed| &parsed.result.parser_id));
        projected["status"] = json!(resolved.map(|parsed| format!("{:?}", parsed.result.status)));
        if resolved.is_some_and(|parsed| !parsed.result.diagnostics.is_empty() || !parsed.result.attachments.is_empty()) {
            return Err(format!("capture #{index} has unprojected diagnostics/attachments"));
        }
        Ok(projected)
    }).collect()
}

fn project_expression(node: &ExpressionNode, catalog: &Catalog) -> Result<Value, String> {
    let kind = match &node.kind {
        ExpressionNodeKind::Registered {
            definition_id,
            registration_id,
            element_class,
            pattern_index,
        } => json!({
            "kind": "registered", "identity": registered(catalog, definition_id, registration_id, *pattern_index), "class": element_class.as_str(),
        }),
        ExpressionNodeKind::Default { info } => {
            json!({"kind": "default", "captureIndex": info.capture_index,
            "typeDefinition": info.type_definition_id, "typeRegistration": info.type_registration_id,
            "provider": info.provider_id, "component": info.component_id, "expression": info.expression_source,
            "requestedType": info.requested_type.class_name.as_str(), "plural": info.requested_type.plural,
            "literal": info.is_literal, "time": info.time,
            "anchor": [info.anchor.virtual_range.start, info.anchor.virtual_range.end]})
        }
        ExpressionNodeKind::Grouped => json!({"kind": "grouped"}),
        ExpressionNodeKind::List { conjunction } => {
            json!({"kind": "list", "conjunction": format!("{conjunction:?}")})
        }
        ExpressionNodeKind::Arithmetic {
            operator,
            operation_registration_id,
        } => {
            json!({"kind": "arithmetic", "operator": operator, "operation": operation_registration_id})
        }
        ExpressionNodeKind::Variable { parser_id } => {
            json!({"kind": "variable", "parser": parser_id})
        }
        ExpressionNodeKind::Literal { parser_id } => {
            json!({"kind": "literal", "parser": parser_id})
        }
        ExpressionNodeKind::Function { parser_id } => {
            json!({"kind": "function", "parser": parser_id})
        }
        ExpressionNodeKind::Custom { parser_id } => json!({"kind": "custom", "parser": parser_id}),
    };
    let children = if node.captures.is_empty() {
        node.children
            .iter()
            .map(|child| project_expression(child, catalog))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    Ok(json!({"expression": kind, "span": span(&node.span),
    "returnType": node.return_type.as_ref().map(ClassName::as_str),
    "possibleTypes": node.possible_return_types.iter().map(ClassName::as_str).collect::<Vec<_>>(),
    "possibleTypesState": format!("{:?}", node.possible_return_types_state),
    "multiplicity": node.multiplicity.map(|multiplicity| format!("{multiplicity:?}")),
    "mark": node.mark, "tags": node.tags.iter().map(|tag| &tag.value).collect::<Vec<_>>(),
    "captures": project_captures(&node.captures, &node.parsed_captures(), catalog)?, "children": children,
    "function": node.function.as_ref().map(|call| json!({"name": call.name,
        "definition": call.definition_id, "registration": call.registration_id,
        "arguments": call.arguments.iter().enumerate().map(|(index, argument)| json!({"parameterIndex": index,
            "parameter": argument.parameter_name, "suppliedName": argument.supplied_name,
            "childStart": argument.child_start, "childCount": argument.child_count, "omitted": argument.omitted})).collect::<Vec<_>>()
    }))}))
}

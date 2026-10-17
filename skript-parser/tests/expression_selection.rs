//! Selected results come from a stock Skript JVM, not another native parse.
//! See data/selection-oracle-2.15.4/README.md for observation limits.

use serde::Deserialize;
use skript_parser::{
    CandidateMatch, ExpressionCandidate, ExpressionExpectedType, ExpressionNodeKind,
    ExpressionParseContext, ExpressionParseRequest, ExpressionParserConfig, ExpressionRootMode,
    MappedSource, MatchInput, NoopExpressionEnvironment, PatternMatcherConfig, TextRange,
    catalog_pattern_candidates, match_pattern_candidates_with_environment, parse_expression,
};
use std::{path::Path, sync::OnceLock};
use syntaxes::{ClassName, Multiplicity, Syntax, SyntaxKind};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Oracle {
    snapshot_id: String,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    id: String,
    input: String,
    expected_types: Vec<String>,
    event_classes: Vec<String>,
    observed: Observed,
    mapped: Identity,
    expected_matcher_candidates: Vec<Identity>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Observed {
    element_class: String,
    return_type: String,
    single: bool,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Identity {
    definition_id: String,
    registration_id: String,
    element_class: String,
    pattern_index: usize,
    registry_index: usize,
}

fn fixture() -> &'static ssg::Snapshot {
    static SNAPSHOT: OnceLock<ssg::Snapshot> = OnceLock::new();
    SNAPSHOT.get_or_init(|| {
        ssg::load(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/data/selection-oracle-2.15.4/snapshot"),
        )
        .expect("complete, unmodified same-boot SSG export must load")
    })
}

fn oracle() -> Oracle {
    let oracle: Oracle = serde_json::from_str(include_str!(
        "data/selection-oracle-2.15.4/expectations.json"
    ))
    .expect("independently captured selection cases must decode");
    let observed: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("data/selection-oracle-2.15.4/observed.json")).unwrap();
    assert_eq!(oracle.cases.len(), observed.len());
    for case in &oracle.cases {
        let rows: Vec<_> = observed.iter().filter(|row| row["id"] == case.id).collect();
        assert_eq!(rows.len(), 1, "oracle case must have one raw observation");
        let row = rows[0];
        assert_eq!(row["input"], case.input);
        assert_eq!(row["expectedTypes"], serde_json::json!(case.expected_types));
        assert_eq!(row["eventClasses"], serde_json::json!(case.event_classes));
        assert_eq!(row["parseFlags"], "PARSE_EXPRESSIONS");
        assert_eq!(row["parseContext"], "DEFAULT");
        assert_eq!(row["errors"], serde_json::json!([]));
        assert_eq!(row["selected"]["class"], case.observed.element_class);
        assert_eq!(row["selected"]["returnType"], case.observed.return_type);
        assert_eq!(row["selected"]["single"], case.observed.single);
        assert!(row["selected"]["matchedPattern"].is_null());
    }
    oracle
}

fn identity(candidate: &ExpressionCandidate) -> Identity {
    let ExpressionNodeKind::Registered {
        definition_id,
        registration_id,
        element_class,
        pattern_index,
    } = &candidate.node.kind
    else {
        panic!("a complete registered Expression must be selected: {candidate:?}");
    };
    let registrations = fixture()
        .catalog()
        .syntax_by_registration_id(registration_id);
    assert_eq!(
        registrations.len(),
        1,
        "registration identity must be unique"
    );
    Identity {
        definition_id: definition_id.clone(),
        registration_id: registration_id.clone(),
        element_class: element_class.as_str().to_owned(),
        pattern_index: *pattern_index,
        registry_index: registrations[0]
            .common()
            .expect("Expression common fields")
            .registration_order,
    }
}

fn matcher_identity(candidate: &CandidateMatch) -> Identity {
    let registrations = fixture()
        .catalog()
        .syntax_by_registration_id(&candidate.registration_id);
    assert_eq!(registrations.len(), 1);
    let common = registrations[0].common().expect("Expression common fields");
    Identity {
        definition_id: candidate.definition_id.clone(),
        registration_id: candidate.registration_id.clone(),
        element_class: common.element_class.as_str().to_owned(),
        pattern_index: candidate.pattern_index,
        registry_index: candidate.registration_order,
    }
}

fn assert_cases(ids: &[&str]) {
    let oracle = oracle();
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "data/selection-oracle-2.15.4/snapshot/Manifest.json"
    ))
    .unwrap();
    assert_eq!(manifest["snapshotId"], oracle.snapshot_id);
    for id in ids {
        let case = oracle
            .cases
            .iter()
            .find(|case| case.id == *id)
            .expect("named oracle case must exist");

        // Upstream instances do not retain matchedPattern. This mapping is a
        // separately checked inference: class + exact literal identifies one
        // registration and one original pattern index in the complete export.
        let mappings: Vec<_> = fixture()
            .catalog()
            .syntaxes()
            .iter()
            .filter_map(|syntax| {
                let Syntax::Expression(expression) = syntax else {
                    return None;
                };
                (expression.common.element_class.as_str() == case.observed.element_class)
                    .then_some(expression)
            })
            .flat_map(|expression| {
                expression
                    .common
                    .patterns
                    .iter()
                    .enumerate()
                    .filter_map(|(index, pattern)| {
                        (pattern.source == case.input).then_some((&expression.common, index))
                    })
            })
            .collect();
        assert_eq!(
            mappings.len(),
            1,
            "{id}: observed class and literal must map uniquely"
        );
        let (common, index) = mappings[0];
        assert_eq!(
            case.mapped,
            Identity {
                definition_id: common.definition_id.as_str().to_owned(),
                registration_id: common.registration_id.as_str().to_owned(),
                element_class: common.element_class.as_str().to_owned(),
                pattern_index: index,
                registry_index: common.registration_order,
            },
            "{id}: oracle-to-export identity mapping"
        );

        let source = MappedSource::identity(case.input.as_str());
        let result = parse_expression(
            fixture().catalog(),
            ExpressionParseRequest {
                source: &source,
                range: TextRange::new(0, case.input.len()),
                expected_types: case
                    .expected_types
                    .iter()
                    .map(|name| ExpressionExpectedType {
                        class_name: ClassName(name.clone()),
                        plural: false,
                    })
                    .collect(),
                context: ExpressionParseContext {
                    event_classes: case.event_classes.iter().cloned().map(ClassName).collect(),
                    ..ExpressionParseContext::default()
                },
            },
            &mut NoopExpressionEnvironment,
            ExpressionParserConfig {
                root_mode: ExpressionRootMode::ExpressionsOnly,
                ..ExpressionParserConfig::default()
            },
        )
        .unwrap_or_else(|error| panic!("{id}: parse failed: {error}"));
        let selected = result.selected.as_ref().unwrap_or_else(|| {
            panic!("{id}: upstream selected an Expression, native returned {result:?}")
        });
        assert_eq!(
            identity(selected),
            case.mapped,
            "{id}: selected registration and original pattern index"
        );
        assert_eq!(
            selected.node.return_type.as_ref().map(ClassName::as_str),
            Some(case.observed.return_type.as_str()),
            "{id}: observed return type"
        );
        assert_eq!(
            selected.node.multiplicity,
            Some(if case.observed.single {
                Multiplicity::Single
            } else {
                Multiplicity::Multiple
            }),
            "{id}: observed cardinality"
        );
        assert_eq!(
            selected.node.span.local_range,
            TextRange::new(0, case.input.len()),
            "{id}: must consume the complete input"
        );
        assert!(
            result.failure.is_none(),
            "{id}: successful parse has no root failure"
        );

        // Expression parsing stops at the first semantically accepted registry
        // match. Its alternatives are not all later registrations.
        assert!(
            result.alternatives.is_empty(),
            "{id}: registered first-success result"
        );

        // The lower matcher retains the whole literal competition in resolved
        // registry order, before Expression type/Event applicability is checked.
        // This is a native matcher contract, not observed JVM alternatives.
        let candidates = catalog_pattern_candidates(fixture().catalog(), SyntaxKind::Expression);
        let matched = match_pattern_candidates_with_environment(
            MatchInput::from_source(&source, TextRange::new(0, case.input.len())).unwrap(),
            &candidates,
            &mut NoopExpressionEnvironment,
            PatternMatcherConfig::default(),
        )
        .unwrap_or_else(|error| panic!("{id}: lower matcher failed: {error}"));
        let candidates: Vec<_> = matched
            .selected
            .iter()
            .chain(&matched.alternatives)
            .map(matcher_identity)
            .collect();
        assert_eq!(
            candidates, case.expected_matcher_candidates,
            "{id}: complete ordered lower matcher candidate set"
        );
    }
}

#[test]
fn priority_reversal_selects_the_same_registration_as_upstream() {
    assert_cases(&["registry-rank", "registry-reverse"]);
}

#[test]
fn equal_priority_competition_preserves_the_captured_registry_order() {
    assert_cases(&["registry-tie"]);
}

#[test]
fn expected_type_excludes_the_earlier_incompatible_registration() {
    assert_cases(&["broad-type", "specific-type"]);
}

#[test]
fn event_restriction_selects_the_next_applicable_registration() {
    assert_cases(&["allowed-event", "other-event"]);
}

#[test]
fn competing_and_original_supplier_patterns_preserve_their_indices() {
    assert_cases(&["pattern-index", "existing-supplier"]);
}

use serde::Deserialize;
use skript_parser::{
    MappedSource, MatchInput, MatchPattern, MatchSyntaxKind, NoopPatternMatchHooks,
    PatternCandidate, PatternMatcherConfig, TextRange, TypeExpressionRequest,
    TypeExpressionResolution, TypeExpressionResolver, match_pattern_candidates,
};
use std::path::{Path, PathBuf};
use syntax_pattern_parser::syntax::{
    self, ParseResult, PatternElement, PluralRules, SpannedPatternElement,
};

const FIXTURES: [&str; 2] = ["dummy-addon-2.15.4", "multi-addon-2.15.4"];
const FILES: [&str; 6] = [
    "Events.json",
    "Conditions.json",
    "Effects.json",
    "Expressions.json",
    "Sections.json",
    "Structures.json",
];

#[derive(Deserialize)]
struct SyntaxEntry {
    #[serde(rename = "definitionId")]
    definition_id: String,
    #[serde(rename = "registrationId")]
    registration_id: String,
    patterns: Vec<String>,
}

struct AcceptFirstType;

impl TypeExpressionResolver for AcceptFirstType {
    fn resolve(
        &mut self,
        request: TypeExpressionRequest<'_>,
    ) -> Result<Vec<TypeExpressionResolution>, String> {
        Ok(request
            .candidate_ends
            .first()
            .copied()
            .map(|end| TypeExpressionResolution {
                range: TextRange::new(request.remaining.start, end),
                alternative_index: (!request.expression.alternatives.is_empty()).then_some(0),
                resolution_id: None,
            })
            .into_iter()
            .collect())
    }
}

#[test]
fn generator_patterns_are_safe_to_match_and_all_regexes_compile() {
    let root = corpus_root();
    let shared_rules = read_rules(
        &root
            .parent()
            .expect("corpus has parent")
            .join("PluralRules-2.15.4.json"),
    );
    let mut pattern_count = 0usize;
    let mut regex_count = 0usize;

    for fixture in FIXTURES {
        let directory = root.join(fixture);
        let rules = if directory.join("PluralRules.json").exists() {
            read_rules(&directory.join("PluralRules.json"))
        } else {
            shared_rules.clone()
        };
        for file in FILES {
            let path = directory.join(file);
            let entries: Vec<SyntaxEntry> =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            for entry in entries {
                for source in entry.patterns {
                    pattern_count += 1;
                    let parsed = syntax::parse(&source, &rules)
                        .unwrap_or_else(|error| panic!("{}: {source:?}: {error}", path.display()));
                    let generated = render_sequence(&parsed.elements);
                    run_match(
                        &generated,
                        &source,
                        &parsed,
                        &entry.definition_id,
                        &entry.registration_id,
                    )
                    .unwrap_or_else(|error| {
                        panic!(
                            "{}: generated={generated:?}, pattern={source:?}: {error}",
                            path.display()
                        )
                    });

                    let mut regexes = Vec::new();
                    collect_regexes(&parsed.elements, &mut regexes);
                    for (pattern, span) in regexes {
                        regex_count += 1;
                        let isolated = ParseResult {
                            elements: vec![syntax_pattern_parser::syntax::Spanned::new(
                                PatternElement::Regex(pattern.clone()),
                                span,
                            )],
                            warnings: Vec::new(),
                        };
                        run_match(
                            "matcher regex probe",
                            &source,
                            &isolated,
                            &entry.definition_id,
                            &entry.registration_id,
                        )
                        .unwrap_or_else(|error| {
                            panic!(
                                "{}: regex={pattern:?}, pattern={source:?}: {error}",
                                path.display()
                            )
                        });
                    }
                }
            }
        }
    }

    assert!(pattern_count > 1_000, "unexpectedly small Generator corpus");
    assert!(
        regex_count > 10,
        "Generator corpus did not exercise regexes"
    );
}

fn run_match(
    input: &str,
    source: &str,
    parsed: &ParseResult,
    definition_id: &str,
    registration_id: &str,
) -> Result<(), skript_parser::PatternMatchError> {
    let mapped = MappedSource::identity(input);
    match_pattern_candidates(
        MatchInput::from_source(&mapped, TextRange::new(0, input.len()))?,
        &[PatternCandidate {
            kind: MatchSyntaxKind::Effect,
            definition_id: definition_id.to_owned(),
            registration_id: registration_id.to_owned(),
            priority: 0,
            registration_order: 0,
            resolved_order: None,
            patterns: vec![MatchPattern {
                pattern_index: 0,
                source,
                parsed,
            }],
        }],
        &mut AcceptFirstType,
        &mut NoopPatternMatchHooks,
        PatternMatcherConfig {
            max_states: 500_000,
            max_backtracks: 250_000,
            ..PatternMatcherConfig::default()
        },
    )?;
    Ok(())
}

fn render_sequence(elements: &[SpannedPatternElement]) -> String {
    let mut output = String::new();
    for element in elements {
        match &element.value {
            PatternElement::Literal(value) => output.push_str(value),
            PatternElement::Choice(branches) => {
                if let Some(branch) = branches.first() {
                    output.push_str(&render_sequence(branch));
                }
            }
            PatternElement::Group(children) | PatternElement::Option(children) => {
                output.push_str(&render_sequence(children));
            }
            PatternElement::Regex(_) => output.push('1'),
            PatternElement::TypeExpr(_) => output.push_str("value"),
            PatternElement::ParseTag(_) | PatternElement::ParseMark(_) | PatternElement::Empty => {}
        }
    }
    output
}

fn collect_regexes(
    elements: &[SpannedPatternElement],
    output: &mut Vec<(String, syntax_pattern_parser::syntax::Span)>,
) {
    for element in elements {
        match &element.value {
            PatternElement::Regex(pattern) => output.push((pattern.clone(), element.span)),
            PatternElement::Choice(branches) => {
                for branch in branches {
                    collect_regexes(branch, output);
                }
            }
            PatternElement::Group(children) | PatternElement::Option(children) => {
                collect_regexes(children, output);
            }
            _ => {}
        }
    }
}

fn read_rules(path: &Path) -> PluralRules {
    PluralRules::from_json(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("syntax-pattern-parser")
        .join("tests")
        .join("data")
        .join("corpus")
}

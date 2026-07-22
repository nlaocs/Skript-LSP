#[allow(dead_code)]
mod support;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};
use syntax_pattern_parser::syntax::{self, PluralRules, Span};

const CATEGORIES: [SyntaxCategory; 6] = [
    SyntaxCategory::new("event", "Events.json"),
    SyntaxCategory::new("condition", "Conditions.json"),
    SyntaxCategory::new("effect", "Effects.json"),
    SyntaxCategory::new("expression", "Expressions.json"),
    SyntaxCategory::new("section", "Sections.json"),
    SyntaxCategory::new("structure", "Structures.json"),
];

const DUMMY_ADDON_CORPUS: CorpusFixture = CorpusFixture {
    name: "dummy-addon-2.15.4",
    directory: "dummy-addon-2.15.4",
    expected_schema_version: 3,
    plural_rules_file: None,
    expected_plugins: &["Skript", "SkriptDummyAddon"],
};

const MULTI_ADDON_CORPUS: CorpusFixture = CorpusFixture {
    name: "multi-addon-2.15.4",
    directory: "multi-addon-2.15.4",
    expected_schema_version: 3,
    plural_rules_file: Some("PluralRules.json"),
    expected_plugins: &[
        "Skript",
        "SkJson",
        "skript-reflect",
        "SkBee",
        "Lusk",
        "SkriptDummyAddon",
        "Hippo",
        "skript-particle",
    ],
};

#[derive(Clone, Copy)]
struct SyntaxCategory {
    name: &'static str,
    file_name: &'static str,
}

impl SyntaxCategory {
    const fn new(name: &'static str, file_name: &'static str) -> Self {
        Self { name, file_name }
    }
}

#[derive(Clone, Copy)]
struct CorpusFixture {
    name: &'static str,
    directory: &'static str,
    expected_schema_version: u32,
    plural_rules_file: Option<&'static str>,
    expected_plugins: &'static [&'static str],
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestFixture {
    schema_version: u32,
    snapshot_id: String,
    plugins: Vec<PluginFixture>,
    files: Vec<String>,
}

#[derive(Deserialize)]
struct PluginFixture {
    name: String,
    version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyntaxEntry {
    element_class: String,
    definition_id: Option<String>,
    registration_id: Option<String>,
    patterns: Vec<String>,
}

#[test]
fn parses_dummy_addon_generator_snapshot() {
    run_corpus(DUMMY_ADDON_CORPUS);
}

#[test]
fn parses_multi_addon_generator_snapshot() {
    run_corpus(MULTI_ADDON_CORPUS);
}

fn run_corpus(corpus: CorpusFixture) {
    let root = corpus_root().join(corpus.directory);
    let manifest: ManifestFixture = read_json(&root.join("Manifest.json"));
    validate_manifest(corpus, &manifest);

    let plural_rules_path = corpus
        .plural_rules_file
        .map_or_else(shared_plural_rules_path, |file| root.join(file));
    let plural_rules_json = std::fs::read_to_string(&plural_rules_path).unwrap_or_else(|error| {
        panic!(
            "failed to read plural rules fixture {}: {error}",
            plural_rules_path.display()
        )
    });
    let plural_rules = PluralRules::from_json(&plural_rules_json).unwrap_or_else(|error| {
        panic!(
            "failed to parse plural rules fixture {}: {error}",
            plural_rules_path.display()
        )
    });

    let mut parsed_pattern_count = 0_usize;
    let mut failures = Vec::new();

    for category in CATEGORIES {
        let path = root.join(category.file_name);
        let entries: Vec<SyntaxEntry> = read_json(&path);
        assert!(
            !entries.is_empty(),
            "{} must contain at least one {} entry",
            path.display(),
            category.name
        );

        for (entry_index, entry) in entries.iter().enumerate() {
            for (pattern_index, pattern) in entry.patterns.iter().enumerate() {
                parsed_pattern_count += 1;
                match syntax::parse(pattern, &plural_rules) {
                    Ok(result) => {
                        if let Err(failure) = support::validate_parse_result(pattern, &result) {
                            failures.push(format_failure(
                                corpus,
                                &manifest,
                                category,
                                entry,
                                entry_index,
                                pattern_index,
                                pattern,
                                &failure.message,
                                failure.span,
                            ));
                        }
                    }
                    Err(error) => {
                        failures.push(format_failure(
                            corpus,
                            &manifest,
                            category,
                            entry,
                            entry_index,
                            pattern_index,
                            pattern,
                            &error.to_string(),
                            error.span,
                        ));
                    }
                }
            }
        }
    }

    assert!(
        parsed_pattern_count > 0,
        "{} did not contain any syntax patterns",
        corpus.name
    );

    assert!(
        failures.is_empty(),
        "{} pattern(s) failed in {}:\n\n{}",
        failures.len(),
        corpus.name,
        failures.join("\n\n")
    );
}

fn validate_manifest(corpus: CorpusFixture, manifest: &ManifestFixture) {
    assert_eq!(
        manifest.schema_version, corpus.expected_schema_version,
        "{} has unexpected fixture schema",
        corpus.name
    );
    assert!(
        !manifest.snapshot_id.is_empty(),
        "{} has an empty snapshotId",
        corpus.name
    );

    for category in CATEGORIES {
        assert!(
            manifest.files.iter().any(|file| file == category.file_name),
            "{} does not list {}",
            corpus.name,
            category.file_name
        );
    }

    if let Some(plural_rules_file) = corpus.plural_rules_file {
        assert!(
            manifest.files.iter().any(|file| file == plural_rules_file),
            "{} does not list {plural_rules_file}",
            corpus.name
        );
    }

    for expected_plugin in corpus.expected_plugins {
        let plugin = manifest
            .plugins
            .iter()
            .find(|plugin| plugin.name == *expected_plugin)
            .unwrap_or_else(|| {
                panic!(
                    "{} does not contain expected plugin {expected_plugin}",
                    corpus.name
                )
            });
        assert!(
            !plugin.version.is_empty(),
            "{} has no version for plugin {}",
            corpus.name,
            plugin.name
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn format_failure(
    corpus: CorpusFixture,
    manifest: &ManifestFixture,
    category: SyntaxCategory,
    entry: &SyntaxEntry,
    entry_index: usize,
    pattern_index: usize,
    pattern: &str,
    message: &str,
    span: Span,
) -> String {
    format!(
        "[{corpus_name}] snapshotId={snapshot_id}\n\
         category={category_name} entryIndex={entry_index} patternIndex={pattern_index}\n\
         elementClass={element_class}\n\
         registrationId={registration_id}\n\
         definitionId={definition_id}\n\
         error={message}\n\
         pattern={pattern:?}\n\
         {location}",
        corpus_name = corpus.name,
        snapshot_id = manifest.snapshot_id,
        category_name = category.name,
        element_class = entry.element_class,
        registration_id = entry.registration_id.as_deref().unwrap_or("<missing>"),
        definition_id = entry.definition_id.as_deref().unwrap_or("<missing>"),
        location = render_span(pattern, span),
    )
}

fn render_span(pattern: &str, span: Span) -> String {
    if !span.is_valid_for(pattern) {
        return format!(
            "invalid span {}..{} for pattern length {}",
            span.start,
            span.end,
            pattern.len()
        );
    }

    let line_start = pattern[..span.start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let line_end = pattern[span.end..]
        .find('\n')
        .map_or(pattern.len(), |index| span.end + index);
    let line = &pattern[line_start..line_end];
    let column = pattern[line_start..span.start].chars().count();
    let width = pattern[span.start..span.end].chars().count().max(1);

    format!(
        "span={}..{}\nsource={line}\n       {}{}",
        span.start,
        span.end,
        " ".repeat(column),
        "^".repeat(width)
    )
}

fn read_json<T: DeserializeOwned>(path: &Path) -> T {
    let json = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_str(&json)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("corpus")
}

fn shared_plural_rules_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("PluralRules-2.15.4.json")
}

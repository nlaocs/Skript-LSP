use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use parser_wasm::{
    DocumentCancellationToken, DocumentParseError, DocumentParseRequest, DocumentParseStage,
    DocumentParserConfig, HostConfig, ParserHost,
};
use skript_parser::{RawNodeKind, SectionBodyNode, StructureBody, StructureDocumentNode};

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));
const VARIABLE_SCHEMA_ID: &str = "nlaocs.skript.variable";

fn host(fixture: impl AsRef<Path>) -> ParserHost {
    let catalog = Arc::new(
        ssg::load(fixture)
            .expect("fixture snapshot must load")
            .catalog()
            .clone(),
    );
    ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must initialize from the fixture profile")
}

fn modern_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn legacy_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/type-parser-versions/skript-2.6.4-mc-1.12.2")
}

fn current_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/type-parser-versions/skript-2.16.0")
}

fn request(revision: u64, source: &str) -> DocumentParseRequest {
    DocumentParseRequest::new(
        "file:///workspace",
        "file:///workspace/document.sk",
        revision,
        source.to_owned(),
    )
}

fn trigger_body(result: &parser_wasm::DocumentParseResult) -> &[SectionBodyNode] {
    result
        .syntax
        .roots
        .iter()
        .find_map(|root| match root {
            StructureDocumentNode::Structure(matches) => {
                matches
                    .selected
                    .as_ref()
                    .and_then(|structure| match &structure.body {
                        StructureBody::Trigger(body) => Some(body.as_slice()),
                        StructureBody::Entries(_) | StructureBody::None | StructureBody::Raw(_) => {
                            None
                        }
                    })
            }
            StructureDocumentNode::Trivia(_) | StructureDocumentNode::Unclaimed(_) => None,
        })
        .expect("document must contain a selected trigger Structure")
}

#[test]
fn parses_and_commits_a_document_with_public_expression_data() {
    let mut host = host(modern_fixture());
    let result = host
        .parse_document(
            request(1, "on load:\n    set {_value} to 1\n"),
            DocumentParserConfig::default(),
        )
        .expect("document pipeline must succeed");

    assert!(result.component_failures.is_empty());
    assert_eq!(result.syntax.roots.len(), 1);
    assert!(
        trigger_body(&result).iter().any(|node| matches!(
            node,
            SectionBodyNode::Effect(matches) if matches.selected.is_some()
        )),
        "the Effect body must be parsed"
    );
    let variables = result
        .ast
        .nodes
        .iter()
        .filter_map(|node| node.summary.as_ref())
        .flat_map(|summary| &summary.public_data)
        .filter(|entry| entry.schema_id == VARIABLE_SCHEMA_ID)
        .collect::<Vec<_>>();
    assert_eq!(variables.len(), 1);
    let variable: serde_json::Value =
        serde_json::from_str(&variables[0].json).expect("variable public data must be JSON");
    assert_eq!(variable["scope"], "local");
    assert_eq!(variable["name"][0]["text"], "value");
    assert_eq!(result.state.writes, 0);
}

#[test]
fn parses_tree_macro_generated_effect_text() {
    let mut host = host(modern_fixture());
    let result = host
        .parse_document(
            request(
                2,
                "options:\n    message: send 1\non load:\n    {@message}\n",
            ),
            DocumentParserConfig::default(),
        )
        .expect("options expansion and syntax parsing must succeed");

    assert!(
        result
            .tree_macro_calls
            .iter()
            .any(|call| call.accepted && call.expansion.is_some()),
        "CoreLibrary options expansion must run"
    );
    let event = result
        .syntax
        .roots
        .iter()
        .filter_map(|root| match root {
            StructureDocumentNode::Structure(matches) => matches.selected.as_ref(),
            StructureDocumentNode::Trivia(_) | StructureDocumentNode::Unclaimed(_) => None,
        })
        .find(|structure| matches!(structure.body, StructureBody::Trigger(_)))
        .expect("expanded document must contain an Event Structure");
    let StructureBody::Trigger(body) = &event.body else {
        unreachable!();
    };
    let effect = body
        .iter()
        .find_map(|node| match node {
            SectionBodyNode::Effect(matches) => matches.selected.as_ref(),
            _ => None,
        })
        .expect("generated `send 1` must parse as an Effect");
    let range = effect.matched.matched.span.mapped.virtual_range;
    assert_eq!(range.slice(result.source.virtual_source()), Some("send 1"));
    assert!(range.start >= result.source.original().len());
    let mapped = result
        .source
        .map_range(range)
        .expect("generated Effect span must map to the original document");
    let origin = mapped
        .primary_origin()
        .expect("generated Effect span must retain its call site");
    assert_eq!(
        origin.original_range.slice(result.source.original()),
        Some("    {@message}\n")
    );
    let expansion = origin
        .expansion
        .expect("generated Effect span must identify its Tree expansion");
    let backtrace = result
        .source
        .expansion_backtrace(expansion)
        .expect("generated Effect span must expose an expansion backtrace");
    assert_eq!(backtrace[0].component.as_str(), "nlaocs.core-library");
}

#[test]
fn maps_tree_macro_generated_failures_to_the_call_site() {
    let mut host = host(modern_fixture());
    let result = host
        .parse_document(
            request(
                3,
                "options:\n    broken: this effect does not exist\non load:\n    {@broken}\n",
            ),
            DocumentParserConfig::default(),
        )
        .expect("an unknown generated Effect must remain recoverable");
    let unknown = trigger_body(&result)
        .iter()
        .find_map(|node| match node {
            SectionBodyNode::Effect(matches) => matches.unknown.as_ref(),
            _ => None,
        })
        .expect("generated invalid text must remain as an unknown Effect");
    let diagnostic = unknown
        .failures
        .primary()
        .map(|failure| &failure.matched.trace.failure.span.mapped)
        .or_else(|| {
            unknown
                .failures
                .fallback
                .as_ref()
                .map(|failure| &failure.failure.span.mapped)
        })
        .expect("unknown generated Effect must retain a diagnostic span");
    let origin = diagnostic
        .primary_origin()
        .expect("generated diagnostic must retain its call site");
    assert_eq!(
        origin.original_range.slice(result.source.original()),
        Some("    {@broken}\n")
    );
    let expansion = origin
        .expansion
        .expect("generated diagnostic must identify its Tree expansion");
    assert!(
        result
            .source
            .expansion_backtrace(expansion)
            .is_some_and(|trace| !trace.is_empty())
    );
}

#[test]
fn preserves_unknown_nodes_and_continues_parsing_later_lines() {
    let mut host = host(modern_fixture());
    let result = host
        .parse_document(
            request(4, "on load:\n    this effect does not exist\n    send 1\n"),
            DocumentParserConfig::default(),
        )
        .expect("syntax errors must produce a partial document");
    let body = trigger_body(&result);

    assert!(body.iter().any(|node| matches!(
        node,
        SectionBodyNode::Effect(matches) if matches.selected.is_none() && matches.unknown.is_some()
    )));
    assert!(body.iter().any(|node| matches!(
        node,
        SectionBodyNode::Effect(matches) if matches.selected.is_some()
    )));
}

#[test]
fn derives_multiline_comment_rules_from_the_snapshot_version() {
    let source = "###\nsend 1\n###\n";
    let mut modern = host(modern_fixture());
    let modern = modern
        .parse_document(request(5, source), DocumentParserConfig::default())
        .expect("modern document must parse");
    let mut legacy = host(legacy_fixture());
    let legacy = legacy
        .parse_document(request(5, source), DocumentParserConfig::default())
        .expect("legacy document must parse");

    assert!(
        modern
            .raw_tree
            .nodes
            .iter()
            .all(|node| node.kind != RawNodeKind::Simple)
    );
    assert!(
        legacy
            .raw_tree
            .nodes
            .iter()
            .any(|node| node.kind == RawNodeKind::Simple && node.text == "send 1")
    );
}

#[test]
fn legacy_and_modern_snapshots_share_the_document_parser_api() {
    const SOURCE: &str = "on dummy fixture event with priority high:\n    dummy effect registered through wrapper\nfunction fixture():\n    dummy effect registered through wrapper\ncommand /fixture:\n    trigger:\n        dummy effect registered through wrapper\n";

    for (fixture, legacy) in [(legacy_fixture(), true), (current_fixture(), false)] {
        let mut host = host(fixture);
        let transaction = host
            .begin_parse("file:///workspace", "file:///workspace/document.sk", 10)
            .expect("dynamic syntax snapshot must begin");
        let dynamic = host
            .dynamic_syntax_snapshot(&transaction)
            .expect("dynamic syntax registrations must freeze");
        let legacy_registrations = dynamic
            .definitions
            .keys()
            .filter(|id| {
                id.component_id == "nlaocs.core-library"
                    && id.local_id.starts_with("legacy-struct-")
            })
            .count();
        assert_eq!(legacy_registrations, if legacy { 3 } else { 0 });
        transaction.cancel().expect("inspection may be cancelled");

        let result = host
            .parse_document(request(11, SOURCE), DocumentParserConfig::default())
            .expect("legacy and modern documents must use the same parser API");
        assert!(result.component_failures.is_empty(), "{result:#?}");
        assert_eq!(result.ast.roots.len(), 3, "{result:#?}");
        assert_eq!(result.functions.registrations().len(), 1, "{result:#?}");

        let selected = result
            .syntax
            .roots
            .iter()
            .filter_map(|root| match root {
                StructureDocumentNode::Structure(matches) => matches.selected.as_ref(),
                StructureDocumentNode::Trivia(_) | StructureDocumentNode::Unclaimed(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(selected.len(), 3, "{result:#?}");

        for semantic_mode in ["event-structure", "function-structure", "command-structure"] {
            let structure = selected
                .iter()
                .find(|structure| {
                    structure
                        .metadata
                        .get("nlaocs.core-library/semantic-mode")
                        .is_some_and(|mode| mode == semantic_mode)
                })
                .unwrap_or_else(|| panic!("missing {semantic_mode}: {result:#?}"));
            let is_legacy_registration = structure
                .matched
                .registration_id
                .starts_with("dynamic:nlaocs.core-library/legacy-struct-");
            assert_eq!(is_legacy_registration, legacy, "{structure:#?}");
        }

        let event = selected
            .iter()
            .find(|structure| {
                structure
                    .metadata
                    .get("nlaocs.core-library/semantic-mode")
                    .is_some_and(|mode| mode == "event-structure")
            })
            .expect("event Structure must be selected");
        assert_eq!(
            event
                .metadata
                .get("nlaocs.core-library/event-priority")
                .map(String::as_str),
            Some("high")
        );

        let command = selected
            .iter()
            .find(|structure| {
                structure
                    .metadata
                    .get("nlaocs.core-library/semantic-mode")
                    .is_some_and(|mode| mode == "command-structure")
            })
            .expect("command Structure must be selected");
        assert!(matches!(command.body, StructureBody::Entries(_)));
    }
}

#[test]
fn cancellation_before_start_leaves_the_host_reusable() {
    let mut host = host(modern_fixture());
    let cancellation = DocumentCancellationToken::new();
    cancellation.cancel();
    let error = host
        .parse_document(
            request(6, "on load:\n    send 1\n"),
            DocumentParserConfig {
                cancellation,
                ..DocumentParserConfig::default()
            },
        )
        .expect_err("cancelled document must not parse");
    assert!(matches!(
        error,
        DocumentParseError::Cancelled {
            stage: DocumentParseStage::Start,
            ..
        }
    ));

    host.parse_document(
        request(7, "on load:\n    send 1\n"),
        DocumentParserConfig::default(),
    )
    .expect("a cancelled request must not poison the next revision");
}

#[test]
fn stale_document_revision_cannot_commit_or_poison_a_newer_parse() {
    let mut host = host(modern_fixture());
    let newer = host
        .begin_parse("file:///workspace", "file:///workspace/document.sk", 8)
        .expect("newer revision must begin");

    let error = host
        .parse_document(
            request(7, "on load:\n    send 1\n"),
            DocumentParserConfig::default(),
        )
        .expect_err("stale revision must fail before commit");
    assert!(matches!(
        error,
        DocumentParseError::StaleRevision {
            actual: 7,
            latest: 8,
            ..
        }
    ));

    newer
        .cancel()
        .expect("unused newer transaction may be cancelled");
    host.parse_document(
        request(9, "on load:\n    send 1\n"),
        DocumentParserConfig::default(),
    )
    .expect("a stale revision must not poison a later revision");
}

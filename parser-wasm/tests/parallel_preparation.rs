//! Public-API equivalence and cancellation for isolated invocation preparation.
use parser_wasm::{
    DocumentCancellationToken, DocumentParseError, DocumentParseRequest, DocumentParseResult,
    DocumentParserConfig, HostConfig, ParserHost,
};
use std::{path::Path, sync::Arc};

const CORE: &[u8] = include_bytes!("../../artifacts/core-library.wasm");
const PROJECT: &str = "file:///public-parallel-preparation";
const DOCUMENT: &str = "public-parallel.sk";

fn host() -> ParserHost {
    let snapshot = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/type-parser-versions/skript-2.15.4-type-runtime");
    let catalog = Arc::new(ssg::load(snapshot).unwrap().into_catalog());
    ParserHost::new(
        CORE,
        HostConfig {
            syntax_catalog: Some(catalog),
            ..Default::default()
        },
    )
    .unwrap()
}

fn parse(
    host: &mut ParserHost,
    source: &str,
    revision: u64,
    workers: usize,
) -> DocumentParseResult {
    host.parse_document(
        DocumentParseRequest::new(PROJECT, DOCUMENT, revision, source),
        DocumentParserConfig {
            body_preparation_workers: workers,
            ..Default::default()
        },
    )
    .unwrap()
}

fn fingerprint(result: &DocumentParseResult) -> String {
    assert!(
        result.component_failures.is_empty(),
        "{:?}",
        result.component_failures
    );
    assert!(
        result.fault_regions.is_empty(),
        "{:?}",
        result.fault_regions
    );
    assert!(result.dependency_unavailable.is_empty());
    // Genuine coordinator proofs must authenticate the same native nodes;
    // workers never supply their receipt registries or an adopted forest.
    let witnesses = result.semantic_witnesses();
    let evidence = result
        .ast
        .nodes
        .iter()
        .map(|node| {
            (
                node.id,
                witnesses.native_valid(node),
                witnesses.provider_valid(node, "nlaocs.core-library"),
            )
        })
        .collect::<Vec<_>>();
    format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
        result.execution_status,
        result.raw_tree,
        result.syntax,
        result.functions.definitions(),
        result.ast,
        result.effects,
        result.state,
        result.functions.lookups(),
        evidence
    )
}

#[test]
fn isolated_preparation_preserves_adopted_roots_types_diagnostics_and_proofs() {
    let source = "function value() :: string:\n    return \"text\"\non load:\n    set {_a} to value()\n    send {_a}\ncommand /same:\n    trigger:\n        send \"first\"\ncommand /same:\n    trigger:\n        send \"duplicate\"\non join:\n    if player is set:\n        send \"Unicode: π 日本語\" to player\n    else:\n        this is unsupported\n";
    let serial = fingerprint(&parse(&mut host(), source, 1, 1));
    for workers in [2, 4] {
        let mut parallel = host();
        let result = parse(&mut parallel, source, 1, workers);
        assert_eq!(fingerprint(&result), serial, "worker budget {workers}");
        assert_eq!(parallel.body_preparation_metrics().spawned, workers - 1);
        // A subsequent edit uses the ordinary current-revision cache path.
        let edited = source.replace("return \"text\"", "return \"edited\"");
        let edited_result = parse(&mut parallel, &edited, 2, workers);
        assert_eq!(
            fingerprint(&edited_result),
            fingerprint(&parse(&mut host(), &edited, 2, 1))
        );
        assert_eq!(parallel.body_preparation_metrics().spawned, 0);
    }
}

#[test]
fn one_large_root_has_no_speculative_duplicate_parse() {
    let source = format!(
        "command /large:\n    trigger:\n{}",
        (0..24)
            .map(|i| format!("        send \"line {i}\"\n"))
            .collect::<String>()
    );
    let mut parser = host();
    let result = parse(&mut parser, &source, 1, 4);
    assert_eq!(parser.body_preparation_metrics().spawned, 0);
    assert_eq!(
        parser.body_preparation_metrics().reason,
        "fewer than two root candidates"
    );
    assert_eq!(
        fingerprint(&result),
        fingerprint(&parse(&mut host(), &source, 1, 1))
    );
}

#[test]
fn cancelled_attempt_joins_helpers_and_cannot_lend_results_to_reopened_revision() {
    let source = (0..16)
        .map(|i| format!("on load:\n    send \"line {i}\"\n"))
        .collect::<String>();
    let mut parser = host();
    let token = DocumentCancellationToken::new();
    let signal = token.clone();
    let result = parser.parse_document_with_preparation(
        DocumentParseRequest::new(PROJECT, DOCUMENT, 1, source.as_str()),
        DocumentParserConfig {
            body_preparation_workers: 4,
            cancellation: token,
            ..Default::default()
        },
        &mut |event| {
            if matches!(
                event,
                skript_parser::StructurePreparationEvent::Headers { .. }
            ) {
                signal.cancel();
            }
        },
    );
    assert!(
        matches!(result, Err(DocumentParseError::Cancelled { .. })),
        "{result:?}"
    );
    assert_eq!(parser.body_preparation_metrics().spawned, 3);
    parser.release_document(PROJECT, DOCUMENT, 1).unwrap();
    let reopened = parse(&mut parser, &source, 2, 2);
    assert_eq!(
        fingerprint(&reopened),
        fingerprint(&parse(&mut host(), &source, 2, 1))
    );
}

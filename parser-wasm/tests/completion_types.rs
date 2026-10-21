//! Public-API regression for editing type certificates and a real cancellable guest.
use parser_wasm::{
    DocumentCancellationToken, DocumentParseError, DocumentParseRequest, DocumentParseStage,
    DocumentParserConfig, RuntimeProfile,
    host::{HostConfig, InvocationContext, ParserHost, TextMacroRequest},
};
use skript_parser::MappedSource;
use std::{path::Path, sync::Arc, time::Duration};

const CORE: &[u8] = include_bytes!("../../artifacts/core-library.wasm");
const TEXT: &[u8] = include_bytes!("../../artifacts/text-macro-addon.wasm");
const PROJECT: &str = "file:///public-type-certificate";
const DOCUMENT: &str = "file:///public-type-certificate/test.sk";

fn host(cancellable: bool) -> ParserHost {
    let catalog = ssg::load(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/type-parser-versions/skript-2.15.4-type-runtime"),
    )
    .unwrap()
    .into_catalog();
    let mut config = HostConfig {
        syntax_catalog: Some(Arc::new(catalog)),
        runtime_profile: RuntimeProfile {
            skript_version: Some("2.15.4".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    if cancellable {
        // This existing stress guest intentionally spins. Its test-only quota
        // allows epoch cancellation to win instead of ordinary fuel exhaustion.
        config.fuel_per_call = 1_000_000_000;
        config.call_timeout = Duration::from_secs(3);
        config.epoch_tick = Duration::from_millis(1);
    }
    let mut host = ParserHost::new(CORE, config).unwrap();
    host.load_addon(TEXT).unwrap();
    host
}
fn request(revision: u64, text: &str) -> TextMacroRequest {
    TextMacroRequest {
        context: InvocationContext {
            invocation_id: revision,
            subscription_id: String::new(),
            document_id: DOCUMENT.into(),
            document_revision: revision,
            expansion: None,
            syntax_context: 0,
        },
        source: MappedSource::identity(text),
    }
}

#[test]
fn public_guest_fault_revokes_a_retained_completion_type_certificate() {
    // Core manifest initialization here uses the normal production fuel quota.
    let mut host = host(false);
    let certificate = host.completion_type_certificate();
    assert!(certificate.is_current());
    let result = host.expand_text(PROJECT, request(1, "trap")).unwrap();
    assert!(!result.failures.is_empty());
    assert!(
        host.components()
            .iter()
            .any(|c| c.component_id == "nlaocs.test.text-macro" && c.disabled)
    );
    assert!(!certificate.is_current());
    assert!(host.completion_type_certificate().is_current());
}

#[test]
fn public_document_cancellation_recovers_without_revoking_unchanged_type_providers() {
    let mut host = host(true);
    let warm = host.expand_text(PROJECT, request(1, "healthy")).unwrap();
    assert!(warm.failures.is_empty());
    assert_eq!(warm.calls.len(), 2);
    let certificate = host.completion_type_certificate();
    let cancellation = DocumentCancellationToken::new();
    let signal = cancellation.clone();
    let cancel = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        signal.cancel();
    });
    let result = host.parse_document(
        DocumentParseRequest::new(PROJECT, DOCUMENT, 2, "epoch-cancel"),
        DocumentParserConfig {
            cancellation,
            ..Default::default()
        },
    );
    cancel.join().unwrap();
    assert!(
        matches!(
            result,
            Err(DocumentParseError::Cancelled {
                stage: DocumentParseStage::TextMacro,
                ..
            })
        ),
        "{result:?}"
    );
    assert!(certificate.is_current());
    assert!(host.components().iter().all(|c| !c.disabled));
    let healthy = host.expand_text(PROJECT, request(3, "healthy")).unwrap();
    assert!(healthy.failures.is_empty());
    assert_eq!(healthy.calls.len(), 2);
    assert!(healthy.calls.iter().all(|c| c.accepted));
    assert!(certificate.is_current());
    assert!(host.components().iter().all(|c| !c.disabled));
}

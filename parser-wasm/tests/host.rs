use parser_wasm::bindings::nlaocs::skript_parser_addon::types::DocumentPayload;
use parser_wasm::host::{
    DispatchRequest, DispatchTarget, HookDecision, HookPayload, HookPhase, HostConfig, HostError,
    InvocationContext, ParserHost,
};

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));

fn context() -> InvocationContext {
    InvocationContext {
        invocation_id: 1,
        subscription_id: String::new(),
        document_id: "file:///test.sk".to_owned(),
        document_revision: 1,
        expansion: None,
        syntax_context: 0,
    }
}

fn document_request(phase: HookPhase) -> DispatchRequest {
    DispatchRequest {
        context: context(),
        target: DispatchTarget::ParseStage,
        phase,
        payload: HookPayload::Document(DocumentPayload {
            text: "on load:".to_owned(),
        }),
    }
}

#[test]
fn rejects_a_missing_core_library() {
    let error = ParserHost::new(&[], HostConfig::default())
        .err()
        .expect("an empty CoreLibrary must fail");
    assert!(matches!(error, HostError::CoreLibraryMissing));
}

#[test]
fn loads_and_initializes_the_mandatory_core_library() {
    let host =
        ParserHost::new(CORE_LIBRARY, HostConfig::default()).expect("CoreLibrary must initialize");
    let components = host.components();
    assert_eq!(components.len(), 1);
    assert_eq!(components[0].component_id, "nlaocs.core-library");
    assert_eq!(components[0].component_version, "0.1.0");
    assert!(!components[0].disabled);
}

#[test]
fn skips_wasm_when_no_subscription_matches_the_phase() {
    let mut host =
        ParserHost::new(CORE_LIBRARY, HostConfig::default()).expect("CoreLibrary must initialize");
    let result = host
        .dispatch(document_request(HookPhase::Ast))
        .expect("unmatched dispatch must succeed");
    assert!(result.calls.is_empty());
    assert!(result.failures.is_empty());
    assert!(matches!(result.decision, HookDecision::ContinueProcessing));
}

#[test]
fn dispatches_the_core_health_subscription() {
    let mut host =
        ParserHost::new(CORE_LIBRARY, HostConfig::default()).expect("CoreLibrary must initialize");
    let result = host
        .dispatch(document_request(HookPhase::Document))
        .expect("health hook must dispatch");
    assert_eq!(result.calls.len(), 1);
    assert_eq!(result.calls[0].component_id, "nlaocs.core-library");
    assert_eq!(result.calls[0].subscription_id, "core.health-check");
    assert!(result.failures.is_empty());
    assert!(matches!(result.decision, HookDecision::ContinueProcessing));
    let HookPayload::Document(document) = result.payload else {
        panic!("health hook must preserve the document payload");
    };
    assert_eq!(document.text, "on load:");
}

#[test]
fn rejects_zero_resource_limits_before_starting_wasmtime() {
    let config = HostConfig {
        fuel_per_call: 0,
        ..HostConfig::default()
    };
    let error = ParserHost::new(CORE_LIBRARY, config)
        .err()
        .expect("zero fuel must be rejected");
    assert!(matches!(error, HostError::InvalidConfiguration));
}

use std::{path::Path, sync::Arc};

use parser_wasm::host::{
    CORE_LIBRARY_COMPONENT_ID, DispatchRequest, DispatchTarget, HookDecision, HookPayload,
    HookPhase, HostConfig, HostError, InvocationContext, ParserHost, RuntimeProfile,
};
use parser_wasm::{
    CompatibilityError, bindings::nlaocs::skript_parser_addon::types::DocumentPayload,
};
use syntaxes::{DynamicSyntaxId, SyntaxCandidateSource};

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));
const DYNAMIC_SYNTAX_ADDON: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/dynamic-syntax-addon.wasm"
));
const COMPONENT_ID: &str = "nlaocs.test.dynamic-syntax";
const DELAY_DEFINITION_ID: &str =
    "effect:skript:751b28432979bd1f00e370ffe6f6c3279e4936b90071eda5ed732d7cda2c0504";

fn catalog() -> Arc<syntaxes::Catalog> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../ssg/tests/data/legacy-2.6.4-mc-1.12.2");
    Arc::new(
        ssg::load(&path)
            .expect("legacy schema 3 fixture must load")
            .into_catalog(),
    )
}

fn context(document_id: &str, revision: u64) -> InvocationContext {
    InvocationContext {
        invocation_id: revision,
        subscription_id: String::new(),
        document_id: document_id.to_owned(),
        document_revision: revision,
        expansion: None,
        syntax_context: 0,
    }
}

fn document_request(document_id: &str, revision: u64) -> DispatchRequest {
    document_request_with_text(document_id, revision, "on load:")
}

fn document_request_with_text(document_id: &str, revision: u64, text: &str) -> DispatchRequest {
    DispatchRequest {
        context: context(document_id, revision),
        target: DispatchTarget::ParseStage,
        phase: HookPhase::Document,
        payload: HookPayload::Document(DocumentPayload {
            text: text.to_owned(),
        }),
    }
}

fn configured_host(catalog: Arc<syntaxes::Catalog>) -> ParserHost {
    ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog),
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must initialize with an SSG Catalog")
}

#[test]
fn rejects_dynamic_syntax_addons_without_an_ssg_catalog() {
    let mut host = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            runtime_profile: RuntimeProfile {
                skript_version: Some("2.6.4".to_owned()),
                ..RuntimeProfile::default()
            },
            ..HostConfig::default()
        },
    )
    .expect("CoreLibrary must initialize");
    let error = host
        .load_addon(DYNAMIC_SYNTAX_ADDON)
        .expect_err("dynamic syntax capability must not be advertised without a Catalog");
    assert!(matches!(
        error,
        HostError::Compatibility {
            source: CompatibilityError::MissingRequiredCapability { .. },
            ..
        }
    ));
}

#[test]
fn registers_prepass_syntaxes_overrides_and_unloads_component_state() {
    let catalog = catalog();
    let delay_index = catalog
        .syntaxes()
        .iter()
        .position(|syntax| syntax.definition_id().as_str() == DELAY_DEFINITION_ID)
        .expect("legacy fixture must contain the Delay effect");
    let mut host = configured_host(Arc::clone(&catalog));
    let info = host
        .load_addon(DYNAMIC_SYNTAX_ADDON)
        .expect("dynamic syntax addon must initialize");
    assert_eq!(info.component_id, COMPONENT_ID);

    let initial_transaction = host
        .begin_parse("file:///workspace", "file:///initial.sk", 1)
        .expect("initial snapshot parse must begin");
    let initial = host
        .dynamic_syntax_snapshot(&initial_transaction)
        .expect("initial registrations must freeze");
    assert!(
        initial
            .definitions
            .contains_key(&DynamicSyntaxId::new(COMPONENT_ID, "initial-effect"))
    );
    let delay = initial
        .candidates
        .iter()
        .find(|candidate| candidate.source == SyntaxCandidateSource::Static(delay_index))
        .expect("Delay must remain a static candidate");
    assert_eq!(delay.overrides.len(), 1);
    assert_eq!(delay.overrides[0].handler, "dynamic.delay-override");
    initial_transaction.commit().unwrap();

    let document_id = "file:///prepass.sk";
    let transaction = host
        .begin_parse("file:///workspace", document_id, 1)
        .expect("prepass parse must begin");
    let result = host
        .dispatch_in_parse(&transaction, document_request(document_id, 1))
        .expect("document prepass must dispatch");
    assert!(result.failures.is_empty());
    assert_eq!(result.calls.len(), 2);

    let frozen = host
        .dynamic_syntax_snapshot(&transaction)
        .expect("prepass registrations must freeze");
    assert_eq!(
        frozen
            .definitions
            .keys()
            .filter(|id| id.component_id == COMPONENT_ID)
            .count(),
        2
    );
    let dynamic_order = frozen
        .candidates
        .iter()
        .filter_map(|candidate| match &candidate.source {
            SyntaxCandidateSource::Dynamic(id) if id.component_id == COMPONENT_ID => {
                Some(id.local_id.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(dynamic_order, ["initial-effect", "prepass-effect"]);

    let error = host
        .dispatch_in_parse(&transaction, document_request(document_id, 1))
        .expect_err("document updates must be rejected after the registry freezes");
    assert!(matches!(
        error,
        HostError::DynamicSyntax(syntaxes::DynamicRegistryError::Frozen { .. })
    ));
    transaction.commit().unwrap();

    assert!(host.unload_addon(COMPONENT_ID).unwrap());
    assert!(
        host.components()
            .iter()
            .find(|component| component.component_id == COMPONENT_ID)
            .expect("unloaded component remains observable")
            .disabled
    );
    assert!(matches!(
        host.unload_addon(CORE_LIBRARY_COMPONENT_ID),
        Err(HostError::CannotUnloadCoreLibrary)
    ));

    let future_transaction = host
        .begin_parse("file:///workspace", "file:///after-unload.sk", 1)
        .expect("future parse must begin");
    let future = host
        .dynamic_syntax_snapshot(&future_transaction)
        .expect("future snapshot must freeze");
    assert!(
        future
            .definitions
            .keys()
            .all(|id| id.component_id != COMPONENT_ID)
    );
    assert!(
        future
            .overrides
            .keys()
            .all(|id| id.component_id != COMPONENT_ID)
    );
    assert_eq!(
        frozen
            .definitions
            .keys()
            .filter(|id| id.component_id == COMPONENT_ID)
            .count(),
        2
    );
    future_transaction.commit().unwrap();
}

#[test]
fn rolls_back_dynamic_registrations_when_a_prepass_rejects() {
    let mut host = configured_host(catalog());
    host.load_addon(DYNAMIC_SYNTAX_ADDON)
        .expect("dynamic syntax addon must initialize");
    let document_id = "file:///rejected.sk";
    let transaction = host
        .begin_parse("file:///workspace", document_id, 1)
        .expect("rejected prepass parse must begin");

    let result = host
        .dispatch_in_parse(
            &transaction,
            document_request_with_text(document_id, 1, "reject"),
        )
        .expect("a typed rejection is a successful dispatch result");
    assert!(matches!(result.decision, HookDecision::Reject(_)));

    let snapshot = host
        .dynamic_syntax_snapshot(&transaction)
        .expect("rolled back registry must remain valid");
    assert!(
        snapshot
            .definitions
            .contains_key(&DynamicSyntaxId::new(COMPONENT_ID, "initial-effect"))
    );
    assert!(
        !snapshot
            .definitions
            .contains_key(&DynamicSyntaxId::new(COMPONENT_ID, "prepass-effect"))
    );
    transaction.cancel().unwrap();
}

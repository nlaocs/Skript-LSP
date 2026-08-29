use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parser_wasm::bindings::nlaocs::skript_parser_addon::types::DocumentPayload;
use parser_wasm::host::{
    DispatchRequest, DispatchResult, DispatchTarget, HookDecision, HookPayload, HookPhase,
    HostConfig, HostError, InvocationContext, ParserHost, RuntimeProfile,
};
use syntaxes::CatalogSource;

const CORE_LIBRARY: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));
const CATALOG_DATA_ADDON: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/catalog-data-addon.wasm"
));

const EFFECTS: &[u8] = br#"[
  {
    "registrationId": "duplicate-registration",
    "definitionId": "duplicate-definition",
    "futureField": {"enabled": true},
    "label": "first"
  },
  {
    "registrationId": "duplicate-registration",
    "definitionId": "duplicate-definition",
    "futureField": {"enabled": false},
    "label": "second"
  }
]"#;
const TYPES: &[u8] = br#"[]"#;

fn modern_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn catalog_with_test_source() -> Arc<syntaxes::Catalog> {
    let source = CatalogSource::from_json_documents(
        "ssg",
        3,
        "catalog-test",
        BTreeMap::from([
            ("Effects.json".to_owned(), EFFECTS.to_vec()),
            ("Types.json".to_owned(), TYPES.to_vec()),
        ]),
    )
    .expect("catalog fixture JSON must be valid");
    Arc::new(
        ssg::load(modern_fixture())
            .expect("modern SSG fixture must load")
            .into_catalog()
            .with_unchecked_source(source),
    )
}

fn full_modern_catalog() -> Arc<syntaxes::Catalog> {
    Arc::new(
        ssg::load(modern_fixture())
            .expect("modern SSG fixture must load")
            .into_catalog(),
    )
}

fn request(mode: &str) -> DispatchRequest {
    DispatchRequest {
        context: InvocationContext {
            invocation_id: 1,
            subscription_id: String::new(),
            document_id: "file:///catalog-data-test.sk".to_owned(),
            document_revision: 1,
            expansion: None,
            syntax_context: 0,
        },
        target: DispatchTarget::ParseStage,
        phase: HookPhase::Document,
        payload: HookPayload::Document(DocumentPayload {
            text: mode.to_owned(),
        }),
    }
}

fn host_with_catalog(config: HostConfig) -> ParserHost {
    ParserHost::new(CORE_LIBRARY, config).expect("CoreLibrary must initialize")
}

fn assert_guest_report(result: DispatchResult, expected: &str) {
    assert!(
        result.failures.is_empty(),
        "real catalog guest failed: {:?}",
        result.failures
    );
    assert!(
        matches!(result.decision, HookDecision::ContinueProcessing),
        "catalog fixture should continue processing: {:?}",
        result.decision
    );
    let diagnostic = result
        .effects
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "catalog-data.fixture.passed")
        .unwrap_or_else(|| {
            panic!(
                "catalog fixture did not report success; diagnostics={:?}",
                result.effects.diagnostics
            )
        });
    assert!(
        diagnostic.message.contains(expected),
        "unexpected catalog fixture report: {}",
        diagnostic.message
    );
}

#[test]
fn real_wasm_component_reads_source_pages_records_chunks_and_relations() {
    let mut host = host_with_catalog(HostConfig {
        syntax_catalog: Some(catalog_with_test_source()),
        ..HostConfig::default()
    });
    let info = host
        .load_addon(CATALOG_DATA_ADDON)
        .expect("catalog-data component must load with an SSG source");
    assert_eq!(info.component_id, "test.catalog-data-addon");

    let result = host
        .dispatch(request("catalog-success"))
        .expect("catalog-data dispatch must succeed");
    assert_guest_report(
        result,
        "source, documents, chunks, duplicate IDs, unknown fields, and type relations verified",
    );
}

#[test]
fn real_wasm_component_reaches_every_document_in_a_real_snapshot() {
    let mut host = host_with_catalog(HostConfig {
        syntax_catalog: Some(full_modern_catalog()),
        ..HostConfig::default()
    });
    host.load_addon(CATALOG_DATA_ADDON)
        .expect("catalog-data component must load with the real snapshot");

    let result = host
        .dispatch(request("catalog-full-snapshot"))
        .expect("full snapshot dispatch must succeed");
    assert_guest_report(result, "all 19 real SSG documents are reachable");
}

#[test]
fn real_wasm_component_observes_catalog_capability_advertisement() {
    let mut with_source = host_with_catalog(HostConfig {
        syntax_catalog: Some(catalog_with_test_source()),
        ..HostConfig::default()
    });
    with_source
        .load_addon(CATALOG_DATA_ADDON)
        .expect("component must load with source capability");
    let result = with_source
        .dispatch(request("catalog-profile"))
        .expect("profile dispatch with source must succeed");
    assert_guest_report(result, "catalog capability advertised: true");

    let mut without_source = host_with_catalog(HostConfig::default());
    without_source
        .load_addon(CATALOG_DATA_ADDON)
        .expect("catalog capability is optional for the fixture");
    let result = without_source
        .dispatch(request("catalog-profile"))
        .expect("profile dispatch without source must succeed");
    assert_guest_report(result, "catalog capability advertised: false");
}

#[test]
fn rejects_runtime_profile_from_a_different_snapshot() {
    let result = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog_with_test_source()),
            runtime_profile: RuntimeProfile {
                snapshot_schema_version: Some(3),
                snapshot_id: Some("different-snapshot".to_owned()),
                ..RuntimeProfile::default()
            },
            ..HostConfig::default()
        },
    );
    let error = match result {
        Ok(_) => panic!("profile and source Catalog must identify the same snapshot"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        HostError::CatalogProfileMismatch {
            field: "snapshot ID",
            ..
        }
    ));

    let result = ParserHost::new(
        CORE_LIBRARY,
        HostConfig {
            syntax_catalog: Some(catalog_with_test_source()),
            runtime_profile: RuntimeProfile {
                snapshot_schema_version: Some(4),
                snapshot_id: Some("catalog-test".to_owned()),
                ..RuntimeProfile::default()
            },
            ..HostConfig::default()
        },
    );
    let error = match result {
        Ok(_) => panic!("profile and source Catalog must use the same schema version"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        HostError::CatalogProfileMismatch {
            field: "schema version",
            ..
        }
    ));
}

#[test]
fn real_wasm_component_gets_unavailable_without_an_ssg_source() {
    let mut host = host_with_catalog(HostConfig::default());
    host.load_addon(CATALOG_DATA_ADDON)
        .expect("optional catalog-data component must load without a source");
    let result = host
        .dispatch(request("catalog-unavailable"))
        .expect("unavailable catalog dispatch must succeed");
    assert_guest_report(result, "source-unavailable error verified");
}

#[test]
fn real_wasm_component_sees_chunk_bounds_and_page_quota() {
    let mut host = host_with_catalog(HostConfig {
        max_catalog_response_bytes: 8,
        syntax_catalog: Some(catalog_with_test_source()),
        ..HostConfig::default()
    });
    host.load_addon(CATALOG_DATA_ADDON)
        .expect("catalog-data component must load with a source");
    let result = host
        .dispatch(request("catalog-quota"))
        .expect("quota dispatch must succeed");
    assert_guest_report(
        result,
        "page quota rejection and bounded document chunk verified",
    );
}

#[test]
fn real_wasm_component_reconstructs_document_and_record_chunks_under_small_quota() {
    let mut host = host_with_catalog(HostConfig {
        max_catalog_response_bytes: 192,
        syntax_catalog: Some(catalog_with_test_source()),
        ..HostConfig::default()
    });
    host.load_addon(CATALOG_DATA_ADDON)
        .expect("catalog-data component must load with a source");
    let result = host
        .dispatch(request("catalog-reconstruct"))
        .expect("chunk reconstruction dispatch must succeed");
    assert_guest_report(
        result,
        "descriptor paging and 64-byte document/record reconstruction verified",
    );
}

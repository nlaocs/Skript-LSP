use super::*;
use crate::host::{CORE_LIBRARY_COMPONENT_ID, HostConfig, HostError, ParserHost, RuntimeProfile};

const CORE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../artifacts/core-library.wasm"
));

fn source(component: &str, binary: &str, id: &str) -> AddonDataSource {
    let bytes = serde_json::to_vec(&serde_json::json!({
        "formatVersion": 1, "componentId": component, "id": id,
        "schemaId": "test.optional-data", "schemaVersion": 1,
        "bindings": {}, "payload": {"declarations": []}
    }))
    .unwrap();
    AddonDataSource::from_json(
        AddonDataPin {
            component_id: component.into(),
            component_sha256: binary.into(),
            id: id.into(),
            schema_id: "test.optional-data".into(),
            schema_version: 1,
            sha256: digest(&bytes),
            bindings: BTreeMap::new(),
        },
        &bytes,
    )
    .unwrap()
}
fn config(sources: Vec<AddonDataSource>) -> HostConfig {
    HostConfig {
        addon_data_sources: sources,
        runtime_profile: RuntimeProfile {
            skript_version: Some("2.15.4".into()),
            ..Default::default()
        },
        ..Default::default()
    }
}
fn wrong_sha(actual: &str) -> String {
    if actual == "a".repeat(64) {
        "b".repeat(64)
    } else {
        "a".repeat(64)
    }
}

#[test]
fn addon_data_v2_is_advertised_without_changing_the_wire_or_requiring_sources() {
    assert!(
        crate::host::host_capabilities()
            .iter()
            .any(|c| c.id == crate::CAPABILITY_ADDON_DATA && c.version == 2)
    );
    let v1_consumer = crate::CapabilityRequirement {
        id: crate::CAPABILITY_ADDON_DATA.into(),
        minimum_version: 1,
        required: true,
    };
    assert!(
        crate::validate_compatibility(
            crate::ABI_VERSION,
            crate::ABI_VERSION,
            &[v1_consumer],
            &crate::host::host_capabilities()
        )
        .is_ok()
    );
    let host = ParserHost::new(CORE, config(Vec::new())).unwrap();
    assert_eq!(host.components().len(), 1);
    assert_eq!(host.components()[0].component_id, CORE_LIBRARY_COMPONENT_ID);
}

#[test]
fn a_same_component_wrong_pin_rejects_load_even_with_an_exact_api_source() {
    let actual = digest(CORE);
    let configured = vec![
        source(CORE_LIBRARY_COMPONENT_ID, &actual, "api"),
        source(CORE_LIBRARY_COMPONENT_ID, &wrong_sha(&actual), "members"),
    ];
    assert!(
        validate_sources(&configured),
        "this is a binding failure, not invalid source framing"
    );
    let result = ParserHost::new(CORE, config(configured));
    assert!(
        matches!(result, Err(HostError::AddonDataBindingMismatch {component_id, source_id})
        if component_id == CORE_LIBRARY_COMPONENT_ID && source_id == "members")
    );
}

#[test]
fn exact_sources_continue_and_other_component_pins_do_not_affect_this_load() {
    let actual = digest(CORE);
    let configured = vec![
        source(CORE_LIBRARY_COMPONENT_ID, &actual, "api"),
        source(CORE_LIBRARY_COMPONENT_ID, &actual, "members"),
        source("other.analysis", &wrong_sha(&actual), "members"),
    ];
    assert!(validate_sources(&configured));
    let host = ParserHost::new(CORE, config(configured)).unwrap();
    assert_eq!(host.components().len(), 1);
}

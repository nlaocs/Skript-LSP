//! Explicit native cost experiment, not a real Wasm proof-cache benchmark.
use super::*;
use crate::nlaocs::skript_parser_addon::types::{
    RuntimeAliasCapabilities, RuntimePlugin, RuntimeSnapshotCapabilities,
    RuntimeSyntaxKindCapabilities,
};
use serde_json::{Value, json};
use std::{hint::black_box, time::Instant};

struct RestoreProfile(Option<RuntimeProfile>);
impl Drop for RestoreProfile {
    fn drop(&mut self) {
        *CURRENT_PROFILE
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = self.0.take();
    }
}

fn profile() -> RuntimeProfile {
    let manifest: Value = serde_json::from_str(include_str!(
        "../../parser-wasm/tests/data/type-parser-versions/skript-2.15.4-type-runtime/Manifest.json"
    ))
    .unwrap();
    let text = |v: &Value| v.as_str().unwrap().to_owned();
    RuntimeProfile {
        snapshot_schema_version: Some(manifest["schemaVersion"].as_u64().unwrap() as u32),
        snapshot_id: Some(text(&manifest["snapshotId"])),
        server_name: Some(text(&manifest["server"]["name"])),
        server_version: Some(text(&manifest["server"]["version"])),
        minecraft_version: Some(text(&manifest["server"]["minecraftVersion"])),
        java_version: Some(text(&manifest["server"]["javaVersion"])),
        language: Some(text(&manifest["language"])),
        skript_version: Some("2.15.4".into()),
        plugins: manifest["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| RuntimePlugin {
                load_order: p["loadOrder"].as_u64().unwrap(),
                name: text(&p["name"]),
                version: text(&p["version"]),
                main: text(&p["main"]),
            })
            .collect(),
        snapshot_capabilities: Some(RuntimeSnapshotCapabilities {
            syntax_api: text(&manifest["capabilities"]["syntaxApi"]),
            event_value_api: text(&manifest["capabilities"]["eventValueApi"]),
            syntax_kinds: RuntimeSyntaxKindCapabilities {
                conditions: true,
                effects: true,
                events: true,
                expressions: true,
                types: true,
                functions: true,
                sections: true,
                structures: true,
                properties: true,
                arithmetic: true,
                converters: true,
                comparators: true,
                event_values: true,
            },
            aliases: RuntimeAliasCapabilities {
                supported: true,
                collected: true,
            },
        }),
    }
}

fn borrowed_version(major: u64, minor: u64) -> Option<bool> {
    let guard = CURRENT_PROFILE
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let version = parse_skript_version(guard.as_ref()?.skript_version.as_deref()?)?;
    (version <= LATEST_SUPPORTED_SKRIPT).then_some(version >= (major, minor))
}
fn borrowed_schema(schema: u32) -> Option<bool> {
    let guard = CURRENT_PROFILE
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard
        .as_ref()?
        .snapshot_schema_version
        .map(|value| value >= schema)
}
fn copied_pinned() -> bool {
    current()
        .and_then(|profile| profile.skript_version)
        .as_deref()
        == Some("2.15.4")
}
fn borrowed_pinned() -> bool {
    let guard = CURRENT_PROFILE
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard
        .as_ref()
        .and_then(|profile| profile.skript_version.as_deref())
        == Some("2.15.4")
}

fn sample<T>(mut f: impl FnMut() -> T) -> f64 {
    const ITERATIONS: u32 = 10_000;
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        black_box(f());
    }
    start.elapsed().as_secs_f64() * 1e9 / f64::from(ITERATIONS)
}
fn distribution(mut values: Vec<f64>) -> Value {
    values.sort_by(f64::total_cmp);
    json!({"samplesNsPerOperation": values, "median":values[10], "p95":values[19], "min":values[0], "max":values[20]})
}

#[test]
#[ignore = "manual native allocation-cost experiment; run alone with --test-threads=1"]
fn measure_profile_reads_and_cached_list_projection() {
    let original = current();
    let restore = RestoreProfile(
        CURRENT_PROFILE
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take(),
    );
    for version in [
        None,
        Some("unknown"),
        Some("2.15.4-pre1"),
        Some(" 2.15.4"),
        Some("2.6.4"),
        Some("2.15.4"),
        Some("2.16.0-pre1"),
        Some("2.17.0"),
    ] {
        let mut p = profile();
        p.skript_version = version.map(str::to_owned);
        for schema in [None, Some(3), Some(7)] {
            p.snapshot_schema_version = schema;
            *CURRENT_PROFILE
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(p.clone());
            for (major, minor) in [(2, 6), (2, 16), (2, 17)] {
                assert_eq!(
                    skript_at_least(major, minor),
                    borrowed_version(major, minor)
                );
            }
            assert_eq!(snapshot_schema_at_least(4), borrowed_schema(4));
            assert_eq!(copied_pinned(), borrowed_pinned());
        }
    }
    *CURRENT_PROFILE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    assert_eq!(skript_at_least(2, 6), None);
    assert_eq!(borrowed_version(2, 6), None);
    assert_eq!(snapshot_schema_at_least(4), None);
    assert_eq!(borrowed_schema(4), None);
    let mut rows = Vec::new();
    for plugin_count in [2, 32] {
        let mut p = profile();
        let authentic = p.plugins.clone();
        assert_eq!(authentic.len(), 2);
        p.plugins = (0..plugin_count)
            .map(|i| authentic[i % authentic.len()].clone())
            .collect();
        *CURRENT_PROFILE
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(p);
        let mut snapshot = Vec::new();
        let mut version = Vec::new();
        let mut schema = Vec::new();
        let mut version_probe = Vec::new();
        let mut schema_probe = Vec::new();
        let mut pinned_copy = Vec::new();
        let mut pinned_borrow = Vec::new();
        for batch in 0..21 {
            snapshot.push(sample(current));
            if batch % 2 == 0 {
                version.push(sample(|| skript_at_least(black_box(2), black_box(12))));
                version_probe.push(sample(|| borrowed_version(black_box(2), black_box(12))));
                schema.push(sample(|| snapshot_schema_at_least(black_box(4))));
                schema_probe.push(sample(|| borrowed_schema(black_box(4))));
                pinned_copy.push(sample(copied_pinned));
                pinned_borrow.push(sample(borrowed_pinned));
            } else {
                pinned_borrow.push(sample(borrowed_pinned));
                pinned_copy.push(sample(copied_pinned));
                schema_probe.push(sample(|| borrowed_schema(black_box(4))));
                schema.push(sample(|| snapshot_schema_at_least(black_box(4))));
                version_probe.push(sample(|| borrowed_version(black_box(2), black_box(12))));
                version.push(sample(|| skript_at_least(black_box(2), black_box(12))));
            }
        }
        rows.push(json!({"plugins":plugin_count,"syntheticExpansion":plugin_count!=2,
            "ownedProfileSnapshot":distribution(snapshot), "actualVersionPredicate":distribution(version),
            "actualSchemaPredicate":distribution(schema), "borrowedVersionPrototype":distribution(version_probe),
            "borrowedSchemaPrototype":distribution(schema_probe), "copiedPinnedPredicate":distribution(pinned_copy),
            "borrowedPinnedPrototype":distribution(pinned_borrow)}));
    }
    let classes: Vec<String> = [
        "Long", "Double", "Float", "Boolean", "Short", "Byte", "String",
    ]
    .iter()
    .map(|s| format!("java.lang.{s}"))
    .collect();
    let mut clone_check = Vec::new();
    let mut borrow_check = Vec::new();
    let mut clone_json = Vec::new();
    let mut borrow_json = Vec::new();
    assert_eq!(
        serde_json::to_string(&classes.clone()).unwrap(),
        serde_json::to_string(classes.as_slice()).unwrap()
    );
    for batch in 0..21 {
        let copied = || {
            classes
                .clone()
                .iter()
                .any(|s| s == black_box("java.lang.String"))
        };
        let borrowed = || classes.iter().any(|s| s == black_box("java.lang.String"));
        let copied_json = || serde_json::to_string(&classes.clone()).unwrap();
        let borrowed_json = || serde_json::to_string(classes.as_slice()).unwrap();
        if batch % 2 == 0 {
            clone_check.push(sample(copied));
            borrow_check.push(sample(borrowed));
            clone_json.push(sample(copied_json));
            borrow_json.push(sample(borrowed_json));
        } else {
            borrow_json.push(sample(borrowed_json));
            clone_json.push(sample(copied_json));
            borrow_check.push(sample(borrowed));
            clone_check.push(sample(copied));
        }
    }
    drop(restore);
    assert_eq!(format!("{:?}", current()), format!("{original:?}"));
    println!(
        "core_copy_benchmark {}",
        json!({"nativeOnly":true,"iterationsPerBatch":10000,"batches":21,
        "profile":rows,"sevenClassListModel":{"cloneThenContains":distribution(clone_check),"borrowThenContains":distribution(borrow_check),
        "cloneThenJson":distribution(clone_json),"borrowThenJson":distribution(borrow_json)},
        "limits":"Native profile getters and local Vec/JSON cost model only. Native source stubs are not genuine positive proof caches; no Wasm boundary or end-to-end latency is measured."})
    );
}

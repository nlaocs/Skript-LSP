//! Test-only, opt-in host callback accounting. Timers start after import argument
//! lifting and end before return lowering; they do not measure guest JSON work.
//! Counters are observations outside StateStore, so rejected work is not erased.

use super::*;
use crate::bindings::nlaocs::skript_parser_addon::types::HookInvocation;
use std::time::Instant;

#[derive(Clone, Default)]
pub(super) struct Probe(Arc<Mutex<Measurement>>);

#[derive(Default)]
struct Measurement {
    imports: [ImportSample; 8],
    hooks: BTreeMap<String, HookSample>,
    active: Vec<String>,
}

#[derive(Default, serde::Serialize)]
struct HookSample {
    calls: u64,
    wall_ns: u64,
    fuel_consumed: u64,
    fuel_unavailable: u64,
    imports: [ImportSample; 8],
}

#[derive(Clone, Copy)]
pub(super) enum Operation {
    Source,
    RecordsByRegistrationId,
    ReadRecord,
    TypeForClass,
    ChangeContractForType,
    ReadRecordFields,
    DeclaredMethodExists,
    RegisteredHandlerMatches,
}

const OPERATIONS: [&str; 8] = [
    "source",
    "records_by_registration_id",
    "read_record",
    "type_for_class",
    "change_contract_for_type",
    "read_record_fields",
    "declared_method_exists",
    "registered_handler_matches",
];

#[derive(Clone, Copy, Default, serde::Serialize)]
struct ImportSample {
    calls: u64,
    host_ns: u64,
    returned_raw_bytes: u64,
}

impl Probe {
    pub(super) fn start(&self, operation: Operation) -> Call {
        Call {
            probe: self.clone(),
            operation,
            hook: self.0.lock().unwrap().active.last().cloned(),
            started: Instant::now(),
            returned_raw_bytes: 0,
        }
    }

    fn take(&self) -> [ImportSample; 8] {
        self.take_all().imports
    }

    fn take_all(&self) -> Measurement {
        let mut measurement = self.0.lock().unwrap();
        assert!(measurement.active.is_empty(), "snapshot during active hook");
        std::mem::take(&mut *measurement)
    }

    pub(super) fn hook(&self, subscription: &str, invocation: &HookInvocation, fuel: Option<u64>) -> HookTimer {
        let key = match &invocation.payload {
            HookPayload::RegisteredExpression(payload) => format!("{subscription}/{}#{}", payload.registration_id, payload.pattern_index),
            _ => subscription.to_owned(),
        };
        self.begin_hook(key, fuel)
    }

    fn begin_hook(&self, key: String, fuel: Option<u64>) -> HookTimer {
        {
            let mut measurement = self.0.lock().unwrap();
            measurement.hooks.entry(key.clone()).or_default();
            measurement.active.push(key.clone());
        }
        HookTimer { probe: self.clone(), key, fuel, started: Instant::now() }
    }
}

#[test]
fn nested_probe_attributes_callbacks_without_losing_outer_observations() {
    let probe = Probe::default();
    let outer = probe.begin_hook("outer".into(), Some(100));
    drop(probe.start(Operation::Source));
    let inner = probe.begin_hook("inner".into(), Some(50));
    drop(probe.start(Operation::DeclaredMethodExists));
    inner.finish(Some(30));
    drop(probe.start(Operation::ReadRecordFields));
    outer.finish(Some(40));
    let reset_fuel = probe.begin_hook("reset".into(), Some(1));
    reset_fuel.finish(Some(2));
    let result = probe.take_all();
    assert_eq!(result.imports[Operation::Source as usize].calls, 1);
    assert_eq!(result.hooks["outer"].fuel_consumed, 60);
    assert_eq!(result.hooks["inner"].fuel_consumed, 20);
    assert_eq!(result.hooks["outer"].imports[Operation::DeclaredMethodExists as usize].calls, 0);
    assert_eq!(result.hooks["inner"].imports[Operation::DeclaredMethodExists as usize].calls, 1);
    assert_eq!(result.hooks["outer"].imports[Operation::ReadRecordFields as usize].calls, 1);
    assert_eq!(result.hooks["reset"].fuel_unavailable, 1);
    assert!(probe.take_all().hooks.is_empty());
}

#[test]
#[ignore = "explicit public-fixture comparison of saved Core artifacts; requires SKRIPT_STORAGE_MEASUREMENT_PLAN"]
fn measure_storage_certificate_paths() {
    use sha2::{Digest, Sha256};
    let plan_path = std::env::var("SKRIPT_STORAGE_MEASUREMENT_PLAN").unwrap();
    let plan: serde_json::Value = serde_json::from_slice(&std::fs::read(plan_path).unwrap()).unwrap();
    for file in plan["snapshotFiles"].as_array().unwrap() {
        let bytes = std::fs::read(std::path::Path::new(plan["snapshot"].as_str().unwrap()).join(file["path"].as_str().unwrap())).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), file["sha256"].as_str().unwrap());
    }
    let catalog = Arc::new(ssg::load(std::path::Path::new(plan["snapshot"].as_str().unwrap())).unwrap().into_catalog());
    let cases = plan["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 9);
    assert_eq!(cases[8]["name"], "property-series");
    assert_eq!(cases[8]["text"].as_str().unwrap().len(), 856);
    let mut expected = BTreeMap::<usize, String>::new();
    let mut rows = Vec::new();
    // Counter overhead is separately observable. Order also reverses on the
    // second block; this is attribution, not a latency/speedup assertion.
    for block in 0..2 {
        let order = if block == 0 { [(0, false), (1, true), (1, false), (0, true)] }
            else { [(1, false), (0, true), (0, false), (1, true)] };
        for (variant_index, enabled) in order {
            let variant = &plan["variants"][variant_index];
            let bytes = std::fs::read(variant["component"].as_str().unwrap()).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(&bytes)), variant["sha256"].as_str().unwrap());
            let started = Instant::now();
            let mut host = ParserHost::new(&bytes, HostConfig {
                syntax_catalog: Some(Arc::clone(&catalog)),
                runtime_profile: RuntimeProfile { skript_version: Some("2.15.4".into()), ..Default::default() },
                ..Default::default()
            }).unwrap();
            let host_setup_ms = started.elapsed().as_secs_f64() * 1000.0;
            let probe = Probe::default();
            if enabled {
                for entry in &mut host.components { entry.store.data_mut().catalog_measurement = Some(probe.clone()); }
            }
            for (index, case) in cases.iter().enumerate() {
                let before_calls = guest_calls(&host);
                let started = Instant::now();
                let result = host.parse_document(DocumentParseRequest::new(
                    "file:///storage-measurement", "file:///storage-measurement/public.sk", index as u64 + 1,
                    case["text"].as_str().unwrap()), DocumentParserConfig::default()).unwrap();
                let parse_ms = started.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(result.execution_status, DocumentExecutionStatus::Complete);
                assert!(result.component_failures.is_empty(), "{:?}", result.component_failures);
                assert!(result.raw_tree.diagnostics.is_empty(), "{:?}", result.raw_tree.diagnostics);
                assert!(result.fault_regions.is_empty(), "{:?}", result.fault_regions);
                assert!(result.dependency_unavailable.is_empty(), "{:?}", result.dependency_unavailable);
                assert!(result.syntax.diagnostics.is_empty(), "{:?}", result.syntax.diagnostics);
                assert!(result.effects.diagnostics.is_empty(), "{:?}", result.effects.diagnostics);
                let observed = canonical(&result);
                assert_eq!(expected.entry(index).or_insert_with(|| observed.clone()).as_str(), observed.as_str());
                let witnesses = result.semantic_witnesses();
                let storage_facts = result.ast.nodes.iter().filter(|node| witnesses.provider_valid(node, "nlaocs.core-library"))
                    .flat_map(|node| &node.metadata).filter(|entry| entry.key == "skript.semantic.storage-result")
                    .map(|entry| {
                        assert_eq!(entry.owner_component_id.as_deref(), Some("nlaocs.core-library"));
                        let fact: serde_json::Value = serde_json::from_str(&entry.value).unwrap();
                        assert_eq!(fact["version"], 1);
                        assert_eq!(fact["scope"], "local-scalar");
                        assert_eq!(fact["complete"], true);
                        assert_eq!(fact["multiple"], false);
                        assert_eq!(fact["preservesLocalBindings"], true);
                        assert!(fact["types"].as_array().is_some_and(|types| !types.is_empty()));
                        fact
                    }).collect::<Vec<_>>();
                if index == 8 {
                    let components = storage_facts.iter().filter(|fact| fact["types"] == serde_json::json!(["net.kyori.adventure.text.Component"]) && fact["maybeUnset"] == true).count();
                    assert!(components >= 24, "positive Component post-storage guarantees required: {components}");
                }
                let measurements = probe.take_all();
                assert_eq!(measurements.imports.iter().any(|sample| sample.calls > 0), enabled);
                let metrics = host.incremental_metrics();
                let hook_calls: u64 = measurements.hooks.values().map(|sample| sample.calls).sum();
                assert_eq!(hook_calls, if enabled {guest_calls(&host)-before_calls} else {0});
                let hook_wall_ns: u64 = measurements.hooks.values().map(|sample| sample.wall_ns).sum();
                let hook_fuel: u64 = measurements.hooks.values().map(|sample| sample.fuel_consumed).sum();
                rows.push(serde_json::json!({"block":block,"variant":variant["label"],"componentSha256":variant["sha256"],
                    "probeEnabled":enabled,"case":case["name"],"revision":index+1,"hostSetupMs":host_setup_ms,"parseMs":parse_ms,
                    "guestHookCalls":guest_calls(&host)-before_calls,"storageFacts":storage_facts.len(),"hookCallsObserved":hook_calls,
                    "hookWallNsInclusive":hook_wall_ns,"hookFuel":hook_fuel,"hostIncrementalHits":metrics.hits,"hostHeaderHits":metrics.header_hits,
                    "observationSha256":format!("{:x}",Sha256::digest(observed.as_bytes())),
                    "imports":OPERATIONS.iter().zip(measurements.imports).map(|(name,sample)|serde_json::json!({"operation":name,"sample":sample})).collect::<Vec<_>>(),
                    "hooks":if index==8 {Some(measurements.hooks)} else {None}}));
            }
        }
    }
    println!("storage_certificate_measurement {}", serde_json::json!({"rows":rows,
        "conditions":"Same native host executable (profile.test opt-level=1, debug assertions enabled), saved release-profile before/optimized guest Components, verified real snapshot, nine public edits; host incremental policy unchanged. Not the release LSP project coordinator or UI; costs are not extrapolated to its latency.",
        "limits":"Callback timers exclude WIT lifting/lowering. Hook wall/fuel include the entire invocation and nested work, so totals are inclusive. Remainder is not isolated guest/key time. Cache hit/miss is not directly exported. Initialization traffic unobserved; probe attached afterwards. Probe-off/on comparisons retain complete semantic observations."}));
}

pub(super) struct HookTimer {
    probe: Probe,
    key: String,
    fuel: Option<u64>,
    started: Instant,
}

impl HookTimer {
    pub(super) fn finish(self, remaining: Option<u64>) {
        let wall_ns = u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let mut measurement = self.probe.0.lock().unwrap();
        assert_eq!(measurement.active.pop().as_deref(), Some(self.key.as_str()));
        let sample = measurement.hooks.get_mut(&self.key).unwrap();
        sample.calls += 1;
        sample.wall_ns = sample.wall_ns.saturating_add(wall_ns);
        if let Some(consumed) = self.fuel.zip(remaining).and_then(|(start, end)| start.checked_sub(end)) {
            sample.fuel_consumed = sample.fuel_consumed.saturating_add(consumed);
        } else {
            sample.fuel_unavailable += 1;
        }
    }
}

pub(super) struct Call {
    probe: Probe,
    operation: Operation,
    hook: Option<String>,
    started: Instant,
    returned_raw_bytes: u64,
}

impl Call {
    pub(super) fn returned_bytes(&mut self, bytes: usize) {
        self.returned_raw_bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        let elapsed = u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let mut samples = self
            .probe
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let update = |sample: &mut ImportSample| {
            sample.calls = sample.calls.saturating_add(1);
            sample.host_ns = sample.host_ns.saturating_add(elapsed);
            sample.returned_raw_bytes = sample.returned_raw_bytes.saturating_add(self.returned_raw_bytes);
        };
        update(&mut samples.imports[self.operation as usize]);
        if let Some(key) = &self.hook {
            update(&mut samples.hooks.get_mut(key).unwrap().imports[self.operation as usize]);
        }
    }
}

fn actual_host(catalog: Arc<Catalog>) -> ParserHost {
    let mut host = ParserHost::new(
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../artifacts/core-library.wasm"
        )),
        HostConfig {
            syntax_catalog: Some(catalog),
            runtime_profile: RuntimeProfile {
                skript_version: Some("2.15.4".to_owned()),
                ..RuntimeProfile::default()
            },
            ..HostConfig::default()
        },
    )
    .unwrap();
    // Decline host reuse, as in the existing cold/reused equivalence tests.
    // Keep native parsing and the guest's own immutable caches unchanged.
    for entry in &mut host.components {
        entry.manifest.incremental_local_subscriptions.clear();
        entry.manifest.incremental_header_handlers.clear();
    }
    host
}

fn guest_calls(host: &ParserHost) -> u64 {
    host.components
        .iter()
        .map(|entry| entry.store.data().guest_hook_calls)
        .sum()
}

fn canonical(result: &DocumentParseResult) -> String {
    let mut ast = result.ast.clone();
    for node in &mut ast.nodes {
        node.metadata.retain(|entry| {
            entry.owner_component_id.is_some() || entry.key != "parser.semantic-writer-receipt.v1"
        });
        if let Some(summary) = &mut node.summary {
            summary.metadata.retain(|entry| {
                entry.owner_component_id.is_some()
                    || entry.key != "parser.semantic-writer-receipt.v1"
            });
        }
    }
    let mut effects = result.effects.clone();
    for parsed in &mut effects.parse_results {
        parsed.host_token = 0;
    }
    let witnesses = result.semantic_witnesses();
    let proof = result
        .ast
        .nodes
        .iter()
        .map(|node| {
            (
                node.id,
                witnesses.native_valid(node),
                witnesses.unique_issuer(node),
            )
        })
        .collect::<Vec<_>>();
    let trace = result
        .syntax_calls
        .iter()
        .map(|call| (&call.component_id, &call.subscription_id))
        .collect::<Vec<_>>();
    format!(
        "{ast:?}|{effects:?}|{:?}|{:?}|{proof:?}|{trace:?}|{:?}|{:?}|{:?}|{:?}",
        result.syntax.diagnostics,
        result.functions,
        result.state.writes,
        result.state.read_write_set.reads,
        result.state.read_write_set.writes,
        result
            .state
            .read_write_set
            .namespace_revisions
            .keys()
            .collect::<Vec<_>>()
    )
}

#[test]
#[ignore = "explicit real-Wasm catalog boundary measurement; no latency assertion"]
fn measure_catalog_imports_with_real_core() {
    use sha2::{Digest, Sha256};

    const SOURCE: &str = "function echo(value: text) :: text:\n    set {_copy} to {_value}\n    return {_copy}\non load:\n    set {_message} to echo(\"hello\")\n    set {_count} to 2\n    if {_count} is set:\n        send \"%{_message}%:%{_count}%\" to console\n";
    let catalog_started = Instant::now();
    let catalog = Arc::new(
        ssg::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/data/type-parser-versions/skript-2.15.4-type-runtime"),
        )
        .unwrap()
        .into_catalog(),
    );
    let catalog_setup_ms = catalog_started.elapsed().as_secs_f64() * 1000.0;
    let mut expected = BTreeMap::<u64, String>::new();
    let mut rows = Vec::new();
    // Disabled/enabled use the same binary and input. Active probe overhead is
    // a separate comparison; no wall-time threshold or speedup is asserted.
    for fresh in 0..3 {
        for enabled in if fresh % 2 == 0 {
            [false, true]
        } else {
            [true, false]
        } {
            let setup_started = Instant::now();
            let mut host = actual_host(Arc::clone(&catalog));
            let probe = Probe::default();
            if enabled {
                // create_store starts with None: initialization/manifest import
                // traffic is intentionally unobserved, never reported as zero.
                for entry in &mut host.components {
                    entry.store.data_mut().catalog_measurement = Some(probe.clone());
                }
            }
            let host_setup_ms = setup_started.elapsed().as_secs_f64() * 1000.0;
            for revision in 1..=4 {
                let calls_before = guest_calls(&host);
                let parse_started = Instant::now();
                let result = host
                    .parse_document(
                        DocumentParseRequest::new(
                            "file:///catalog-measurement",
                            "file:///catalog-measurement/public.sk",
                            revision,
                            SOURCE,
                        ),
                        DocumentParserConfig::default(),
                    )
                    .unwrap();
                let parse_ms = parse_started.elapsed().as_secs_f64() * 1000.0;
                let samples = probe.take();
                assert_eq!(result.execution_status, DocumentExecutionStatus::Complete);
                assert!(
                    result.component_failures.is_empty(),
                    "{:?}",
                    result.component_failures
                );
                assert!(
                    result.raw_tree.diagnostics.is_empty(),
                    "{:?}",
                    result.raw_tree.diagnostics
                );
                assert!(
                    result.syntax.diagnostics.is_empty(),
                    "{:?}",
                    result.syntax.diagnostics
                );
                assert!(
                    result.effects.diagnostics.is_empty(),
                    "{:?}",
                    result.effects.diagnostics
                );
                let witnesses = result.semantic_witnesses();
                let positive_variable_types = result
                    .ast
                    .nodes
                    .iter()
                    .filter(|node| witnesses.provider_valid(node, "nlaocs.core-library"))
                    .flat_map(|node| &node.metadata)
                    .filter(|entry| {
                        entry.key == "skript.semantic.variable-read"
                            && entry.owner_component_id.as_deref() == Some("nlaocs.core-library")
                    })
                    .map(|entry| {
                        serde_json::from_str::<serde_json::Value>(&entry.value).unwrap()
                        ["noRefreshTypes"].as_array().unwrap().len()
                    })
                    .sum::<usize>();
                assert!(
                    positive_variable_types > 0,
                    "a genuine positive runtime proof is required"
                );
                let metrics = host.incremental_metrics();
                assert_eq!(metrics.hits, 0);
                assert_eq!(metrics.header_hits, 0);
                let actual_guest_calls = guest_calls(&host) - calls_before;
                assert!(actual_guest_calls > 0);
                let import_calls = samples.iter().map(|sample| sample.calls).sum::<u64>();
                assert_eq!(import_calls > 0, enabled);
                // Compare the same current revision, retaining revision metadata.
                // Only host receipt/resource IDs are excluded, as in existing
                // incremental equivalence tests; no provider metadata is erased.
                let observed = canonical(&result);
                assert_eq!(
                    expected
                        .entry(revision)
                        .or_insert_with(|| observed.clone())
                        .as_str(),
                    observed.as_str()
                );
                let imports = OPERATIONS
                    .iter()
                    .zip(samples)
                    .map(|(name, sample)| serde_json::json!({"operation":name,"sample":sample}))
                    .collect::<Vec<_>>();
                rows.push(serde_json::json!({
                    "probeEnabled":enabled,"freshHost":fresh,"revision":revision,
                    "phase":if revision == 1 {"first-parse-after-initialization"} else {"same-instance-repeat"},
                    "hostSetupMs":host_setup_ms,"parseMs":parse_ms,
                    "actualGuestHookCalls":actual_guest_calls,"positiveVariableTypeEntries":positive_variable_types,
                    "adoptedSyntaxTraceCalls":result.syntax_calls.len(),
                    "hostIncrementalHits":metrics.hits,"hostHeaderHits":metrics.header_hits,
                    "observationSha256":format!("{:x}",Sha256::digest(observed.as_bytes())),
                    "imports":imports,
                }));
            }
        }
    }
    println!(
        "catalog_boundary_measurement {}",
        serde_json::json!({
            "sourceSha256":format!("{:x}",Sha256::digest(SOURCE.as_bytes())),
            "sourceBytes":SOURCE.len(),"catalogSetupMs":catalog_setup_ms,
            "initializationTraffic":"unobserved: probe installed after ParserHost::new",
            "returnedRawBytes":"read_record raw payload only; WIT string/list overhead is not included",
            "hostTimer":"callback work after argument lifting and before return lowering; excludes guest JSON decode",
            "cold":"fresh guest instance after initialization; shared Wasmtime compilation may already be warm",
            "hostReuse":"disabled in test policy; native parse and Core immutable caches remain active",
            "rows":rows,
        })
    );
}

//! Test-only, opt-in host callback accounting. Timers start after import argument
//! lifting and end before return lowering; they do not measure guest JSON work.
//! Counters are observations outside StateStore, so rejected work is not erased.

use super::*;
use crate::bindings::nlaocs::skript_parser_addon::types::HookInvocation;
use std::time::Instant;
mod trace;

#[derive(Clone, Default)]
pub(super) struct Probe(Arc<Mutex<Measurement>>);

#[derive(Default)]
struct Measurement {
    imports: [ImportSample; 8],
    hooks: BTreeMap<String, HookSample>,
    active: Vec<String>,
    max_hook_depth: usize,
    scopes: BTreeMap<&'static str, ScopeSample>,
    scope_stack: Vec<(&'static str, u64)>,
    input_bytes_at_clone_sites: BTreeMap<&'static str, u64>,
    sampling: Option<Sampling>,
    trace: Option<trace::Trace>,
    type_input_repairs: u64,
    type_input_repair_bytes: u64,
}

struct Sampling {
    subscription: String,
    seen: u64,
    samples: Vec<serde_json::Value>,
    contexts: Vec<SampleContext>,
}

#[derive(Clone, Copy)]
struct SampleContext {
    dynamic_revisions: Option<(u64, u64)>,
    function_savepoint: Option<skript_parser::FunctionRegistrySavepoint>,
    namespace_complete: bool,
}

pub(super) struct ContextGuard(Probe);
impl Drop for ContextGuard {
    fn drop(&mut self) {
        self.0.0.lock().unwrap().sampling.as_mut().unwrap().contexts.pop().unwrap();
    }
}

pub(super) struct IdentitySample { probe: Probe, index: usize }
impl IdentitySample {
    pub(super) fn finish(self, invocation: &InvocationTransaction) {
        let _scope = self.probe.scope("probe.identity_sampling");
        let dependencies = invocation.read_write_set();
        let mut measurement = self.probe.0.lock().unwrap();
        let sample = &mut measurement.sampling.as_mut().unwrap().samples[self.index];
        sample["postDependencies"] = fingerprint(&dependencies);
        sample["reads"] = dependencies.reads.len().into();
        sample["writes"] = dependencies.writes.len().into();
        sample["namespaceRevisions"] = dependencies.namespace_revisions.len().into();
    }
}

// Hash at most1MiB per field, with no full Debug string allocation. A capped
// fingerprint deliberately has no digest and cannot be used for equality.
fn fingerprint<T: std::fmt::Debug + ?Sized>(value: &T) -> serde_json::Value {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    struct Writer { hash: Sha256, bytes: usize, truncated: bool }
    impl std::fmt::Write for Writer {
        fn write_str(&mut self, value: &str) -> std::fmt::Result {
            if value.len() > (1 << 20) - self.bytes {
                self.truncated = true;
                return Err(std::fmt::Error);
            }
            self.hash.update(value.as_bytes()); self.bytes += value.len(); Ok(())
        }
    }
    let mut writer = Writer { hash: Sha256::new(), bytes: 0, truncated: false };
    let complete = write!(&mut writer, "{value:?}").is_ok() && !writer.truncated;
    serde_json::json!({"complete":complete,"bytes":writer.bytes,
        "sha256":complete.then(||format!("{:x}",writer.hash.finalize()))})
}

pub(super) fn input_bytes(payload: &HookPayload) -> usize {
    input(payload).map_or(0, str::len)
}
fn input(payload: &HookPayload) -> Option<&str> {
    match payload {
        HookPayload::Expression(value) => Some(&value.input),
        HookPayload::RegisteredExpression(value) => Some(&value.input),
        HookPayload::Effect(value) => Some(&value.input),
        HookPayload::Condition(value) => Some(&value.input),
        HookPayload::Structure(value) => Some(&value.input),
        HookPayload::Matching(value) => Some(&value.input),
        HookPayload::Section(value) => Some(&value.input),
        HookPayload::Parser(value) => Some(&value.input),
        _ => None,
    }
}

#[derive(Default, serde::Serialize)]
struct ScopeSample {
    calls: u64,
    inclusive_ns: u64,
    exclusive_ns: u64,
}

pub(super) struct ScopeTimer {
    probe: Probe,
    name: &'static str,
    started: Instant,
    trace: Option<trace::Guard>,
}

impl Drop for ScopeTimer {
    fn drop(&mut self) {
        let ended = Instant::now();
        let ns = u64::try_from(ended.duration_since(self.started).as_nanos()).unwrap_or(u64::MAX);
        let mut measurement = self.probe.0.lock().unwrap();
        let (name, children) = measurement.scope_stack.pop().unwrap();
        assert_eq!(name, self.name);
        let sample = measurement.scopes.entry(name).or_default();
        sample.calls += 1;
        sample.inclusive_ns = sample.inclusive_ns.saturating_add(ns);
        sample.exclusive_ns = sample.exclusive_ns.saturating_add(ns.saturating_sub(children));
        if let Some((_, children)) = measurement.scope_stack.last_mut() {
            *children = children.saturating_add(ns);
        }
        drop(measurement);
        if let Some(trace) = self.trace.take() { trace.finish(ended,None); }
    }
}

impl ParserHost {
    pub(super) fn measurement_probe(&self) -> Option<Probe> {
        self.components.iter().find_map(|entry| entry.store.data().catalog_measurement.clone())
    }
    pub(super) fn measurement_copy(&self, name: &'static str, bytes: usize) -> Option<ScopeTimer> {
        self.measurement_probe().map(|probe| {
            let mut measurement = probe.0.lock().unwrap();
            *measurement.input_bytes_at_clone_sites.entry(name).or_default() += bytes as u64;
            drop(measurement); probe.scope_bytes(name,Some(bytes as u64))
        })
    }
    pub(super) fn measurement_scope(&self, name: &'static str) -> Option<ScopeTimer> {
        self.components.iter().find_map(|entry| entry.store.data().catalog_measurement.as_ref())
            .map(|probe| probe.scope(name))
    }
}

#[derive(Default, serde::Serialize)]
struct HookSample {
    calls: u64,
    wall_ns: u64,
    fuel_consumed: u64,
    fuel_unavailable: u64,
    input_string_bytes: u64,
    max_input_string_bytes: usize,
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

const TRACE_OPERATIONS: [&str; 8] = [
    "catalog.source", "catalog.records_by_registration_id", "catalog.read_record",
    "catalog.type_for_class", "catalog.change_contract_for_type", "catalog.read_record_fields",
    "catalog.declared_method_exists", "catalog.registered_handler_matches",
];

#[derive(Clone, Copy, Default, serde::Serialize)]
struct ImportSample {
    calls: u64,
    host_ns: u64,
    returned_raw_bytes: u64,
}

impl Probe {
    pub(super) fn type_input_repair(&self, bytes: usize) {
        let mut measurement = self.0.lock().unwrap();
        measurement.type_input_repairs += 1;
        measurement.type_input_repair_bytes += bytes as u64;
    }
    pub(super) fn context(&self, dynamic_revisions: Option<(u64, u64)>,
        function_savepoint: Option<skript_parser::FunctionRegistrySavepoint>,
        namespace_complete: bool) -> Option<ContextGuard> {
        let mut measurement = self.0.lock().unwrap();
        measurement.sampling.as_mut()?.contexts.push(SampleContext {
            dynamic_revisions, function_savepoint, namespace_complete,
        });
        Some(ContextGuard(self.clone()))
    }

    pub(super) fn sample_before(&self, transaction: &ParseTransaction, invocation: &HookInvocation,
        incremental_key: Option<&str>, interruption_epoch: Option<u64>) -> Option<IdentitySample> {
        let (index, sequence, context) = {
            let mut measurement = self.0.lock().unwrap();
            let sampling = measurement.sampling.as_mut()?;
            if sampling.subscription != invocation.context.subscription_id { return None; }
            sampling.seen += 1;
            if sampling.samples.len() >= 64 || (sampling.seen > 8 && sampling.seen % 1536 != 0) { return None; }
            let index = sampling.samples.len(); sampling.samples.push(serde_json::Value::Null);
            (index, sampling.seen, sampling.contexts.last().copied())
        };
        let _scope = self.scope("probe.identity_sampling");
        // Do not retain an extra Arc-backed state snapshot into guest execution.
        let before_state = { let snapshot = transaction.savepoint().unwrap(); fingerprint(&snapshot) };
        let sample = serde_json::json!({"sequence":sequence,"invocationId":invocation.context.invocation_id,
            "documentRevision":invocation.context.document_revision,"syntaxContext":invocation.context.syntax_context,
            "target":fingerprint(&invocation.target),"phase":format!("{:?}",invocation.phase),"cacheHit":false,
            "expansion":format!("{:?}",invocation.context.expansion),"payload":fingerprint(&invocation.payload),
            "input":input(&invocation.payload).map(fingerprint),"inputStringBytes":input_bytes(&invocation.payload),
            "parseResults":fingerprint(&invocation.parse_results),"stateRevision":transaction.state_revision().unwrap(),
            "beforeState":before_state,"incrementalKey":incremental_key.map(fingerprint),
            "interruptionEpoch":interruption_epoch,
            "dynamicDocumentRevision":context.and_then(|c|c.dynamic_revisions.map(|r|r.0)),
            "dynamicRegistryRevision":context.and_then(|c|c.dynamic_revisions.map(|r|r.1)),
            "functionSavepoint":context.map(|c|fingerprint(&c.function_savepoint)),
            "functionNamespaceComplete":context.map(|c|c.namespace_complete)});
        self.0.lock().unwrap().sampling.as_mut().unwrap().samples[index] = sample;
        Some(IdentitySample { probe: self.clone(), index })
    }
    pub(super) fn scope(&self, name: &'static str) -> ScopeTimer {
        self.scope_bytes(name,None)
    }

    fn scope_bytes(&self, name: &'static str, bytes: Option<u64>) -> ScopeTimer {
        let trace = {
            let mut measurement = self.0.lock().unwrap();
            measurement.scope_stack.push((name,0)); measurement.trace.clone()
        };
        let started = Instant::now();
        let trace = trace.and_then(|trace|trace.begin(name,trace::Kind::Scope,started,bytes));
        ScopeTimer { probe:self.clone(),name,started,trace }
    }

    pub(super) fn start(&self, operation: Operation) -> Call {
        let (hook,trace) = {
            let measurement = self.0.lock().unwrap();
            (measurement.active.last().cloned(),measurement.trace.clone())
        };
        let started = Instant::now();
        let trace = trace.and_then(|trace|trace.begin(TRACE_OPERATIONS[operation as usize],trace::Kind::Catalog,started,None));
        Call {
            probe: self.clone(),
            operation,
            hook,
            started,
            returned_raw_bytes: 0,
            trace,
        }
    }

    fn take(&self) -> [ImportSample; 8] {
        self.take_all().imports
    }

    fn take_all(&self) -> Measurement {
        let mut measurement = self.0.lock().unwrap();
        assert!(measurement.active.is_empty(), "snapshot during active hook");
        assert!(measurement.scope_stack.is_empty(), "snapshot during active scope");
        assert!(measurement.sampling.as_ref().is_none_or(|sampling|sampling.contexts.is_empty()));
        std::mem::take(&mut *measurement)
    }

    pub(super) fn hook(&self, subscription: &str, invocation: &HookInvocation, fuel: Option<u64>) -> HookTimer {
        let key = match &invocation.payload {
            HookPayload::RegisteredExpression(payload) => format!("{subscription}/{}#{}", payload.registration_id, payload.pattern_index),
            _ => subscription.to_owned(),
        };
        let mut timer = self.begin_hook(key, fuel);
        timer.input_bytes = input_bytes(&invocation.payload);
        timer
    }

    fn begin_hook(&self, key: String, fuel: Option<u64>) -> HookTimer {
        {
            let mut measurement = self.0.lock().unwrap();
            measurement.hooks.entry(key.clone()).or_default();
            measurement.active.push(key.clone());
            measurement.max_hook_depth = measurement.max_hook_depth.max(measurement.active.len());
        }
        HookTimer { probe: self.clone(), key, fuel, started: Instant::now(), input_bytes: 0 }
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
    assert_eq!(result.max_hook_depth, 2);
    assert!(probe.take_all().hooks.is_empty());
}

#[test]
fn nested_measurement_scopes_do_not_double_count_children() {
    let probe = Probe::default();
    let outer = probe.scope("outer");
    drop(probe.scope("inner"));
    drop(outer);
    let result = probe.take_all();
    assert_eq!(result.scopes["outer"].calls, 1);
    assert_eq!(result.scopes["inner"].calls, 1);
    assert_eq!(result.scopes["inner"].inclusive_ns, result.scopes["inner"].exclusive_ns);
    assert_eq!(result.scopes["outer"].inclusive_ns,
        result.scopes["outer"].exclusive_ns + result.scopes["inner"].inclusive_ns);
}

#[test]
fn incomplete_fingerprints_never_provide_an_identity_digest() {
    let a = fingerprint("bounded");
    assert_eq!(a["complete"], true);
    assert_eq!(a, fingerprint("bounded"));
    assert_ne!(a["sha256"], fingerprint("different")["sha256"]);
    let capped = fingerprint(&"x".repeat(1 << 20));
    assert_eq!(capped["complete"], false);
    assert!(capped["sha256"].is_null());
    assert!(capped["bytes"].as_u64().unwrap() <= 1 << 20);
}

#[test]
fn owned_native_measurements_close_on_error_and_unwind() {
    fn failing(probe: &Probe) -> Result<(), ()> {
        let _scope = skript_parser::ParserMeasurementScope::new(probe.scope("native.error"));
        Err(())?;
        Ok(())
    }
    let probe = Probe::default();
    let outer = probe.scope("outer");
    assert!(failing(&probe).is_err());
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _scope = skript_parser::ParserMeasurementScope::new(probe.scope("native.unwind"));
        panic!("diagnostic scope unwind");
    }));
    assert!(panic.is_err());
    drop(outer);
    let result = probe.take_all();
    assert_eq!(result.scopes["outer"].calls, 1);
    assert_eq!(result.scopes["outer"].inclusive_ns,
        result.scopes["outer"].exclusive_ns + result.scopes["native.error"].inclusive_ns
            + result.scopes["native.unwind"].inclusive_ns);
}

#[test]
#[ignore = "bounded public-document attribution; requires SKRIPT_DOCUMENT_MEASUREMENT_PLAN"]
fn measure_public_document_phases() {
    use sha2::{Digest, Sha256};
    use std::time::Duration;
    let plan: serde_json::Value = serde_json::from_slice(&std::fs::read(
        std::env::var_os("SKRIPT_DOCUMENT_MEASUREMENT_PLAN").unwrap(),
    ).unwrap()).unwrap();
    assert_eq!(plan["publicFixtures"], true);
    let core = std::fs::read(plan["component"].as_str().unwrap()).unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&core)), plan["coreSha256"]);
    let files = plan["snapshotFiles"].as_array().unwrap();
    assert_eq!(files.len(), 21);
    let verify_snapshot = || {
        for file in files {
            let relative = std::path::Path::new(file["name"].as_str().unwrap());
            assert!(relative.is_relative() && relative.components().all(|part|
                matches!(part, std::path::Component::Normal(_))));
            let bytes = std::fs::read(std::path::Path::new(plan["snapshot"].as_str().unwrap()).join(relative)).unwrap();
            assert_eq!(format!("{:x}", Sha256::digest(bytes)), file["sha256"]);
        }
    };
    verify_snapshot();
    let catalog = Arc::new(ssg::load(plan["snapshot"].as_str().unwrap()).unwrap().into_catalog());
    let cases = plan["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 3);
    let compare_original = plan["compareDispatchOriginal"].as_bool().unwrap_or(false);
    let compare_type = plan["compareTypeExpected"].as_bool().unwrap_or(false);
    assert!(!(compare_original && compare_type), "compare one optimization at a time");
    let comparison = compare_original || compare_type;
    let trace_directory = plan["traceDirectory"].as_str().map(std::path::PathBuf::from);
    assert!(!(comparison && trace_directory.is_some()), "trace and legacy comparison are separate investigations");
    let blocks = if comparison { 2 } else { 1 };
    let mut rows = Vec::new();
    for (index, case) in cases.iter().enumerate() {
        let source = std::fs::read_to_string(case["path"].as_str().unwrap()).unwrap();
        assert!(source.len() <= 150_000);
        assert_eq!(format!("{:x}", Sha256::digest(source.as_bytes())), case["sha256"]);
        let mut expected = None;
        for block in 0..blocks {
        let mut variants = if trace_directory.is_some() && plan["includeProbeOff"] == true {
            vec![(false, false), (true, false), (true, true)]
        } else {
            vec![(trace_directory.is_some() || comparison, false), (true, true)]
        };
        if (index + block) % 2 != 0 { variants.reverse(); }
        for (enabled, variant) in variants {
            let setup = Instant::now();
            let mut host = ParserHost::new(&core, HostConfig {
                syntax_catalog: Some(Arc::clone(&catalog)), ..Default::default()
            }).unwrap();
            host.legacy_dispatch_original = compare_original && !variant;
            host.legacy_type_expected = compare_type && !variant;
            let setup_ms = setup.elapsed().as_secs_f64() * 1000.0;
            let probe = Probe::default();
            if enabled {
                if trace_directory.is_none() {
                    probe.0.lock().unwrap().sampling = Some(Sampling {
                        subscription: plan["sampleSubscription"].as_str().unwrap().to_owned(),
                        seen: 0, samples: Vec::new(), contexts: Vec::new(),
                    });
                }
                for entry in &mut host.components {
                    entry.store.data_mut().catalog_measurement = Some(probe.clone());
                }
            }
            let before_calls = guest_calls(&host);
            let config = DocumentParserConfig::default();
            let cancellation = config.cancellation.clone();
            let (stop, deadline) = std::sync::mpsc::channel();
            let timer = std::thread::spawn(move || {
                if deadline.recv_timeout(Duration::from_secs(180)).is_err() { cancellation.cancel(); }
            });
            let parse = Instant::now();
            if trace_directory.is_some() && variant {
                probe.0.lock().unwrap().trace = Some(trace::Trace::new(parse,20_000,2_000_000));
            }
            let mut headers = Vec::new();
            let parsed = host.parse_document_with_preparation(
                DocumentParseRequest::new("file:///document-measurement", "file:///document-measurement/public.sk", 1, source.as_str()),
                config,
                &mut |event| {
                    if let skript_parser::StructurePreparationEvent::Headers { headers: items, .. } = event {
                        assert!(headers.len() < 16);
                        let measurement = probe.0.lock().unwrap();
                        headers.push(serde_json::json!({"ms":parse.elapsed().as_secs_f64()*1000.0,"headers":items.len(),
                            "regularHookCalls":measurement.hooks.values().map(|sample|sample.calls).sum::<u64>(),
                            "regularHookWallNsInclusive":measurement.hooks.values().map(|sample|sample.wall_ns).sum::<u64>()}));
                    }
                },
            );
            let parse_elapsed = parse.elapsed();
            let parse_ms = parse_elapsed.as_secs_f64() * 1000.0;
            let _ = stop.send(()); timer.join().unwrap();
            let result = parsed.unwrap();
            assert_eq!(result.execution_status, DocumentExecutionStatus::Complete);
            assert!(result.component_failures.is_empty() && result.fault_regions.is_empty()
                && result.dependency_unavailable.is_empty());
            let measurement = probe.take_all();
            let regular_calls = guest_calls(&host) - before_calls;
            assert_eq!(measurement.hooks.values().map(|sample|sample.calls).sum::<u64>(),
                if enabled { regular_calls } else { 0 });
            if enabled {
                assert_eq!(measurement.scopes.values().map(|sample|sample.exclusive_ns).sum::<u64>(),
                    measurement.scopes["document.total"].inclusive_ns);
            }
            // Semantic controls are outside all parse/probe timers. Retain faults,
            // availability, original mapping and the existing normalized proof/AST view.
            let observed = format!("{}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}", canonical(&result),
                result.execution_status, result.fault_regions, result.dependency_unavailable,
                result.component_failures, result.raw_tree, result.source);
            let digest = format!("{:x}", Sha256::digest(observed.as_bytes()));
            assert_eq!(expected.get_or_insert_with(|| digest.clone()).as_str(), digest.as_str(),
                "probe must preserve the complete semantic observation");
            // Export is outside parse/probe timers. Only fixed labels and numeric
            // counters enter trace files, never source/payload or identity samples.
            let export_started = Instant::now();
            let trace_export = measurement.trace.as_ref().map(|trace| {
                let name = case["name"].as_str().unwrap();
                assert!(["small","medium","large"].contains(&name));
                trace.write(&trace_directory.as_ref().unwrap().join(format!("{name}.trace.json")),
                    u64::try_from(parse_elapsed.as_nanos()).unwrap())
            });
            let export_ms = export_started.elapsed().as_secs_f64()*1000.0;
            let row = serde_json::json!({"case":case["name"],"probeEnabled":enabled,"block":block,
                "traceEnabled":measurement.trace.is_some(),"traceExport":trace_export,"traceExportMs":export_ms,
                "dispatchOriginal":if host.legacy_dispatch_original { "legacy" } else { "retained" },
                "typeExpected":if host.legacy_type_expected { "legacy" } else { "borrowed-input" },
                "typeInputRepairCopies":measurement.type_input_repairs,"typeInputRepairBytes":measurement.type_input_repair_bytes,
                "sourceSha256":case["sha256"],"sourceBytes":source.len(),"setupMs":setup_ms,"parseMs":parse_ms,
                "headerObservations":headers,"regularHookCalls":regular_calls,
                "maxHookDepth":measurement.max_hook_depth,"scopes":measurement.scopes,
                "inputBytesAtCloneSites":measurement.input_bytes_at_clone_sites,
                "identitySamples":measurement.sampling.map(|sampling|sampling.samples),
                "imports":OPERATIONS.iter().zip(measurement.imports).map(|(name,sample)|
                    serde_json::json!({"operation":name,"sample":sample})).collect::<Vec<_>>(),
                "hooks":measurement.hooks,"observationSha256":digest,"rawNodes":result.raw_tree.nodes.len(),
                "astNodes":result.ast.nodes.len(),"syntaxDiagnostics":result.syntax.diagnostics.len()});
            eprintln!("document_phase_observation {row}");
            rows.push(row);
        }
        }
    }
    verify_snapshot();
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true)
        .open(plan["output"].as_str().unwrap()).unwrap();
    file.write_all(&serde_json::to_vec_pretty(&serde_json::json!({"rows":rows,
        "schedule":if trace_directory.is_some() && plan["includeProbeOff"] == true { "probe off / aggregate only / aggregate plus trace per fixture; reverse order for medium; one actual trace per fixture" }
            else if trace_directory.is_some() { "one trace-off/on pair per fixture, same aggregate counters on both; one actual trace per fixture; reverse order for medium" }
            else if compare_type { "two blocks ABBA/BAAB per fixture, legacy Type expectation clone versus borrowed native input, priority1 retained and probes on for both" }
            else if compare_original { "two blocks ABBA/BAAB per fixture, legacy versus retained original, probes on for both" } else { "one off/on pair per fixture" },
        "limits":"Same optimized test executable and saved Core. Fresh hosts retain default incremental declarations; same-process compilation caches can warm. Scope exclusive times exclude nested instrumented scopes only. Regular hook wall includes WIT and all imports; measured Catalog callbacks exclude argument lifting and return lowering. Other imports and guest/ABI costs remain unseparated. Initialization and canonicalization are outside parse timers."})).unwrap()).unwrap();
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
    input_bytes: usize,
}

impl HookTimer {
    pub(super) fn finish(self, remaining: Option<u64>) {
        let wall_ns = u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let mut measurement = self.probe.0.lock().unwrap();
        assert_eq!(measurement.active.pop().as_deref(), Some(self.key.as_str()));
        let sample = measurement.hooks.get_mut(&self.key).unwrap();
        sample.calls += 1;
        sample.input_string_bytes += self.input_bytes as u64;
        sample.max_input_string_bytes = sample.max_input_string_bytes.max(self.input_bytes);
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
    trace: Option<trace::Guard>,
}

impl Call {
    pub(super) fn returned_bytes(&mut self, bytes: usize) {
        self.returned_raw_bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        let ended = Instant::now();
        let elapsed = u64::try_from(ended.duration_since(self.started).as_nanos()).unwrap_or(u64::MAX);
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
        drop(samples);
        if let Some(trace) = self.trace.take() { trace.finish(ended,Some(self.returned_raw_bytes)); }
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

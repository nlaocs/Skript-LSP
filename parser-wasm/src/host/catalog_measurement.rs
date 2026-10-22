//! Test-only, opt-in host callback accounting. Timers start after import argument
//! lifting and end before return lowering; they do not measure guest JSON work.
//! Counters are observations outside StateStore, so rejected work is not erased.

use super::*;
use std::time::Instant;

#[derive(Clone, Default)]
pub(super) struct Probe(Arc<Mutex<[ImportSample; 5]>>);

#[derive(Clone, Copy)]
pub(super) enum Operation {
    Source,
    RecordsByRegistrationId,
    ReadRecord,
    TypeForClass,
    ChangeContractForType,
}

const OPERATIONS: [&str; 5] = [
    "source",
    "records_by_registration_id",
    "read_record",
    "type_for_class",
    "change_contract_for_type",
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
            started: Instant::now(),
            returned_raw_bytes: 0,
        }
    }

    fn take(&self) -> [ImportSample; 5] {
        std::mem::take(
            &mut *self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

pub(super) struct Call {
    probe: Probe,
    operation: Operation,
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
        let sample = &mut samples[self.operation as usize];
        sample.calls = sample.calls.saturating_add(1);
        sample.host_ns = sample.host_ns.saturating_add(elapsed);
        sample.returned_raw_bytes = sample
            .returned_raw_bytes
            .saturating_add(self.returned_raw_bytes);
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

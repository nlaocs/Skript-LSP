//! Actual Core/WASM cold-versus-reused document equivalence. The only canonical
//! exclusions are private per-parse receipt IDs and opaque parse-result tokens.
use super::*;

fn actual_host() -> ParserHost {
    actual_host_with_fixture("skript-2.16.2-java25")
}

fn actual_host_with_fixture(name: &str) -> ParserHost {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/type-parser-versions").join(name);
    ParserHost::new(
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../artifacts/core-library.wasm"
        )),
        HostConfig {
            syntax_catalog: Some(Arc::new(ssg::load(fixture).unwrap().into_catalog())),
            ..HostConfig::default()
        },
    )
    .unwrap()
}

fn run(host: &mut ParserHost, source: &str, revision: u64) -> DocumentParseResult {
    host.parse_document(
        DocumentParseRequest::new(
            "file:///workspace",
            "file:///workspace/actual-incremental.sk",
            revision,
            source,
        ),
        DocumentParserConfig::default(),
    )
    .unwrap()
}

fn guest_calls(host: &ParserHost) -> u64 {
    host.components
        .iter()
        .map(|entry| entry.store.data().guest_hook_calls)
        .sum()
}

fn canonical_ast(tree: &AstTree) -> String {
    let mut tree = tree.clone();
    for node in &mut tree.nodes {
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
    format!("{tree:?}")
}

fn canonical_effects(effects: &HookEffects) -> String {
    let mut effects = effects.clone();
    let mut receipts = BTreeMap::new();
    // These are opaque invocation-resource handles, not semantic IDs. The
    // cache never reuses AdditionalParse results and native lookups rerun.
    for result in &mut effects.parse_results {
        result.host_token = 0;
        for node in &mut result.nodes {
            for entry in node.metadata.iter_mut().chain(node.summary.iter_mut().flat_map(|summary| &mut summary.metadata)) {
                if entry.owner_component_id.is_none() && entry.key == "parser.semantic-writer-receipt.v1" {
                    // Host-lifetime labels differ even for two cold parses. Keep
                    // equality/alias topology, without comparing numeric labels.
                    let next = receipts.len();
                    let canonical = receipts.entry(entry.value.clone()).or_insert(next);
                    entry.value = format!("receipt:{canonical}");
                }
            }
        }
    }
    format!("{effects:?}")
}

fn assert_equivalent(warm: &DocumentParseResult, clean: &DocumentParseResult) {
    assert!(
        warm.component_failures.is_empty(),
        "{:?}",
        warm.component_failures
    );
    assert!(
        clean.component_failures.is_empty(),
        "{:?}",
        clean.component_failures
    );
    assert_eq!(warm.execution_status, clean.execution_status);
    assert_eq!(warm.source.original(), clean.source.original());
    assert_eq!(warm.source.virtual_source(), clean.source.virtual_source());
    assert_eq!(
        format!("{:?}", warm.source.source_map()),
        format!("{:?}", clean.source.source_map())
    );
    assert_eq!(canonical_ast(&warm.ast), canonical_ast(&clean.ast));
    assert_eq!(
        format!("{:?}", warm.syntax.diagnostics),
        format!("{:?}", clean.syntax.diagnostics)
    );
    assert_eq!(
        canonical_effects(&warm.effects),
        canonical_effects(&clean.effects)
    );
    assert_eq!(
        format!("{:?}", warm.functions),
        format!("{:?}", clean.functions),
        "current function definitions and complete positive/negative lookup observations must match"
    );
    assert_eq!(warm.state.writes, clean.state.writes);
    assert_eq!(
        warm.state.read_write_set.reads,
        clean.state.read_write_set.reads
    );
    assert_eq!(
        warm.state.read_write_set.writes,
        clean.state.read_write_set.writes
    );
    assert_eq!(
        warm.state
            .read_write_set
            .namespace_revisions
            .keys()
            .collect::<Vec<_>>(),
        clean
            .state
            .read_write_set
            .namespace_revisions
            .keys()
            .collect::<Vec<_>>(),
        "current namespace dependency keys must match; distinct stores need not share clock values"
    );
    let trace = |result: &DocumentParseResult| {
        result
            .syntax_calls
            .iter()
            .map(|call| (call.component_id.clone(), call.subscription_id.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        trace(warm),
        trace(clean),
        "logical adopted call audit remains current"
    );
    let warm_proof = warm.semantic_witnesses();
    let clean_proof = clean.semantic_witnesses();
    assert_eq!(warm.ast.nodes.len(), clean.ast.nodes.len());
    for (left, right) in warm.ast.nodes.iter().zip(&clean.ast.nodes) {
        assert_eq!(
            warm_proof.native_valid(left),
            clean_proof.native_valid(right)
        );
        assert_eq!(
            warm_proof.unique_issuer(left),
            clean_proof.unique_issuer(right),
            "cache output must receive current semantic authority"
        );
    }
}

fn compare_case(
    name: &str,
    before: &str,
    current: &str,
    expect_hits: Option<bool>,
) -> (IncrementalMetrics, String) {
    let mut warm_host = actual_host();
    // These cases isolate invocation/header reuse. Body reuse has a separate
    // cold differential below and must not distort its guest-call accounting.
    warm_host.components[0].manifest.incremental_body_subscriptions.clear();
    assert!(
        !warm_host.components[0]
            .manifest
            .incremental_local_subscriptions
            .is_empty(),
        "test must exercise the actual advertised Core contract"
    );
    let first = run(&mut warm_host, before, 1);
    assert!(first.component_failures.is_empty());
    let before_calls = guest_calls(&warm_host);
    let warm = run(&mut warm_host, current, 2);
    let actual_calls = guest_calls(&warm_host) - before_calls;
    let metrics = warm_host.incremental_metrics();
    let mut clean_host = actual_host();
    clean_host.components[0].manifest.incremental_body_subscriptions.clear();
    // Host policy may decline reuse. The guest, native matcher, registry, ABI,
    // source and revision are otherwise the same as the tested warm request.
    clean_host.components[0]
        .manifest
        .incremental_local_subscriptions
        .clear();
    clean_host.components[0]
        .manifest
        .incremental_header_handlers
        .clear();
    let clean_before = guest_calls(&clean_host);
    let clean = run(&mut clean_host, current, 2);
    let clean_calls = guest_calls(&clean_host) - clean_before;
    assert_eq!(clean_host.incremental_metrics().hits, 0);
    assert_equivalent(&warm, &clean);
    assert_eq!(
        actual_calls + metrics.hits,
        clean_calls,
        "{name}: actual WASM hook counter must prove exactly the reported calls were skipped"
    );
    if let Some(expected) = expect_hits {
        assert!(
            !warm.ast.roots.is_empty(),
            "{name}: equivalence must not be vacuous empty parses"
        );
        let witnesses = warm.semantic_witnesses();
        assert!(
            warm.ast
                .nodes
                .iter()
                .any(|node| witnesses.provider_valid(node, CORE_LIBRARY_COMPONENT_ID)),
            "{name}: at least one actual Core semantic node must retain current authority"
        );
        assert_eq!(metrics.hits > 0, expected, "{name}: {metrics:?}");
    }
    eprintln!(
        "incremental-equivalence,{name},eligible={},hits={},actualGuestCalls={actual_calls},cleanGuestCalls={clean_calls}",
        metrics.eligible, metrics.hits
    );
    (metrics, canonical_ast(&warm.ast))
}

#[test]
fn real_core_body_reuses_individually_certified_typed_sets_with_current_context_and_offsets() {
    for fixture in ["skript-2.15.4-type-runtime", "skript-2.16.2-java25"] {
    let source = "function first():\n    send \"a\" to all players\nfunction second():\n    send \"b\" to all players\n";
    let mut host = actual_host_with_fixture(fixture);
    host.body_debug = true;
    host.components[0].manifest.incremental_local_subscriptions.clear();
    host.components[0].manifest.incremental_header_handlers.clear();
    let first = run(&mut host, source, 1);
    assert!(first.syntax.diagnostics.is_empty(), "{:?}", first.syntax.diagnostics);
    assert!(first.ast.nodes.iter().any(|node| node.summary.as_ref()
        .is_some_and(|summary| summary.return_type.as_deref() == Some("org.bukkit.entity.Player"))));
    assert!(host.incremental_metrics().body_stored > 0, "{:?}", host.incremental_metrics());
    let cases = [
        source.to_owned(),
        source.replacen("\"a\"", "\"c\"", 1),
        format!("# 日本語 😀\n{source}"),
        source.replace("all players", "all entities"),
        source.replace("\"a\"", "\"%all players%\""),
        source.replace("function first():", "on load:"),
    ];
    for (index, current) in cases.iter().enumerate() {
        let revision = index as u64 + 2;
        let before = guest_calls(&host);
        let warm = run(&mut host, current, revision);
        let calls = guest_calls(&host) - before;
        let metrics = host.incremental_metrics();
        let mut cold_host = actual_host_with_fixture(fixture);
        cold_host.components[0].manifest.incremental_body_subscriptions.clear();
        cold_host.components[0].manifest.incremental_body_handler_subscriptions.clear();
        cold_host.components[0].manifest.incremental_local_subscriptions.clear();
        cold_host.components[0].manifest.incremental_header_handlers.clear();
        let before = guest_calls(&cold_host);
        let cold = run(&mut cold_host, current, revision);
        assert_equivalent(&warm, &cold);
        eprintln!("typed body {fixture} case {index}: {metrics:?}; physical calls {calls}/{} cold", guest_calls(&cold_host) - before);
        if index < 2 {
            assert!(metrics.body_hits > 0, "{index}: {metrics:?}");
            assert!(calls < guest_calls(&cold_host) - before);
            let proof = warm.semantic_witnesses();
            assert!(warm.ast.nodes.iter().any(|node| node.summary.as_ref()
                .is_some_and(|summary| summary.return_type.as_deref() == Some("org.bukkit.entity.Player"))
                && proof.provider_valid(node, "nlaocs.core-library")),
                "reused typed result must retain current Core authority");
        }
    }
    }
}

#[test]
#[ignore = "bounded public fixture edit comparison; requires SKRIPT_BODY_MEASUREMENT_PLAN"]
fn measure_public_structure_body_edit_reuse() {
    use sha2::{Digest, Sha256};
    use std::time::Instant;
    fn root_samples(result: &DocumentParseResult) -> Vec<serde_json::Value> {
        result.syntax.roots.iter().filter_map(|root| {
            let skript_parser::StructureDocumentNode::Structure(matches)=root else {return None;};
            let candidate=matches.selected.as_ref()?;
            let body=match &candidate.body {skript_parser::StructureBody::None=>"none",skript_parser::StructureBody::Raw(_)=>"raw",
                skript_parser::StructureBody::Entries(_)=>"entries",skript_parser::StructureBody::Trigger(_)=>"trigger"};
            // Post-parse examples only, not admission authority or an attempt
            // denominator. Bound serialized origins and never emit source text.
            let mapping=result.raw_tree.get(candidate.raw_node_id).and_then(|root| {
                let end=root.body_span.as_ref()?.virtual_range.end.max(root.span.virtual_range.end);
                let range=skript_parser::TextRange::new(root.span.virtual_range.start,end);
                let span=result.source.map_range(range).ok()?;
                let all_exact=span.origins.iter().all(|origin|origin.kind==skript_parser::OriginKind::Exact);
                let no_expansions=span.origins.iter().all(|origin|origin.expansion.is_none());
                // map_range flattens/deduplicates origins: only intersections
                // with the actual segments retain virtual coverage and order.
                let pieces=result.source.source_map().segments().iter().filter_map(|segment| {
                    let start=segment.virtual_range.start.max(range.start);
                    let end=segment.virtual_range.end.min(range.end);
                    if start>=end {return None;}
                    let overlap=skript_parser::TextRange::new(start,end);
                    let origins=segment.origins.iter().map(|origin| {
                        let mut origin=*origin;
                        if origin.kind==skript_parser::OriginKind::Exact {
                            let start=origin.original_range.start+overlap.start-segment.virtual_range.start;
                            origin.original_range=skript_parser::TextRange::new(start,start+overlap.len());
                        }
                        origin
                    }).collect::<Vec<_>>();
                    Some((overlap,origins))
                }).collect::<Vec<_>>();
                let bounded= !pieces.is_empty() && pieces.len()<=16
                    && pieces.iter().all(|(_,origins)|origins.len()==1);
                let exact_cover=bounded && pieces.first().unwrap().0.start==range.start
                    && pieces.last().unwrap().0.end==range.end
                    && pieces.windows(2).all(|pair|pair[0].0.end==pair[1].0.start)
                    && pieces.iter().all(|(virtual_range,origins)| {
                        let origin=origins[0];origin.kind==skript_parser::OriginKind::Exact
                            && origin.expansion.is_none() && origin.original_range.len()==virtual_range.len()
                            && origin.original_range.slice(result.source.original())
                                .zip(virtual_range.slice(result.source.virtual_source())).is_some_and(|(a,b)|a==b)
                    });
                let contiguous=exact_cover && pieces.windows(2)
                    .all(|pair|pair[0].1[0].original_range.end==pair[1].1[0].original_range.start);
                Some(serde_json::json!({"virtualStart":range.start,"virtualEnd":range.end,
                    "originCount":span.origins.len(),"allExact":all_exact,"noExpansions":no_expansions,
                    "boundedExactUnexpandedSegmentCoverage":exact_cover,"contiguousOriginalRanges":contiguous,
                    "segmentCount":pieces.len(),"segmentSamplesTruncated":pieces.len()>16,
                    "segmentSamples":pieces.iter().take(16).map(|(virtual_range,origins)|serde_json::json!({
                        "virtualStart":virtual_range.start,"virtualEnd":virtual_range.end,"originCount":origins.len(),
                        "originsTruncated":origins.len()>16,"origins":origins.iter().take(16).map(|origin|
                            serde_json::json!({"kind":format!("{:?}",origin.kind),"start":origin.original_range.start,
                                "end":origin.original_range.end,"hasExpansion":origin.expansion.is_some()})).collect::<Vec<_>>()
                    })).collect::<Vec<_>>(),
                    "truncated":span.origins.len()>16,"origins":span.origins.iter().take(16).map(|origin|
                        serde_json::json!({"kind":format!("{:?}",origin.kind),"start":origin.original_range.start,
                            "end":origin.original_range.end,"hasExpansion":origin.expansion.is_some()})).collect::<Vec<_>>()}))
            });
            Some(serde_json::json!({"rootId":candidate.raw_node_id.get(),"class":candidate.element_class.as_ref().map(|class|class.as_str()),
                "definition":candidate.matched.definition_id,"patternIndex":candidate.matched.pattern_index,
                "headerStart":candidate.matched.matched.span.mapped.virtual_range.start,
                "headerEnd":candidate.matched.matched.span.mapped.virtual_range.end,
                "bodyKind":body,"parsedHeaderCaptures":candidate.parsed_captures.len(),
                "retainedAlternatives":matches.alternatives.len(),"bodyRangeMappingExample":mapping}))
        }).take(16).collect()
    }
    fn signature(result: &DocumentParseResult) -> String {
        assert_eq!(result.execution_status, DocumentExecutionStatus::Complete);
        assert!(result.component_failures.is_empty(), "component failure count {}", result.component_failures.len());
        assert!(result.fault_regions.is_empty(), "fault count {}", result.fault_regions.len());
        assert!(result.dependency_unavailable.is_empty(), "missing dependency count {}", result.dependency_unavailable.len());
        let mut hash = Sha256::new();
        hash.update(result.source.original());
        hash.update(result.source.virtual_source());
        hash.update(format!("{:?}", result.source.source_map()));
        hash.update(canonical_ast(&result.ast));
        hash.update(canonical_effects(&result.effects));
        hash.update(format!("{:?}|{:?}|{:?}|{:?}|{:?}|{:?}", result.syntax.diagnostics,
            result.functions, result.state.writes, result.state.read_write_set.reads,
            result.state.read_write_set.writes, result.state.read_write_set.namespace_revisions.keys().collect::<Vec<_>>()));
        for call in &result.syntax_calls { hash.update(format!("{:?}|{:?}", call.component_id, call.subscription_id)); }
        let proof = result.semantic_witnesses();
        for node in &result.ast.nodes { hash.update(format!("{}|{}|{:?}", node.id, proof.native_valid(node), proof.unique_issuer(node))); }
        format!("{:x}", hash.finalize())
    }
    fn bounded_run(host: &mut ParserHost, source: &str, revision: u64) -> DocumentParseResult {
        let token = DocumentCancellationToken::new();
        let cancel = token.clone();
        let (done, wait) = std::sync::mpsc::channel();
        let timer = std::thread::spawn(move || {
            if wait.recv_timeout(std::time::Duration::from_secs(180)).is_err() { cancel.cancel(); }
        });
        let result = host.parse_document(
            DocumentParseRequest::new("file:///workspace", "file:///workspace/actual-incremental.sk", revision, source),
            DocumentParserConfig { cancellation: token, ..DocumentParserConfig::default() });
        let _ = done.send(());
        timer.join().unwrap();
        result.unwrap()
    }
    let plan: serde_json::Value = serde_json::from_slice(&std::fs::read(
        std::env::var("SKRIPT_BODY_MEASUREMENT_PLAN").unwrap()).unwrap()).unwrap();
    assert_eq!(plan["publicFixtures"], true);
    let admission_reasons=plan["admissionReasons"]==true;
    let compare_bodies=plan["compareBodyReuse"]==true;
    let catalog = Arc::new(ssg::load(plan["snapshot"].as_str().unwrap()).unwrap().into_catalog());
    for file in plan["snapshotFiles"].as_array().unwrap() {
        let bytes = std::fs::read(std::path::Path::new(plan["snapshot"].as_str().unwrap()).join(file["name"].as_str().unwrap())).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), file["sha256"].as_str().unwrap());
    }
    let mut rows = Vec::new();
    for (index, case) in plan["cases"].as_array().unwrap().iter().enumerate() {
        let source = std::fs::read_to_string(case["path"].as_str().unwrap()).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(source.as_bytes())), case["sha256"].as_str().unwrap());
        assert_eq!(source.matches("set {_edit_probe} to 1").count(), 1);
        let edited = source.replacen("set {_edit_probe} to 1", "set {_edit_probe} to 2", 1);
        let mut reference = None;
        for enabled in if index % 2 == 0 { [false, true] } else { [true, false] } {
            eprintln!("body measurement pid={} case={} enabled={enabled} initial", std::process::id(), case["name"]);
            let mut host = ParserHost::new(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../artifacts/core-library.wasm")),
                HostConfig { syntax_catalog: Some(Arc::clone(&catalog)), ..HostConfig::default() }).unwrap();
            if admission_reasons {
                host.body_admission_probe=(enabled || compare_bodies).then(body_admission_measurement::Probe::default);
                assert!(host.measurement_probe().is_none(),"counter-only measurement must not enable catalog timers");
            }
            if (!admission_reasons || compare_bodies) && !enabled {
                host.components[0].manifest.incremental_body_subscriptions.clear();
                host.components[0].manifest.incremental_body_handler_subscriptions.clear();
            }
            let initial_calls_before = guest_calls(&host);
            let start = Instant::now();
            let initial = bounded_run(&mut host, &source, 1);
            let initial_ms = start.elapsed().as_secs_f64() * 1000.0;
            let initial_calls = guest_calls(&host) - initial_calls_before;
            let initial_metrics = host.incremental_metrics();
            let initial_signature = signature(&initial);
            let initial_reasons=host.body_admission_probe.as_ref().map(|probe|probe.snapshot());
            if let Some(reasons)=&initial_reasons {
                assert_eq!(reasons["strictPartition"],true);
                for phase in ["finish","adopt","accept"] {assert_eq!(reasons[phase]["strictPartition"],true);}
                assert_eq!(reasons["unfinalizedObservations"],0);
            }
            if let Some(probe)=&host.body_admission_probe {probe.reset();}
            eprintln!("body measurement initial {initial_ms:.3}ms; {initial_metrics:?}; editing");
            let before = guest_calls(&host);
            let start = Instant::now();
            let current = bounded_run(&mut host, &edited, 2);
            let edit_ms = start.elapsed().as_secs_f64() * 1000.0;
            let edit_calls = guest_calls(&host) - before;
            let metrics = host.incremental_metrics();
            let edit_signature = signature(&current);
            let edit_reasons=host.body_admission_probe.as_ref().map(|probe|probe.snapshot());
            if let Some(reasons)=&edit_reasons {
                assert_eq!(reasons["strictPartition"],true);
                for phase in ["finish","adopt","accept"] {assert_eq!(reasons[phase]["strictPartition"],true);}
                assert_eq!(reasons["unfinalizedObservations"],0);
            }
            if let Some((initial, edited, calls_before, calls_after)) = &reference {
                assert_eq!(&initial_signature, initial, "initial semantic digest differs");
                assert_eq!(&edit_signature, edited, "edited semantic digest differs");
                if admission_reasons && !compare_bodies {assert_eq!(&initial_calls,calls_before);assert_eq!(&edit_calls,calls_after);}
            }
            rows.push(serde_json::json!({"case":case["name"], "bodyEnabled":(admission_reasons && !compare_bodies) || enabled,
                "reasonProbeEnabled":admission_reasons && (enabled || compare_bodies),"initialAdmission":initial_reasons,"editAdmission":edit_reasons,
                "retainedRootSamplesNotAttemptPartition":root_samples(&current),
                "initialMs":initial_ms,"editMs":edit_ms,"initialMetrics":format!("{initial_metrics:?}"),
                "editMetrics":format!("{metrics:?}"),"initialGuestCalls":initial_calls,"editGuestCalls":edit_calls,
                "initialDiagnostics":initial.syntax.diagnostics.len(),"editDiagnostics":current.syntax.diagnostics.len(),
                "initialSignature":initial_signature,"editSignature":edit_signature,
                "initialStatus":format!("{:?}",initial.execution_status),"editStatus":format!("{:?}",current.execution_status),
                "initialFailures":initial.component_failures.len(),"failures":current.component_failures.len(),
                "initialFaults":initial.fault_regions.len(),"editFaults":current.fault_regions.len(),
                "initialMissingDependencies":initial.dependency_unavailable.len(),"editMissingDependencies":current.dependency_unavailable.len(),
                "astNodes":current.ast.nodes.len(),"originalBytes":current.source.original().len(),
                "virtualBytes":current.source.virtual_source().len(),
                "acceptedTreeMacroCalls":current.tree_macro_calls.iter().filter(|call|call.accepted).count()}));
            eprintln!("body measurement edit {edit_ms:.3}ms; {metrics:?}");
            std::fs::write(format!("{}.partial", plan["output"].as_str().unwrap()),
                serde_json::to_vec_pretty(&rows).unwrap()).unwrap();
            if reference.is_none() { reference = Some((initial_signature, edit_signature, initial_calls, edit_calls)); }
        }
    }
    std::fs::write(plan["output"].as_str().unwrap(), serde_json::to_vec_pretty(&serde_json::json!({
        "rows":rows,"admissionReasonsMode":admission_reasons,"bodyReuseComparison":compare_bodies,
        "conditions":if compare_bodies {
            "One body reuse OFF/ON pair per public fixture, same native executable/Core; counter observer enabled both sides. Semantic digests must match; physical guest calls may decrease. Parse wall only, not UI latency or a permanent speedup claim. Retained roots are examples, not the attempt denominator."
        } else {"One pair per public fixture, same native executable/Core; parse wall only, not UI latency or a speedup claim. In admissionReasons mode body reuse remains enabled on both sides, only counter observation toggles. Retained roots are examples, not the attempt denominator."}})).unwrap()).unwrap();
}

#[test]
fn real_core_body_unlisted_registered_resolver_keeps_typed_assignments_cold() {
    let source = "function first() :: number:\n    set {_v} to 1\n    return {_v}\nfunction second() :: text:\n    set {_t} to \"a\"\n    return {_t}\n";
    let mut warm_host=actual_host();
    warm_host.body_debug=true;
    warm_host.components[0].manifest.incremental_local_subscriptions.clear();
    warm_host.components[0].manifest.incremental_header_handlers.clear();
    let first=run(&mut warm_host,source,1);
    assert!(first.syntax.diagnostics.is_empty(),"{:?}",first.syntax.diagnostics);
    let initial=warm_host.incremental_metrics();
    assert!(initial.body_eligible>0,"actual native body boundary must execute: {initial:?}");
    assert_eq!(initial.body_stored,0,"unlisted registered semantics must remain cold: {initial:?}");
    for (revision,current) in [(2,source.to_owned()),(3,source.replace("to 1","to 2"))] {
        let before=guest_calls(&warm_host);
        let warm=run(&mut warm_host,&current,revision);
        let actual_calls=guest_calls(&warm_host)-before;
        let metrics=warm_host.incremental_metrics();
        let mut cold_host=actual_host();
        cold_host.components[0].manifest.incremental_local_subscriptions.clear();
        cold_host.components[0].manifest.incremental_header_handlers.clear();
        cold_host.components[0].manifest.incremental_body_subscriptions.clear();
        let before=guest_calls(&cold_host);
        let cold=run(&mut cold_host,&current,revision);
        let cold_calls=guest_calls(&cold_host)-before;
        assert_equivalent(&warm,&cold);
        assert_eq!(metrics.body_hits,0,"{revision}: {metrics:?}");
        assert_eq!(actual_calls,cold_calls,"unlisted paths must not skip guest work");
    }
}

#[test]
fn real_core_body_reuse_matches_cold_for_stop_bodies_and_a_sibling_case_edit() {
    let source="function first():\n    stop\nfunction second():\n    stop\n";
    let mut warm_host=actual_host();
    warm_host.body_debug=true;
    warm_host.components[0].manifest.incremental_local_subscriptions.clear();
    warm_host.components[0].manifest.incremental_header_handlers.clear();
    let first=run(&mut warm_host,source,1);
    assert!(first.syntax.diagnostics.is_empty(),"{:?}",first.syntax.diagnostics);
    assert!(warm_host.incremental_metrics().body_stored>0,"{:?}",warm_host.incremental_metrics());
    for (revision,current) in [(2,source.to_owned()),(3,source.replacen("stop","Stop",1))] {
        let before=guest_calls(&warm_host);
        let warm=run(&mut warm_host,&current,revision);
        let actual=guest_calls(&warm_host)-before;
        let metrics=warm_host.incremental_metrics();
        let mut cold_host=actual_host();
        cold_host.components[0].manifest.incremental_local_subscriptions.clear();
        cold_host.components[0].manifest.incremental_header_handlers.clear();
        cold_host.components[0].manifest.incremental_body_subscriptions.clear();
        let before=guest_calls(&cold_host);
        let cold=run(&mut cold_host,&current,revision);
        assert_equivalent(&warm,&cold);
        assert!(metrics.body_hits>0,"{revision}: {metrics:?}");
        assert!(actual<guest_calls(&cold_host)-before);
        assert!(warm.syntax_calls.iter().any(|call|call.origin==HookCallOrigin::ReusedStructureBody));
    }
}

#[test]
fn real_core_body_reuses_unexpanded_functions_after_a_sibling_tree_edit() {
    for fixture in ["skript-2.15.4-type-runtime","skript-2.16.2-java25"] {
        let source="options:\n    marker: alpha\nfunction first():\n    send \"a\" to all players\nfunction second():\n    send \"b\" to all players\non load:\n    send \"{@marker}\" to console\n";
        let mut host=actual_host_with_fixture(fixture);
        host.body_debug=true;
        host.components[0].manifest.incremental_local_subscriptions.clear();
        host.components[0].manifest.incremental_header_handlers.clear();
        let first=run(&mut host,source,1);
        assert!(first.syntax.diagnostics.is_empty(),"{:?}",first.syntax.diagnostics);
        assert!(first.source.virtual_source().len()>source.len(),"must exercise actual TreeEdit projection");
        assert!(host.incremental_metrics().body_stored>=2,"{:?}",host.incremental_metrics());
        for (i,current) in [source.to_owned(),source.replacen("\"a\"","\"c\"",1),
            source.replace("alpha","bravo"),source.replace("alpha","longer label"),
            format!("# 😀é\r\n{source}"),source.replace("function first():","on join:")].iter().enumerate() {
            let before=guest_calls(&host);
            let warm=run(&mut host,current,i as u64+2);
            let calls=guest_calls(&host)-before;
            let metrics=host.incremental_metrics();
            let mut cold=actual_host_with_fixture(fixture);
            cold.components[0].manifest.incremental_body_subscriptions.clear();
            cold.components[0].manifest.incremental_body_handler_subscriptions.clear();
            cold.components[0].manifest.incremental_local_subscriptions.clear();
            cold.components[0].manifest.incremental_header_handlers.clear();
            let before=guest_calls(&cold);
            let clean=run(&mut cold,current,i as u64+2);
            assert_equivalent(&warm,&clean);
            if i<2 {
                assert!(metrics.body_hits>0,"{fixture}/{i}: {metrics:?}");
                assert!(calls<guest_calls(&cold)-before);
                let proof=warm.semantic_witnesses();
                assert!(warm.ast.nodes.iter().any(|node|node.summary.as_ref()
                    .is_some_and(|summary|summary.return_type.as_deref()==Some("org.bukkit.entity.Player"))
                    && proof.provider_valid(node,CORE_LIBRARY_COMPONENT_ID)),
                    "fragmented-source reuse must preserve current Core authority on a real typed node");
            }
        }
    }
}

#[test]
fn real_core_fragmented_body_keys_keep_document_revision_and_close_fences() {
    let source="options:\n    marker: a\nfunction first():\n    stop\non load:\n    send \"{@marker}\" to console\n";
    let mut host=actual_host();
    run(&mut host,source,1);
    run(&mut host,source,2);
    assert!(host.incremental_metrics().body_hits>0,"{:?}",host.incremental_metrics());
    let other=host.parse_document(DocumentParseRequest::new("file:///workspace","file:///workspace/other.sk",1,source),DocumentParserConfig::default()).unwrap();
    assert_eq!(host.incremental_metrics().body_hits,0,"a different document must not import the first document's bodies");
    let mut cold=actual_host();
    cold.components[0].manifest.incremental_body_subscriptions.clear();
    cold.components[0].manifest.incremental_body_handler_subscriptions.clear();
    run(&mut cold,source,1);
    run(&mut cold,source,2);
    let clean=cold.parse_document(DocumentParseRequest::new("file:///workspace","file:///workspace/other.sk",1,source),DocumentParserConfig::default()).unwrap();
    assert_equivalent(&other,&clean);
    assert!(host.parse_document(DocumentParseRequest::new("file:///workspace","file:///workspace/actual-incremental.sk",1,source),DocumentParserConfig::default()).is_err());
    assert!(host.release_document("file:///workspace","file:///workspace/actual-incremental.sk",2).unwrap());
    let reopened=run(&mut host,source,1);
    assert_eq!(host.incremental_metrics().body_hits,0);
    assert_eq!(reopened.execution_status,DocumentExecutionStatus::Complete);
    assert!(cold.release_document("file:///workspace","file:///workspace/actual-incremental.sk",2).unwrap());
    let clean_reopened=run(&mut cold,source,1);
    assert_equivalent(&reopened,&clean_reopened);
}

#[test]
fn real_core_cache_reuses_unchanged_event_and_nested_command_calls() {
    for (name, source) in [
        (
            "event",
            "on load:\n    set {_a} to 1\n    send 1 to console\n# old\n",
        ),
        (
            "command-nested",
            "command /incremental:\n    trigger:\n        if 1 is 1:\n            set {_a} to 1\n        send 1 to console\n# old\n",
        ),
        (
            "function-lookups",
            "function local(n: number) :: number:\n    return {_n}\non load:\n    send local(log(8)) to console\n    send 1 to console\n# old\n",
        ),
    ] {
        let current = source.replace("# old", "# new");
        let (metrics, _) = compare_case(name, source, &current, Some(true));
        assert!(metrics.stored > 0);
    }
}

#[test]
fn real_core_cache_changed_semantics_and_unaffected_ranges_match_clean() {
    let before = "on load:\n    send 1 to console\n    set {_a} to 1\n    send {_a} to console\n";
    let current =
        "on load:\n    send 1 to console\n    set {_a} to \"changed\"\n    send {_a} to console\n";
    compare_case("changed-local-value", before, current, Some(true));
    let before = "function local(n: number) :: number:\n    return {_n}\non load:\n    send local(log(8)) to console\n    send 1 to console\n";
    let current = before
        .replace("n: number", "n: string")
        .replace(":: number", ":: string");
    compare_case("changed-function-contract", before, &current, None);
}

#[test]
fn real_core_cache_context_offsets_and_macro_mapping_are_current() {
    let before = "on load:\n    send 1 to console\n";
    compare_case(
        "changed-event-context",
        before,
        &before.replace("on load", "on join"),
        Some(false),
    );
    compare_case(
        "shifted-absolute-spans",
        before,
        &format!("# inserted\n{before}"),
        Some(false),
    );
    let before = "options:\n    count: 1\non load:\n    send {@count} to console\n";
    let current = before.replace("count: 1", "count: 2");
    compare_case("tree-macro-current-map", before, &current, None);
}

#[test]
fn real_core_cache_uses_current_parse_private_aliases_and_owner_validation() {
    let before =
        "aliases:\n    building stone = stone\non load:\n    send building stone to console\n";
    let current = before.replace("building stone = stone", "building stone = dirt");
    let (_, ast) = compare_case("changed-parse-alias", before, &current, None);
    assert!(
        ast.contains("dirt"),
        "current alias payload must be visible, not only equal empty results"
    );
    let before = "command /incremental:\n    trigger:\n        send 1 to console\n";
    let current = "command /incremental:\n    permission: fixture.one\n    permission: fixture.two\n    trigger:\n        send 1 to console\n";
    compare_case("current-entry-rejection", before, current, None);
}

#[test]
fn real_core_header_reuse_rebinds_shifted_utf8_function_declarations() {
    let before = "function sample(n: number) :: number:\n    return {_n}\non load:\n    send sample(1) to console\n";
    let current = format!(
        "# \u{65e5}\u{672c}\u{8a9e}\u{1f98a}\n\n{}",
        before.replace("sample(1)", "sample(2)")
    );
    let (metrics, ast) = compare_case(
        "header-shifted-utf8-declaration",
        before,
        &current,
        Some(true),
    );
    assert!(
        metrics.header_eligible > 0,
        "actual Core header contract must be selected: {metrics:?}"
    );
    assert!(
        metrics.header_hits > 0,
        "actual Function/WASM Enter calls must be skipped: {metrics:?}"
    );
    assert!(
        ast.contains("skript.semantic.parameters"),
        "parameter metadata must survive current replay"
    );
}

#[test]
fn real_core_header_replay_runs_current_duplicate_registration_validation() {
    let before =
        "function sample(n: number) :: number:\n    return {_n}\non load:\n    send 1 to console\n";
    let current = format!("{before}function sample(n: number) :: number:\n    return {{_n}}\n");
    let (metrics, _) = compare_case(
        "header-current-duplicate-declaration",
        before,
        &current,
        None,
    );
    assert!(
        metrics.header_hits > 0,
        "unchanged first declaration must hit before duplicate validation: {metrics:?}"
    );
}

#[test]
fn real_core_function_default_continuations_are_not_header_cached() {
    let before = "function sample(n: number = 1) :: number:\n    return {_n}\non load:\n    send sample() to console\n# old\n";
    let current = before.replace("# old", "# new");
    let (metrics, _) = compare_case("header-default-additional-parse", before, &current, None);
    assert_eq!(
        metrics.header_hits, 0,
        "AdditionalParse-bearing header must execute with current resources: {metrics:?}"
    );
}

//! Opt-in single-call attribution with a pinned Core and unchanged fuel budget. This does not
//! split guest argument lifting, Rust input destruction and automatic post-return.

use super::*;
use std::time::Instant;

fn ordinary_body_simple(tree: &ParserRawTree, node: &skript_parser::RawNode) -> bool {
    if node.kind != ParserRawNodeKind::Simple || node.parent.is_none() { return false; }
    let mut parent = node.parent;
    while let Some(id) = parent {
        let ancestor = tree.get(id).unwrap();
        if ancestor.parent.is_none() && ancestor.text.trim().eq_ignore_ascii_case("options") { return false; }
        parent = ancestor.parent;
    }
    true
}

#[test]
#[ignore = "bounded public-fixture diagnostic; requires SKRIPT_TREE_MEASUREMENT_PLAN"]
fn measure_tree_macro_transport_and_noop_controls() {
    let plan_path = std::env::var("SKRIPT_TREE_MEASUREMENT_PLAN").unwrap();
    let plan: serde_json::Value = serde_json::from_slice(&std::fs::read(plan_path).unwrap()).unwrap();
    let core = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../artifacts/core-library.wasm"));
    assert_eq!(format!("{:x}", Sha256::digest(core)), plan["coreSha256"].as_str().unwrap());
    let config = HostConfig::default();
    assert_eq!(config.fuel_per_call, 20_000_000);
    let started = Instant::now();
    let mut rows = Vec::new();
    let cases = plan["cases"].as_array().unwrap();
    assert!(cases.len() <= 8);
    for block in 0..2 {
        let order: Vec<_> = if block == 0 { (0..cases.len()).collect() }
            else { (0..cases.len()).rev().collect() };
        for case_index in order {
            let case = &cases[case_index];
            let source = if let Some(text) = case["text"].as_str() { text.to_owned() }
                else { std::fs::read_to_string(case["path"].as_str().unwrap()).unwrap() };
            assert!(source.len() <= 150_000);
            assert_eq!(format!("{:x}", Sha256::digest(source.as_bytes())), case["sha256"].as_str().unwrap());
            let mapped = MappedSource::identity(source.as_str());
            let raw = skript_parser::parse_raw_tree(&mapped, skript_parser::RawTreeOptions::for_skript_version(2, 15));
            assert!(raw.diagnostics.is_empty());
            let first_comment = raw.nodes.iter().find(|n| n.kind == ParserRawNodeKind::Comment).unwrap();
            let last_comment = raw.nodes.iter().rfind(|n| n.kind == ParserRawNodeKind::Comment).unwrap();
            let body = raw.nodes.iter().find(|n| ordinary_body_simple(&raw, n) && !n.text.contains("{@")).unwrap();
            let marker = raw.nodes.iter().find(|n| ordinary_body_simple(&raw, n) && n.text.contains("{@"));
            let mut targets = vec![("first-comment", first_comment.id, false),
                ("first-comment-minimal-tree", first_comment.id, true),
                ("last-comment", last_comment.id, false), ("body", body.id, false)];
            if let Some(marker) = marker { targets.push(("marker", marker.id, false)); }
            for (mode, target, minimal) in targets {
                assert!(started.elapsed() < Duration::from_secs(120), "whole diagnostic time bound");
                let setup = Instant::now();
                let mut host = ParserHost::new(core, HostConfig {
                    runtime_profile: RuntimeProfile { skript_version: Some("2.15.4".into()), ..Default::default() },
                    ..Default::default()
                }).unwrap();
                let host_setup_ms = setup.elapsed().as_secs_f64() * 1000.0;
                let transaction = host.begin_parse("file:///tree-measurement", "file:///tree-measurement/public.sk", 1).unwrap();
                let state_invocation = transaction.begin_invocation(CORE_LIBRARY_COMPONENT_ID.to_owned()).unwrap();
                let conversion = Instant::now();
                let mut tree = parser_raw_tree_to_wit(&raw);
                let conversion_ms = conversion.elapsed().as_secs_f64() * 1000.0;
                if minimal {
                    // A valid one-comment tree: same immediate no-op behavior,
                    // but not a proposed production payload-pruning scheme.
                    tree.nodes.retain(|node| node.id == target.get());
                    tree.roots = vec![target.get()];
                    assert!(tree.nodes[0].parent.is_none() && tree.nodes[0].children.is_empty());
                }
                let node_index = tree.nodes.iter().position(|n| n.id == target.get()).unwrap();
                let node = &tree.nodes[node_index];
                let target_kind = format!("{:?}", node.kind);
                let target_has_marker = node.text.contains("{@");
                if mode == "body" || mode == "marker" {
                    assert!(matches!(node.kind, WitRawNodeKind::Simple));
                    assert_eq!(target_has_marker, mode == "marker");
                }
                let target_text_bytes = node.text.len();
                let text_bytes: usize = tree.nodes.iter().map(|n| n.text.len() + n.line.raw_text.len()
                    + n.line.indentation.text.len() + n.line.trailing_trivia.iter().map(|t| t.text.len()).sum::<usize>()).sum();
                let origins: usize = tree.nodes.iter().map(|n| n.span.origins.len() + n.line.span.origins.len()
                    + n.line.content_span.origins.len() + n.line.line_ending_span.origins.len()
                    + n.line.indentation.span.origins.len()
                    + n.line.trailing_trivia.iter().map(|t| t.span.origins.len()).sum::<usize>()).sum();
                let node_count = tree.nodes.len();
                let roots = tree.roots.len();
                let mut depth = 0;
                let mut parent = raw.get(target).unwrap().parent;
                while let Some(id) = parent { depth += 1; parent = raw.get(id).unwrap().parent; }
                let cancellation = DocumentCancellationToken::new();
                let snapshot = if minimal {
                    ParserRawTree { roots: vec![target], nodes: vec![raw.get(target).unwrap().clone()],
                        diagnostics: Vec::new(), indentation: raw.indentation.clone() }
                } else { raw.clone() };
                let lease = tree_view::Lease::new(Arc::new(snapshot), 0, Some(cancellation.clone())).unwrap();
                let input = TreeMacroInput {
                    context: InvocationContext { invocation_id:1, subscription_id:"core.options-preprocessor".into(),
                        document_id:"file:///tree-measurement/public.sk".into(), document_revision:1,
                        expansion:None, syntax_context:0 }, tree: tree_access::reference(lease.reference()), target:tree_access::header(raw.get(target).unwrap()), depth,
                };
                let entry = &mut host.components[0];
                entry.store.data_mut().invocation = Some(state_invocation);
                entry.store.data_mut().tree_view = Some(lease);
                entry.store.data_mut().cancellation = Some(cancellation.clone());
                let cancellation_polling = entry.store.data().interrupt_safe;
                prepare_store(&mut entry.store, config.fuel_per_call, 200, CORE_LIBRARY_COMPONENT_ID, "tree macro").unwrap();
                let before = entry.store.get_fuel().unwrap();
                let call_started = Instant::now();
                let call = entry.bindings.nlaocs_skript_parser_addon_tree_macro().call_expand(&mut entry.store, &input);
                let (tree_queries, tree_response_budget_bytes) = entry.store.data().tree_view.as_ref().unwrap().measurement();
                entry.store.data_mut().tree_view.take();
                let call_ms = call_started.elapsed().as_secs_f64() * 1000.0;
                let after = entry.store.get_fuel().unwrap();
                entry.store.data_mut().invocation.take().unwrap().rollback();
                let (status, edit, diagnostics) = match call {
                    Ok(Ok(output)) => ("ok".to_owned(), Some(output.edit.is_some()), Some(output.effects.diagnostics.len())),
                    Ok(Err(error)) => (format!("addon-error: {:?}", error.kind), None, None),
                    Err(error) => (classify_wasmtime_error(CORE_LIBRARY_COMPONENT_ID.into(), "tree macro", error).to_string(), None, None),
                };
                if minimal { assert_eq!(status, "ok"); assert_eq!(edit, Some(false)); }
                if status == "ok" && mode.contains("comment") { assert_eq!(edit, Some(false)); assert_eq!(diagnostics, Some(0)); }
                assert!(call_ms < 3000.0, "per-call wall guard was not effective: {call_ms}");
                if minimal && block == 0 && case_index == 0 {
                    cancellation.cancel();
                    assert!(matches!(prepare_store(&mut entry.store, config.fuel_per_call, 200,
                        CORE_LIBRARY_COMPONENT_ID, "tree macro"), Err(HostError::Cancelled(_))));
                }
                let row = serde_json::json!({"block":block,"case":case["name"],"sourceSha256":case["sha256"],
                    "sourceBytes":source.len(),"lines":source.lines().count(),"mode":mode,
                    "legacyDiagnosticWireNodes":node_count,"roots":roots,"partialStringBytes":text_bytes,"partialOriginCount":origins,
                    "targetIndex":node_index,"targetKind":target_kind,"targetHasMarker":target_has_marker,
                    "targetTextBytes":target_text_bytes,"hostSetupMs":host_setup_ms,"legacyDiagnosticWireConversionMs":conversion_ms,
                    "treeQueryCalls":tree_queries,"treeResponseBudgetBytes":tree_response_budget_bytes,"callMs":call_ms,"fuelBefore":before,"fuelAfter":after,"fuelConsumed":before-after,
                    "inFlightCancellationPolling":cancellation_polling,
                    "status":status,"edit":edit,"diagnostics":diagnostics});
                println!("tree_macro_measurement_row {row}");
                rows.push(row);
            }
        }
    }
    println!("tree_macro_measurement {}", serde_json::json!({"rows":rows,"fuelPerCall":config.fuel_per_call,
        "coreSha256":plan["coreSha256"],"wallBoundPerCallMs":2000,"wholeTestBudgetSeconds":120,
        "preCancelledPreparationRejected":true,"coreAndAbiUnchanged":false,"treeTransport":"invocation-scoped immutable handle",
        "limits":"Single direct tree-macro calls on fresh initialized hosts without a syntax catalog, not full document or editor timing. Wasmtime43 call includes automatic post-return. Legacy full-WIT conversion is diagnostic-only, measured outside the current handle call and before minimal-tree pruning. Current input is a fixed header/reference plus on-demand reads; response bytes are conservative quota accounting, not canonical transfer bytes; call fuel aggregates realloc/lower/lift, Core body, Rust input drop and output cleanup. No-op controls constrain attribution but do not split these subphases. Partial string/origin counts are not canonical ABI byte totals. The one-comment tree is a diagnostic ablation only. Pre-cancel is tested; in-flight polling follows the actual Core capability and is not forced on. A fixed two-second epoch trap deadline and outer owned-process watchdog bound the experiment."}));
}

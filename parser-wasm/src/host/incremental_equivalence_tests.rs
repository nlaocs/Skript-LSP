//! Actual Core/WASM cold-versus-reused document equivalence. The only canonical
//! exclusions are private per-parse receipt IDs and opaque parse-result tokens.
use super::*;

fn actual_host() -> ParserHost {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/type-parser-versions/skript-2.16.2-java25");
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
    // These are opaque invocation-resource handles, not semantic IDs. The
    // cache never reuses AdditionalParse results and native lookups rerun.
    for result in &mut effects.parse_results {
        result.host_token = 0;
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

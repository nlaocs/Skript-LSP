use parser_wasm::host::{
    HookDecision, HostConfig, HostError, InvocationContext, ParserHost, TreeMacroRequest,
};
use skript_parser::{
    MappedSource, RawNode, RawNodeId, RawTree, RawTreeOptions, SyntaxContextId, parse_raw_tree,
};

const CORE_LIBRARY: &[u8] = include_bytes!("../../artifacts/core-library.wasm");
const TREE_MACRO_ADDON: &[u8] = include_bytes!("../../artifacts/tree-macro-addon.wasm");
const COMPONENT_ID: &str = "nlaocs.test.tree-macro";

fn context(revision: u64) -> InvocationContext {
    InvocationContext {
        invocation_id: revision,
        subscription_id: String::new(),
        document_id: "file:///workspace/test.sk".to_owned(),
        document_revision: revision,
        expansion: None,
        syntax_context: 0,
    }
}

fn host(mut config: HostConfig) -> ParserHost {
    config
        .runtime_profile
        .skript_version
        .get_or_insert_with(|| "2.16.0".to_owned());
    let mut host = ParserHost::new(CORE_LIBRARY, config).expect("CoreLibrary must initialize");
    host.load_addon(TREE_MACRO_ADDON)
        .expect("tree macro addon must initialize");
    host
}

fn core_host(mut config: HostConfig) -> ParserHost {
    config
        .runtime_profile
        .skript_version
        .get_or_insert_with(|| "2.16.0".to_owned());
    ParserHost::new(CORE_LIBRARY, config).expect("CoreLibrary must initialize")
}

fn request(revision: u64, text: &str) -> TreeMacroRequest {
    let source = MappedSource::identity(text);
    let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 16));
    TreeMacroRequest {
        context: context(revision),
        source,
        tree,
    }
}

fn root_texts(tree: &RawTree) -> Vec<&str> {
    tree.roots
        .iter()
        .map(|id| tree.get(*id).expect("root must exist").text.as_str())
        .collect()
}

fn root(tree: &RawTree, index: usize) -> &RawNode {
    tree.get(tree.roots[index]).expect("root must exist")
}

fn addon_calls(
    result: &parser_wasm::host::TreeMacroResult,
) -> impl Iterator<Item = &parser_wasm::host::TreeMacroCall> {
    result
        .calls
        .iter()
        .filter(|call| call.component_id == COMPONENT_ID)
}

fn written_keys(transaction: &parser_wasm::state::ParseTransaction) -> Vec<String> {
    transaction
        .read_write_set()
        .expect("state access set must remain available")
        .writes
        .into_iter()
        .map(|entry| entry.key)
        .collect()
}

#[test]
fn core_options_macro_follows_skripts_header_and_body_timing() {
    let input = "options:\n    message: hello\non load:\n    send \"{@message} {@later}\"\ncommand /{@later}:\n    trigger:\n        send \"{@message}\"\noptions:\n    later: test\n";
    let mut host = core_host(HostConfig::default());
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 40)
        .expect("parse must begin");
    let result = host
        .expand_tree_in_parse(&transaction, request(40, input))
        .expect("CoreLibrary option expansion must finish");

    assert_eq!(
        root_texts(&result.tree),
        ["options", "on load", "command /test", "options"]
    );
    let on_load = root(&result.tree, 1);
    let body = result
        .tree
        .get(on_load.children[0])
        .expect("event body must remain attached");
    assert_eq!(body.text, "send \"hello test\"");
    let command = root(&result.tree, 2);
    let trigger = result
        .tree
        .get(command.children[0])
        .expect("command trigger entry must remain attached");
    let command_body = result
        .tree
        .get(trigger.children[0])
        .expect("command body must remain attached");
    assert_eq!(command_body.text, "send \"hello\"");
    assert!(result.effects.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "core.options.undefined" && diagnostic.message.contains("{@later}")
    }));
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn replaces_one_node_with_zero_one_or_many_and_reenters_generated_nodes() {
    let mut host = host(HostConfig::default());
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 1)
        .expect("parse must begin");
    let result = host
        .expand_tree_in_parse(&transaction, request(1, "delete\none\nmany"))
        .expect("tree macros must run");

    assert_eq!(
        root_texts(&result.tree),
        ["one-expanded", "many-first", "many-second"]
    );
    assert!(result.failures.is_empty());
    assert_eq!(addon_calls(&result).count(), 6);
    assert!(addon_calls(&result).all(|call| call.accepted));
    assert_eq!(
        addon_calls(&result)
            .filter(|call| call.expansion.is_some())
            .count(),
        3
    );
    for node in &result.tree.nodes {
        assert_ne!(node.syntax_context, SyntaxContextId::ROOT);
        let expansion = node
            .span
            .primary_origin()
            .and_then(|origin| origin.expansion)
            .expect("generated node must carry an expansion");
        let trace = result
            .source
            .expansion_backtrace(expansion)
            .expect("generated expansion must be registered");
        assert_eq!(trace[0].component.as_str(), COMPONENT_ID);
        assert_eq!(trace[0].hook.as_str(), "tree.expand");
    }
    let keys = written_keys(&transaction);
    assert!(keys.contains(&"tree.expand:delete".to_owned()));
    assert!(keys.contains(&"tree.expand:one-expanded".to_owned()));
    assert!(keys.contains(&"tree.expand:many-second".to_owned()));
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn preserves_or_replaces_section_bodies_before_visiting_children() {
    let mut host = host(HostConfig::default());
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 2)
        .expect("parse must begin");
    let result = host
        .expand_tree_in_parse(
            &transaction,
            request(
                2,
                "preserve:\n    retained-child\nreplace-body:\n    discarded-child",
            ),
        )
        .expect("section edits must run");

    let preserved = root(&result.tree, 0);
    assert_eq!(preserved.text, "preserved-section");
    assert_eq!(preserved.children.len(), 1);
    assert_eq!(
        result
            .tree
            .get(preserved.children[0])
            .expect("retained child must exist")
            .text,
        "retained-child"
    );

    let replaced = root(&result.tree, 1);
    assert_eq!(replaced.text, "replace-body");
    assert_eq!(replaced.children.len(), 1);
    assert_eq!(
        result
            .tree
            .get(replaced.children[0])
            .expect("replacement child must exist")
            .text,
        "replacement-child"
    );
    let keys = written_keys(&transaction);
    assert!(keys.contains(&"tree.expand:retained-child".to_owned()));
    assert!(keys.contains(&"tree.expand:replacement-child".to_owned()));
    assert!(!keys.contains(&"tree.expand:discarded-child".to_owned()));
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn transports_lossless_raw_tree_fields_through_wit() {
    let mut host = host(HostConfig::default());
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 20)
        .expect("parse must begin");
    let result = host
        .expand_tree_in_parse(
            &transaction,
            request(20, "holder:\n    child\ninspect-wire # trailing"),
        )
        .expect("the lossless tree must cross the component boundary");

    assert_eq!(root_texts(&result.tree), ["holder", "wire-ok"]);
    assert!(result.failures.is_empty());
    transaction.cancel().expect("test parse may be cancelled");
}
#[test]
fn recursively_expands_with_nested_provenance_and_stops_cycles() {
    let mut host = host(HostConfig::default());
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 3)
        .expect("parse must begin");
    let result = host
        .expand_tree_in_parse(&transaction, request(3, "step-0\ncycle-a"))
        .expect("bounded recursive expansion must finish");

    assert_eq!(root_texts(&result.tree), ["step-2", "cycle-a"]);
    assert_eq!(result.failures.len(), 1);
    assert!(matches!(
        result.failures[0].error,
        HostError::TreeMacroCycleDetected { .. }
    ));
    assert!(
        result
            .effects
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "tree-macro-cycle")
    );
    let cycle_call = result
        .calls
        .iter()
        .find(|call| !call.accepted)
        .expect("cycle must be recorded as a rejected call");
    assert_eq!(cycle_call.subscription_id, "tree.expand");

    let expansion = root(&result.tree, 0)
        .span
        .primary_origin()
        .and_then(|origin| origin.expansion)
        .expect("recursive node has provenance");
    let trace = result
        .source
        .expansion_backtrace(expansion)
        .expect("recursive expansion has a backtrace");
    assert_eq!(trace.len(), 2);
    assert!(
        trace
            .iter()
            .all(|entry| entry.component.as_str() == COMPONENT_ID)
    );
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn invalid_edits_and_addon_errors_preserve_nodes_and_rollback_candidate_state() {
    for (revision, text, expected) in [
        (4, "invalid", "invalid"),
        (5, "addon-error", "addon-error"),
        (6, "fragment-cycle", "fragment-cycle"),
    ] {
        let mut host = host(HostConfig::default());
        let transaction = host
            .begin_parse("file:///workspace", "file:///workspace/test.sk", revision)
            .expect("parse must begin");
        let result = host
            .expand_tree_in_parse(&transaction, request(revision, text))
            .expect("one failing candidate must preserve the tree");

        assert_eq!(root_texts(&result.tree), [expected]);
        assert_eq!(result.failures.len(), 1);
        assert!(addon_calls(&result).any(|call| !call.accepted));
        assert!(written_keys(&transaction).is_empty());
        if text == "addon-error" {
            assert!(matches!(
                result.failures[0].error,
                HostError::AddonFailure { .. }
            ));
            assert_eq!(result.effects.diagnostics.len(), 1);
            assert_eq!(
                result.effects.diagnostics[0].code,
                "fixture.tree-addon-error"
            );
        } else {
            assert!(matches!(
                result.failures[0].error,
                HostError::InvalidTreeMacroOutput { .. }
            ));
        }
        transaction.cancel().expect("test parse may be cancelled");
    }
}

#[test]
fn traps_preserve_the_original_tree_and_rollback_state() {
    let mut host = host(HostConfig::default());
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 6)
        .expect("parse must begin");
    let result = host
        .expand_tree_in_parse(&transaction, request(6, "trap"))
        .expect("a trapped addon is isolated");

    assert_eq!(root_texts(&result.tree), ["trap"]);
    assert_eq!(result.failures.len(), 1);
    assert!(matches!(result.failures[0].error, HostError::Trap { .. }));
    assert!(addon_calls(&result).any(|call| !call.accepted));
    assert!(written_keys(&transaction).is_empty());
    assert!(
        host.components()
            .iter()
            .find(|component| component.component_id == COMPONENT_ID)
            .expect("fixture remains registered")
            .disabled
    );
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn rejection_rolls_back_prior_tree_edits_provenance_and_state() {
    let mut host = host(HostConfig::default());
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 7)
        .expect("parse must begin");
    let result = host
        .expand_tree_in_parse(&transaction, request(7, "one\nreject"))
        .expect("typed rejection is a successful pipeline result");

    assert!(matches!(result.decision, HookDecision::Reject(_)));
    assert_eq!(root_texts(&result.tree), ["one", "reject"]);
    assert!(result.source.expansions().is_empty());
    assert!(
        result
            .calls
            .iter()
            .all(|call| !call.accepted && call.expansion.is_none())
    );
    assert!(written_keys(&transaction).is_empty());
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn malformed_input_tree_returns_an_error_and_rolls_back_prior_calls() {
    let mut host = host(HostConfig::default());
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/test.sk", 9)
        .expect("parse must begin");
    let mut malformed = request(9, "holder:");
    malformed.tree.nodes[0].children.push(RawNodeId::new(999));
    let error = host
        .expand_tree_in_parse(&transaction, malformed)
        .expect_err("unknown child IDs must not panic the host");

    assert!(matches!(error, HostError::InvalidTreeMacroOutput { .. }));
    assert!(written_keys(&transaction).is_empty());
    transaction.cancel().expect("test parse may be cancelled");
}

#[test]
fn expansion_node_and_call_quotas_rollback_the_complete_pipeline() {
    let cases = [
        (
            HostConfig {
                max_tree_macro_expansion_depth: 1,
                ..HostConfig::default()
            },
            "step-0",
            "expansion-depth",
        ),
        (
            HostConfig {
                max_tree_macro_nodes: 1,
                ..HostConfig::default()
            },
            "many",
            "nodes",
        ),
        (
            HostConfig {
                max_tree_macro_calls: 1,
                ..HostConfig::default()
            },
            "step-0",
            "calls",
        ),
    ];

    for (index, (config, text, quota)) in cases.into_iter().enumerate() {
        let revision = 10 + index as u64;
        let mut host = host(config);
        let transaction = host
            .begin_parse("file:///workspace", "file:///workspace/test.sk", revision)
            .expect("parse must begin");
        let error = host
            .expand_tree_in_parse(&transaction, request(revision, text))
            .expect_err("quota must abort the pipeline");

        match quota {
            "expansion-depth" => assert!(matches!(
                error,
                HostError::TreeMacroExpansionDepthQuotaExceeded { limit: 1 }
            )),
            "nodes" => assert!(matches!(
                error,
                HostError::TreeMacroNodeQuotaExceeded { limit: 1 }
            )),
            "calls" => assert!(matches!(
                error,
                HostError::TreeMacroCallQuotaExceeded { limit: 1 }
            )),
            _ => unreachable!(),
        }
        assert!(written_keys(&transaction).is_empty());
        transaction.cancel().expect("test parse may be cancelled");
    }
}

#[test]
fn raw_tree_depth_quota_rejects_input_and_generated_nesting() {
    for (revision, text) in [
        (20, "outer:\n    middle:\n        leaf"),
        (21, "deep-generated"),
    ] {
        let mut host = host(HostConfig {
            max_raw_tree_depth: 2,
            ..HostConfig::default()
        });
        let transaction = host
            .begin_parse("file:///workspace", "file:///workspace/test.sk", revision)
            .expect("parse must begin");
        let error = host
            .expand_tree_in_parse(&transaction, request(revision, text))
            .expect_err("deep input and generated trees must be rejected before recursion");

        assert!(matches!(
            error,
            HostError::RawTreeDepthQuotaExceeded { limit: 2 }
        ));
        assert!(written_keys(&transaction).is_empty());
        transaction.cancel().expect("test parse may be cancelled");
    }
}

#[test]
fn raw_tree_and_macro_expansion_depth_quotas_are_independent() {
    {
        let mut host = host(HostConfig {
            max_raw_tree_depth: 3,
            max_tree_macro_expansion_depth: 1,
            ..HostConfig::default()
        });
        let transaction = host
            .begin_parse("file:///workspace", "file:///workspace/test.sk", 30)
            .expect("parse must begin");
        let result = host
            .expand_tree_in_parse(
                &transaction,
                request(30, "outer:\n    middle:\n        leaf"),
            )
            .expect("structural nesting must not consume macro expansion depth");
        assert_eq!(result.tree.nodes.len(), 3);
        transaction.cancel().expect("test parse may be cancelled");
    }

    {
        let mut host = host(HostConfig {
            max_raw_tree_depth: 1,
            max_tree_macro_expansion_depth: 2,
            ..HostConfig::default()
        });
        let transaction = host
            .begin_parse("file:///workspace", "file:///workspace/test.sk", 31)
            .expect("parse must begin");
        let result = host
            .expand_tree_in_parse(&transaction, request(31, "step-0"))
            .expect("macro re-entry must not consume structural tree depth");
        assert_eq!(root_texts(&result.tree), ["step-2"]);
        transaction.cancel().expect("test parse may be cancelled");
    }
}

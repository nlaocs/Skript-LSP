use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use parser_wasm::bindings::nlaocs::skript_parser_addon::types::{
    AstContextOrigin, AstNode, AstTree, HookDecision, MappedSpan, OriginKind, SourceOrigin,
    SyntaxKind, TextRange as WitTextRange,
};
use parser_wasm::host::{
    AstMacroRequest, DocumentParseRequest, DocumentParserConfig, HostConfig, HostError,
    InvocationContext, ParserHost,
};
use skript_parser::{ExpansionKind, MappedSource, TextRange as ParserTextRange};

const CORE_LIBRARY: &[u8] = include_bytes!("../../artifacts/core-library.wasm");
const AST_MACRO_ADDON: &[u8] = include_bytes!("../../artifacts/ast-macro-addon.wasm");
const COMPONENT_ID: &str = "nlaocs.test.ast-macro";
const SUBSCRIPTION_ID: &str = "ast.expand";

fn context(revision: u64) -> InvocationContext {
    InvocationContext {
        invocation_id: revision,
        subscription_id: String::new(),
        document_id: "file:///workspace/ast.sk".to_owned(),
        document_revision: revision,
        expansion: None,
        syntax_context: 0,
    }
}

fn fixture_host(mut config: HostConfig) -> ParserHost {
    if config.syntax_catalog.is_none() {
        config
            .runtime_profile
            .skript_version
            .get_or_insert_with(|| "2.16.0".to_owned());
    }
    let mut host = ParserHost::new(CORE_LIBRARY, config).expect("CoreLibrary must initialize");
    host.load_addon(AST_MACRO_ADDON)
        .expect("AST macro fixture must initialize");
    host
}

fn span(start: usize, end: usize) -> MappedSpan {
    let start = u64::try_from(start).expect("test range fits u64");
    let end = u64::try_from(end).expect("test range fits u64");
    MappedSpan {
        virtual_range: WitTextRange { start, end },
        origins: vec![SourceOrigin {
            original_range: WitTextRange { start, end },
            kind: OriginKind::Exact,
            expansion: None,
        }],
    }
}

fn node(id: u64, text: &str, start: usize, end: usize) -> AstNode {
    AstNode {
        id,
        kind: SyntaxKind::Effect,
        syntax_id: "parser:test.effect".to_owned(),
        text: text.to_owned(),
        span: span(start, end),
        syntax_context: 0,
        context_origin: AstContextOrigin::Preserved,
        summary: None,
        captures: Vec::new(),
        children: Vec::new(),
        metadata: Vec::new(),
    }
}

fn fixture_tree(texts: &[&str]) -> (MappedSource, AstTree) {
    let mut source = String::new();
    let mut nodes = Vec::with_capacity(texts.len());
    for (id, text) in texts.iter().enumerate() {
        if !source.is_empty() {
            source.push('\n');
        }
        let start = source.len();
        source.push_str(text);
        let end = source.len();
        nodes.push(node(id as u64, text, start, end));
    }
    let roots = nodes.iter().map(|node| node.id).collect();
    (MappedSource::identity(source), AstTree { roots, nodes })
}

fn root_texts(tree: &AstTree) -> Vec<&str> {
    tree.roots
        .iter()
        .map(|id| {
            tree.nodes
                .iter()
                .find(|node| node.id == *id)
                .expect("root must exist")
                .text
                .as_str()
        })
        .collect()
}

fn request(revision: u64, source: MappedSource, tree: AstTree) -> AstMacroRequest {
    AstMacroRequest {
        context: context(revision),
        source,
        tree,
    }
}

fn written_keys(transaction: &parser_wasm::state::ParseTransaction) -> Vec<String> {
    transaction
        .read_write_set()
        .expect("StateStore access set must remain available")
        .writes
        .into_iter()
        .map(|entry| entry.key)
        .collect()
}

fn modern_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")
}

fn document_seed() -> (ParserHost, parser_wasm::host::DocumentParseResult) {
    let catalog = Arc::new(
        ssg::load(modern_fixture())
            .expect("the modern SSG fixture must load")
            .catalog()
            .clone(),
    );
    let mut host = fixture_host(HostConfig {
        syntax_catalog: Some(catalog),
        ..HostConfig::default()
    });
    let result = host
        .parse_document(
            DocumentParseRequest::new(
                "file:///workspace",
                "file:///workspace/ast.sk",
                1,
                "on load:\n    send 1\n",
            ),
            DocumentParserConfig::default(),
        )
        .expect("document parsing must produce an AST seed");
    (host, result)
}

fn synthetic_roots_from_document(
    result: &parser_wasm::host::DocumentParseResult,
    texts: &[&str],
) -> AstTree {
    let template_id = *result
        .ast
        .roots
        .first()
        .expect("document must have an AST root");
    let template = result
        .ast
        .nodes
        .iter()
        .find(|node| node.id == template_id)
        .expect("AST root must have a node")
        .clone();
    let nodes = texts
        .iter()
        .enumerate()
        .map(|(id, text)| {
            let mut node = template.clone();
            node.id = id as u64;
            node.text = (*text).to_owned();
            node.syntax_context = 0;
            node.context_origin = AstContextOrigin::Preserved;
            node.summary = None;
            node.captures.clear();
            node.children.clear();
            node.metadata.clear();
            node
        })
        .collect::<Vec<_>>();
    AstTree {
        roots: nodes.iter().map(|node| node.id).collect(),
        nodes,
    }
}

fn assert_ast_expansion(result: &parser_wasm::host::AstMacroResult, original: ParserTextRange) {
    let call = result
        .calls
        .iter()
        .find(|call| call.accepted && call.expansion.is_some())
        .expect("an accepted AST replacement must be recorded");
    let expansion_id = call.expansion.expect("call has an expansion");
    let expansion = result
        .source
        .expansions()
        .get(expansion_id)
        .expect("expansion graph must contain the call");
    assert_eq!(expansion.kind, ExpansionKind::Ast);
    assert_eq!(expansion.component.as_str(), COMPONENT_ID);
    assert_eq!(expansion.hook.as_str(), SUBSCRIPTION_ID);
    assert_eq!(expansion.call_sites[0].original_range, original);
    let backtrace = result
        .source
        .expansion_backtrace(expansion_id)
        .expect("AST expansion must have a backtrace");
    assert_eq!(backtrace.len(), 1);
    assert_eq!(backtrace[0].id, expansion_id);
}

#[test]
fn document_parse_feeds_zero_one_many_replacements_and_reenters_generated_nodes() {
    let (mut host, document) = document_seed();
    assert!(!document.ast.nodes.is_empty());
    assert!(
        document
            .ast_macro_calls
            .iter()
            .all(|call| call.component_id == COMPONENT_ID)
    );

    // The real document parse supplies the canonical AST. The fixture tree below
    // only changes target text so one document-stage result can exercise the
    // fixture's delete, one-root, and two-root branches deterministically.
    let tree = synthetic_roots_from_document(&document, &["delete", "one", "many"]);
    let expected_call_site = tree.nodes[0].span.origins[0].original_range;
    let source = document.source.clone();
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/ast.sk", 2)
        .expect("AST parse transaction must begin");
    let result = host
        .expand_ast_in_parse(&transaction, request(2, source, tree))
        .expect("AST macro expansion must finish");

    assert_eq!(
        root_texts(&result.tree),
        ["one-expanded", "many-first", "many-second"]
    );
    assert_eq!(
        result.calls.len(),
        6,
        "replacement roots must be re-entered"
    );
    assert_eq!(
        result
            .calls
            .iter()
            .filter(|call| call.accepted && call.expansion.is_some())
            .count(),
        3,
        "delete, one-root, and two-root replacements each create provenance"
    );
    assert!(result.failures.is_empty());
    assert!(!written_keys(&transaction).is_empty());
    assert_ast_expansion(
        &result,
        ParserTextRange::new(
            expected_call_site.start as usize,
            expected_call_site.end as usize,
        ),
    );
    transaction
        .cancel()
        .expect("test transaction may be cancelled");
}

#[test]
fn generated_ast_nodes_have_fresh_context_and_call_site_backtrace() {
    let mut host = fixture_host(HostConfig::default());
    let (source, tree) = fixture_tree(&["one", "one"]);
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/ast.sk", 3)
        .expect("AST parse transaction must begin");
    let result = host
        .expand_ast_in_parse(&transaction, request(3, source, tree))
        .expect("one-root AST replacement must finish");

    let generated = result
        .tree
        .nodes
        .iter()
        .filter(|node| node.syntax_id.starts_with("macro:nlaocs.test.ast-macro/"))
        .collect::<Vec<_>>();
    assert_eq!(generated.len(), 2);
    assert!(
        generated
            .iter()
            .all(|node| node.context_origin == AstContextOrigin::Macro)
    );
    assert!(
        generated.iter().all(|node| node.syntax_context != 0),
        "macro nodes need fresh contexts"
    );
    assert_ne!(
        generated[0].syntax_context, generated[1].syntax_context,
        "separate macro expansions must not share a hygiene context"
    );
    assert_ast_expansion(&result, ParserTextRange::new(0, 3));
    transaction
        .cancel()
        .expect("test transaction may be cancelled");
}

#[test]
fn metadata_and_definition_site_intent_survive_validated_replacements() {
    let mut host = fixture_host(HostConfig::default());
    let (source, tree) = fixture_tree(&["metadata", "definition-site"]);
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/ast.sk", 31)
        .expect("AST parse transaction must begin");
    let result = host
        .expand_ast_in_parse(&transaction, request(31, source, tree))
        .expect("metadata and definition-site replacements must finish");

    let metadata = result
        .tree
        .nodes
        .iter()
        .find(|node| node.text == "metadata")
        .expect("metadata node must remain");
    assert!(metadata.metadata.iter().any(|entry| {
        entry.key == "fixture.ast-macro.mode"
            && entry.value == "metadata-updated"
            && entry.owner_component_id.as_deref() == Some(COMPONENT_ID)
    }));

    let definition_site = result
        .tree
        .nodes
        .iter()
        .find(|node| node.text == "definition-site-expanded")
        .expect("definition-site node must be generated");
    assert_eq!(
        definition_site.context_origin,
        AstContextOrigin::DefinitionSite
    );
    assert_ne!(definition_site.syntax_context, 0);
    let expansion_id = result
        .calls
        .iter()
        .find(|call| call.target == 1 && call.accepted)
        .and_then(|call| call.expansion)
        .expect("definition-site replacement must record an expansion");
    assert!(
        result
            .source
            .expansions()
            .get(expansion_id)
            .expect("definition-site expansion must exist")
            .definition_site
            .is_some()
    );
    transaction
        .cancel()
        .expect("test transaction may be cancelled");
}

#[test]
fn preserved_and_call_site_contexts_survive_without_hygiene_replacement() {
    let mut host = fixture_host(HostConfig::default());
    let (source, mut tree) = fixture_tree(&["preserved"]);
    tree.nodes[0].context_origin = AstContextOrigin::CallSite;
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/ast.sk", 4)
        .expect("AST parse transaction must begin");
    let result = host
        .expand_ast_in_parse(&transaction, request(4, source, tree))
        .expect("preserved AST replacement must finish");

    let preserved = result
        .tree
        .nodes
        .first()
        .expect("preserved node must remain");
    assert_eq!(preserved.text, "preserved");
    assert_eq!(preserved.context_origin, AstContextOrigin::Preserved);
    assert_eq!(preserved.syntax_context, 0);
    assert_ast_expansion(&result, ParserTextRange::new(0, 9));
    transaction
        .cancel()
        .expect("test transaction may be cancelled");

    // A generated node may explicitly resolve identifiers in the context of
    // the node it replaces instead of receiving the next fresh macro context.
    let mut host = fixture_host(HostConfig::default());
    let (source, tree) = fixture_tree(&["one"]);
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/ast.sk", 5)
        .expect("AST parse transaction must begin");
    let first = host
        .expand_ast_in_parse(&transaction, request(5, source, tree))
        .expect("fresh AST target must finish");
    let mut tree = first.tree;
    let call_site_context = tree.nodes[0].syntax_context;
    assert_ne!(call_site_context, 0);
    tree.nodes[0].text = "call-site".to_owned();
    let result = host
        .expand_ast_in_parse(&transaction, request(5, first.source, tree))
        .expect("call-site AST target must finish");
    assert_eq!(
        result.tree.nodes[0].context_origin,
        AstContextOrigin::CallSite
    );
    assert_eq!(result.tree.nodes[0].syntax_context, call_site_context);
    transaction
        .cancel()
        .expect("test transaction may be cancelled");
}

#[test]
fn invalid_fragment_addon_error_and_trap_preserve_nodes_and_state() {
    let cases = [(6, "invalid"), (7, "addon-error"), (8, "trap")];

    for (revision, text) in cases {
        let mut host = fixture_host(HostConfig::default());
        let (source, tree) = fixture_tree(&[text]);
        let original = tree.clone();
        let transaction = host
            .begin_parse("file:///workspace", "file:///workspace/ast.sk", revision)
            .expect("AST parse transaction must begin");
        let result = host
            .expand_ast_in_parse(&transaction, request(revision, source, tree))
            .expect("component failures must remain recoverable");

        assert_eq!(format!("{:?}", result.tree), format!("{original:?}"));
        assert!(result.source.expansions().is_empty());
        assert_eq!(result.failures.len(), 1);
        assert!(result.calls.iter().all(|call| !call.accepted));
        assert!(written_keys(&transaction).is_empty());
        match text {
            "invalid" => assert!(matches!(
                result.failures[0].error,
                HostError::InvalidAstMacroOutput { .. }
            )),
            "addon-error" => {
                assert!(matches!(
                    result.failures[0].error,
                    HostError::AddonFailure { .. }
                ));
                assert_eq!(
                    result.effects.diagnostics[0].code,
                    "fixture.ast-addon-error"
                );
            }
            "trap" => {
                assert!(matches!(result.failures[0].error, HostError::Trap { .. }));
                assert!(
                    host.components()
                        .iter()
                        .find(|component| component.component_id == COMPONENT_ID)
                        .expect("trapped fixture remains registered")
                        .disabled
                );
            }
            _ => unreachable!(),
        }
        transaction
            .cancel()
            .expect("test transaction may be cancelled");
    }
}

#[test]
fn reject_rolls_back_the_ast_stage_state_and_provenance() {
    let mut host = fixture_host(HostConfig::default());
    let (source, tree) = fixture_tree(&["one", "reject"]);
    let original = tree.clone();
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/ast.sk", 9)
        .expect("AST parse transaction must begin");
    let result = host
        .expand_ast_in_parse(&transaction, request(9, source, tree))
        .expect("typed AST rejection is recoverable");

    assert!(matches!(&result.decision, HookDecision::Reject(_)));
    assert_eq!(format!("{:?}", result.tree), format!("{original:?}"));
    assert!(result.source.expansions().is_empty());
    assert!(
        result
            .calls
            .iter()
            .all(|call| !call.accepted && call.expansion.is_none())
    );
    assert!(written_keys(&transaction).is_empty());
    let HookDecision::Reject(rejection) = result.decision else {
        unreachable!();
    };
    assert_eq!(rejection.diagnostics[0].code, "fixture.ast-reject");
    transaction
        .cancel()
        .expect("test transaction may be cancelled");
}

#[test]
fn cycle_is_reported_after_the_accepted_expansion_without_rolling_it_back() {
    let mut host = fixture_host(HostConfig::default());
    let (source, tree) = fixture_tree(&["cycle"]);
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/ast.sk", 10)
        .expect("AST parse transaction must begin");
    let result = host
        .expand_ast_in_parse(&transaction, request(10, source, tree))
        .expect("cycle detection must remain recoverable");

    assert_eq!(root_texts(&result.tree), ["cycle"]);
    assert_eq!(result.failures.len(), 1);
    assert!(matches!(
        result.failures[0].error,
        HostError::AstMacroCycleDetected { .. }
    ));
    assert!(result.calls.iter().any(|call| call.accepted));
    assert!(result.calls.iter().any(|call| !call.accepted));
    assert_eq!(result.source.expansions().len(), 1);
    assert_eq!(
        result
            .source
            .expansions()
            .iter()
            .next()
            .expect("expansion")
            .kind,
        ExpansionKind::Ast
    );
    assert!(!written_keys(&transaction).is_empty());
    transaction
        .cancel()
        .expect("test transaction may be cancelled");
}

#[test]
fn oversized_fragment_rolls_back_only_the_rejected_invocation() {
    let mut host = fixture_host(HostConfig {
        max_ast_macro_nodes: 2,
        ..HostConfig::default()
    });
    let (source, tree) = fixture_tree(&["state-write", "many"]);
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/ast.sk", 20)
        .expect("AST parse transaction must begin");
    let result = host
        .expand_ast_in_parse(&transaction, request(20, source, tree))
        .expect("an oversized addon fragment is recoverable");

    assert_eq!(root_texts(&result.tree), ["state-write", "many"]);
    assert_eq!(result.failures.len(), 1);
    assert!(matches!(
        &result.failures[0].error,
        HostError::InvalidAstMacroOutput { message, .. }
            if message.contains("more than 2 nodes")
    ));
    assert!(result.calls[0].accepted);
    assert!(!result.calls[1].accepted);
    let written = written_keys(&transaction);
    assert!(written.iter().any(|key| key == "explicit"));
    assert!(written.iter().any(|key| key == "invocation/20/0"));
    assert!(!written.iter().any(|key| key == "invocation/20/1"));
    transaction
        .cancel()
        .expect("test transaction may be cancelled");
}

#[test]
fn call_quota_aborts_and_rolls_back_the_ast_stage() {
    let mut host = fixture_host(HostConfig {
        max_ast_macro_calls: 1,
        ..HostConfig::default()
    });
    let (source, tree) = fixture_tree(&["one"]);
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/ast.sk", 21)
        .expect("AST parse transaction must begin");
    let error = host
        .expand_ast_in_parse(&transaction, request(21, source, tree))
        .expect_err("the host-wide call quota must abort the AST stage");

    assert!(matches!(
        error,
        HostError::AstMacroCallQuotaExceeded { limit: 1 }
    ));
    assert!(written_keys(&transaction).is_empty());
    transaction
        .cancel()
        .expect("test transaction may be cancelled");
}

#[test]
fn structural_depth_quota_rejects_an_input_before_macro_execution() {
    let mut host = fixture_host(HostConfig {
        max_ast_depth: 2,
        ..HostConfig::default()
    });
    let source = MappedSource::identity("root\nchild\ngrandchild");
    let mut root = node(0, "root", 0, 4);
    root.children = vec![1];
    let mut child = node(1, "child", 5, 10);
    child.children = vec![2];
    let grandchild = node(2, "grandchild", 11, 21);
    let tree = AstTree {
        roots: vec![0],
        nodes: vec![root, child, grandchild],
    };
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/ast.sk", 30)
        .expect("AST parse transaction must begin");
    let error = host
        .expand_ast_in_parse(&transaction, request(30, source, tree))
        .expect_err("input deeper than the AST quota must be rejected");

    assert!(matches!(
        error,
        HostError::InvalidAstMacroOutput { ref message, .. } if message.contains("depth")
    ));
    assert!(written_keys(&transaction).is_empty());
    transaction
        .cancel()
        .expect("test transaction may be cancelled");
}

#[test]
fn expansion_depth_budget_allows_one_reentry_at_its_boundary() {
    let mut host = fixture_host(HostConfig {
        max_ast_macro_expansion_depth: 1,
        ..HostConfig::default()
    });
    let (source, tree) = fixture_tree(&["one"]);
    let transaction = host
        .begin_parse("file:///workspace", "file:///workspace/ast.sk", 31)
        .expect("AST parse transaction must begin");
    let result = host
        .expand_ast_in_parse(&transaction, request(31, source, tree))
        .expect("one generated re-entry is within the configured depth");

    assert_eq!(root_texts(&result.tree), ["one-expanded"]);
    assert_eq!(result.calls.len(), 2);
    assert!(result.failures.is_empty());
    transaction
        .cancel()
        .expect("test transaction may be cancelled");
}

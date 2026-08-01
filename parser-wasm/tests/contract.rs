use std::path::PathBuf;

mod host {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "parser-addon",
    });
}

mod guest {
    wit_bindgen::generate!({
        path: "wit",
        world: "parser-addon",
        generate_unused_types: true,
    });
}

#[test]
fn wit_package_resolves_with_the_expected_world_and_exports() {
    let wit = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("wit");
    let mut resolve = wit_parser::Resolve::default();
    let (package, _) = resolve.push_dir(&wit).expect("WIT package must resolve");
    let package = &resolve.packages[package];

    assert_eq!(package.name.namespace, "nlaocs");
    assert_eq!(package.name.name, "skript-parser-addon");
    assert_eq!(
        package
            .name
            .version
            .as_ref()
            .map(ToString::to_string)
            .as_deref(),
        Some("0.6.0")
    );

    let world = package
        .worlds
        .get("parser-addon")
        .map(|id| &resolve.worlds[*id])
        .expect("parser-addon world must exist");
    let imports = world
        .imports
        .values()
        .filter_map(|item| match item {
            wit_parser::WorldItem::Interface { id, .. } => resolve.interfaces[*id].name.as_deref(),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(imports, ["types", "state-store", "dynamic-syntax-registry"]);
    let exports = world
        .exports
        .values()
        .filter_map(|item| match item {
            wit_parser::WorldItem::Interface { id, .. } => resolve.interfaces[*id].name.as_deref(),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        exports,
        ["addon", "hooks", "text-macro", "tree-macro", "ast-macro"]
    );
}

#[test]
fn host_bindings_expose_typed_hook_contract() {
    use host::nlaocs::skript_parser_addon::types::{
        AbiVersion, CapabilityRequirement, ComponentManifest, HookMode, HookPhase,
        HookSubscription, HookTarget, SyntaxKind,
    };

    let subscription = HookSubscription {
        id: "observe-expressions".to_owned(),
        target: HookTarget::SyntaxDefinition(SyntaxKind::Expression),
        phase: HookPhase::Candidate,
        priority: -20,
        mode: HookMode::Observe,
        capability_id: "parser.hooks".to_owned(),
    };
    let manifest = ComponentManifest {
        component_id: "test.component".to_owned(),
        component_version: "1.2.3".to_owned(),
        abi: AbiVersion { major: 1, minor: 0 },
        capabilities: vec![CapabilityRequirement {
            id: "parser.hooks".to_owned(),
            minimum_version: 1,
            required: true,
        }],
        subscriptions: vec![subscription],
        state_namespaces: Vec::new(),
    };

    assert_eq!(manifest.component_id, "test.component");
    assert_eq!(manifest.component_version, "1.2.3");
    assert_eq!(manifest.subscriptions[0].id, "observe-expressions");
    assert_eq!(manifest.subscriptions[0].priority, -20);
    assert_eq!(manifest.abi.major, 1);
}

#[test]
fn bindings_expose_typed_dynamic_syntax_registration() {
    use host::nlaocs::skript_parser_addon::types::{
        DynamicMultiplicity, DynamicSyntaxDefinition, DynamicSyntaxId, DynamicSyntaxReference,
        MetadataEntry, SyntaxKind,
    };

    let definition = DynamicSyntaxDefinition {
        local_id: "fixture-effect".to_owned(),
        kind: SyntaxKind::Effect,
        patterns: vec!["fixture %string%".to_owned()],
        priority: -10,
        before: vec![DynamicSyntaxReference::RegistrationId(
            "effect:fixture".to_owned(),
        )],
        after: vec![DynamicSyntaxReference::Dynamic(DynamicSyntaxId {
            component_id: None,
            local_id: "other-effect".to_owned(),
        })],
        return_type: Some("java.lang.String".to_owned()),
        return_multiplicity: Some(DynamicMultiplicity::Single),
        handler: "fixture.handle".to_owned(),
        metadata: vec![MetadataEntry {
            key: "origin".to_owned(),
            value: "contract-test".to_owned(),
        }],
    };

    assert_eq!(definition.kind, SyntaxKind::Effect);
    assert_eq!(definition.patterns, ["fixture %string%"]);
    assert!(matches!(
        definition.return_multiplicity,
        Some(DynamicMultiplicity::Single)
    ));
}

#[test]
fn guest_bindings_expose_typed_macro_payloads() {
    use guest::nlaocs::skript_parser_addon::types::{
        AstMacroInput, AstTree, HookDecision, HookEffects, InvocationContext, TextEdit,
        TextMacroOutput, TextRange,
    };

    let context = InvocationContext {
        invocation_id: 7,
        subscription_id: "expand-test".to_owned(),
        document_id: "file:///test.sk".to_owned(),
        document_revision: 2,
        expansion: None,
        syntax_context: 0,
    };
    let input = AstMacroInput {
        context,
        tree: AstTree {
            roots: Vec::new(),
            nodes: Vec::new(),
        },
    };
    let output = TextMacroOutput {
        decision: HookDecision::ContinueProcessing,
        edits: vec![TextEdit {
            range: TextRange { start: 0, end: 0 },
            replacement: "generated".to_owned(),
            anchor: Some(0),
        }],
        effects: HookEffects {
            diagnostics: Vec::new(),
            context_updates: Vec::new(),
            parse_requests: Vec::new(),
        },
    };

    assert_eq!(input.context.invocation_id, 7);
    assert!(input.tree.nodes.is_empty());
    assert_eq!(output.edits[0].replacement, "generated");
    assert_eq!(output.edits[0].anchor, Some(0));
}

#[test]
fn guest_bindings_expose_typed_tree_edits() {
    use guest::nlaocs::skript_parser_addon::types::{
        GeneratedRawNode, GeneratedRawNodeKind, GeneratedRawTree, HookDecision, HookEffects,
        ReplaceNodeEdit, RetainedChildren, RetainedChildrenPlacement, TreeEdit, TreeMacroOutput,
    };

    let output = TreeMacroOutput {
        decision: HookDecision::ContinueProcessing,
        edit: Some(TreeEdit::ReplaceNode(ReplaceNodeEdit {
            replacement: GeneratedRawTree {
                roots: vec![10, 11],
                nodes: vec![
                    GeneratedRawNode {
                        id: 10,
                        kind: GeneratedRawNodeKind::Section,
                        text: "generated".to_owned(),
                        children: Vec::new(),
                    },
                    GeneratedRawNode {
                        id: 11,
                        kind: GeneratedRawNodeKind::Simple,
                        text: "after generated".to_owned(),
                        children: Vec::new(),
                    },
                ],
            },
            retained_children: Some(RetainedChildren {
                target: 10,
                placement: RetainedChildrenPlacement::Append,
            }),
        })),
        effects: HookEffects {
            diagnostics: Vec::new(),
            context_updates: Vec::new(),
            parse_requests: Vec::new(),
        },
    };

    let Some(TreeEdit::ReplaceNode(edit)) = output.edit else {
        panic!("typed replacement must survive binding generation");
    };
    assert_eq!(edit.replacement.roots, [10, 11]);
    assert_eq!(
        edit.retained_children
            .expect("children are retained")
            .target,
        10
    );
}
#[test]
fn contract_defines_every_hook_phase_and_execution_mode() {
    use host::nlaocs::skript_parser_addon::types::{HookMode, HookPhase};

    let phases = [
        HookPhase::Document,
        HookPhase::Preprocess,
        HookPhase::Line,
        HookPhase::Tree,
        HookPhase::Node,
        HookPhase::Matching,
        HookPhase::Expression,
        HookPhase::Effect,
        HookPhase::Capture,
        HookPhase::Syntax,
        HookPhase::Candidate,
        HookPhase::Scope,
        HookPhase::Ast,
        HookPhase::Diagnostic,
    ];
    let modes = [HookMode::Observe, HookMode::Transform, HookMode::Override];

    assert_eq!(phases.len(), 14);
    assert_eq!(modes.len(), 3);
}

#[test]
fn bindings_expose_typed_matching_payload() {
    use host::nlaocs::skript_parser_addon::types::{
        MappedSpan, MatchingPathSegment, MatchingPayload, MatchingScope, MatchingStatus,
        MatchingTiming, OriginKind, SourceOrigin, TextRange,
    };

    let payload = MatchingPayload {
        input: "send hello".to_owned(),
        pattern: Some("send %string%".to_owned()),
        definition_id: "effect:send".to_owned(),
        registration_id: "effect:send#0".to_owned(),
        pattern_index: Some(0),
        element_path: vec![
            MatchingPathSegment::Element(1),
            MatchingPathSegment::Branch(0),
        ],
        pattern_span: Some(TextRange { start: 5, end: 13 }),
        scope: MatchingScope::Element,
        timing: MatchingTiming::Before,
        input_range: TextRange { start: 5, end: 10 },
        span: MappedSpan {
            virtual_range: TextRange { start: 5, end: 10 },
            origins: vec![SourceOrigin {
                original_range: TextRange { start: 20, end: 25 },
                kind: OriginKind::Exact,
                expansion: None,
            }],
        },
        status: MatchingStatus::Pending,
        failure_reason: None,
    };

    assert_eq!(payload.definition_id, "effect:send");
    assert_eq!(payload.registration_id, "effect:send#0");
    assert_eq!(payload.pattern_index, Some(0));
    assert_eq!(payload.element_path.len(), 2);
    assert_eq!(payload.span.origins[0].original_range.start, 20);
}

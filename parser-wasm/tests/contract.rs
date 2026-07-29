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
        Some("0.2.0")
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
fn contract_defines_every_hook_phase_and_execution_mode() {
    use host::nlaocs::skript_parser_addon::types::{HookMode, HookPhase};

    let phases = [
        HookPhase::Document,
        HookPhase::Preprocess,
        HookPhase::Line,
        HookPhase::Tree,
        HookPhase::Node,
        HookPhase::Matching,
        HookPhase::Capture,
        HookPhase::Syntax,
        HookPhase::Candidate,
        HookPhase::Scope,
        HookPhase::Ast,
        HookPhase::Diagnostic,
    ];
    let modes = [HookMode::Observe, HookMode::Transform, HookMode::Override];

    assert_eq!(phases.len(), 12);
    assert_eq!(modes.len(), 3);
}

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

//! Test component for AST macro component-boundary behavior.
//!
//! The fixture is intentionally deterministic: the target AST node's text
//! chooses the returned replacement, decision, error, trap, or state write.
#![allow(missing_docs)]

wit_bindgen::generate!({
    path: "../../parser-wasm/wit",
    world: "parser-addon",
    generate_unused_types: true,
});

use exports::nlaocs::skript_parser_addon::{addon, ast_macro, hooks, text_macro, tree_macro};
use nlaocs::skript_parser_addon::{
    state_store,
    types::{
        AbiVersion, AddonError, AddonErrorKind, AstContextOrigin, AstMacroInput, AstMacroOutput,
        AstNode, AstTree, CapabilityRequirement, CompatibilityError, CompatibilityErrorKind,
        ComponentManifest, Diagnostic, DiagnosticSeverity, HookDecision, HookEffects,
        HookInvocation, HookMode, HookOutput, HookPhase, HookSelector, HookSubscription,
        HookTarget, HostProfile, MetadataEntry, Rejection, StateEncoding,
        StateNamespaceDeclaration, StateNamespaceVisibility, StateScope, StateValue,
        TextMacroInput, TextMacroOutput, TreeMacroInput, TreeMacroOutput,
    },
};
use parser_wasm::{
    ABI_VERSION, AbiVersion as ParserAbiVersion, CAPABILITY_AST_MACRO, CAPABILITY_STATE_STORE,
    Capability as ParserCapability, CapabilityRequirement as ParserCapabilityRequirement,
    CompatibilityError as ParserCompatibilityError, validate_compatibility,
};

const COMPONENT_ID: &str = "nlaocs.test.ast-macro";
const SUBSCRIPTION: &str = "ast.expand";
const STATE_NAMESPACE: &str = "ast-state";
const STATE_SCHEMA: &str = "nlaocs.test.ast-macro-state";

struct AstMacroAddon;

fn empty_selector() -> HookSelector {
    HookSelector {
        pattern_index: None,
        pattern_source: None,
        mark: None,
        tags: Vec::new(),
        captures: Vec::new(),
        return_type: None,
        multiplicity: None,
        metadata: Vec::new(),
    }
}

impl addon::Guest for AstMacroAddon {
    fn manifest() -> ComponentManifest {
        ComponentManifest {
            component_id: COMPONENT_ID.to_owned(),
            component_version: env!("CARGO_PKG_VERSION").to_owned(),
            abi: AbiVersion {
                major: ABI_VERSION.major,
                minor: ABI_VERSION.minor,
            },
            capabilities: vec![
                CapabilityRequirement {
                    id: CAPABILITY_AST_MACRO.to_owned(),
                    minimum_version: 1,
                    required: true,
                },
                CapabilityRequirement {
                    id: CAPABILITY_STATE_STORE.to_owned(),
                    minimum_version: 1,
                    required: true,
                },
            ],
            subscriptions: vec![HookSubscription {
                id: SUBSCRIPTION.to_owned(),
                target: HookTarget::ParseStage,
                phase: HookPhase::Ast,
                priority: 0,
                mode: HookMode::Transform,
                capability_id: CAPABILITY_AST_MACRO.to_owned(),
                selector: empty_selector(),
            }],
            registered_syntax_handlers: Vec::new(),
            catalog_annotations: Vec::new(),
            state_namespaces: vec![StateNamespaceDeclaration {
                name: STATE_NAMESPACE.to_owned(),
                visibility: StateNamespaceVisibility::Private,
                schema_id: STATE_SCHEMA.to_owned(),
                schema_version: 1,
                readers: Vec::new(),
                writers: Vec::new(),
            }],
        }
    }

    fn initialize(profile: HostProfile) -> Result<(), CompatibilityError> {
        let requirements = [
            ParserCapabilityRequirement::required(CAPABILITY_AST_MACRO, 1),
            ParserCapabilityRequirement::required(CAPABILITY_STATE_STORE, 1),
        ];
        let capabilities = profile
            .capabilities
            .into_iter()
            .map(|capability| ParserCapability::new(capability.id, capability.version))
            .collect::<Vec<_>>();
        validate_compatibility(
            ABI_VERSION,
            ParserAbiVersion::new(profile.abi.major, profile.abi.minor),
            &requirements,
            &capabilities,
        )
        .map_err(map_compatibility_error)
    }
}

impl ast_macro::Guest for AstMacroAddon {
    fn expand(input: AstMacroInput) -> Result<AstMacroOutput, AddonError> {
        let target = input
            .tree
            .nodes
            .iter()
            .find(|node| node.id == input.target)
            .ok_or_else(|| addon_error(AddonErrorKind::InvalidPayload, "target node is absent"))?;

        record_invocation(&input, &target.text)?;

        match target.text.as_str() {
            "delete" => Ok(changed(empty_tree())),
            "one" => Ok(changed(replacement_tree(vec![generated_node(
                target,
                0,
                "one-expanded",
                "one",
            )]))),
            "many" => Ok(changed(replacement_tree(vec![
                generated_node(target, 0, "many-first", "many/first"),
                generated_node(target, 1, "many-second", "many/second"),
            ]))),
            "metadata" if has_fixture_metadata(target) => Ok(unchanged()),
            "metadata" => Ok(changed(preserved_fragment(
                &input.tree,
                target.id,
                Some(MetadataEntry {
                    key: "fixture.ast-macro.mode".to_owned(),
                    value: "metadata-updated".to_owned(),
                    owner_component_id: Some(COMPONENT_ID.to_owned()),
                }),
            ))),
            "preserved" if has_fixture_metadata(target) => Ok(unchanged()),
            "preserved" => Ok(changed(preserved_fragment(
                &input.tree,
                target.id,
                Some(MetadataEntry {
                    key: "fixture.ast-macro.mode".to_owned(),
                    value: "preserved".to_owned(),
                    owner_component_id: Some(COMPONENT_ID.to_owned()),
                }),
            ))),
            "call-site" => {
                let mut replacement = generated_node(target, 0, "call-site-expanded", "call-site");
                replacement.context_origin = AstContextOrigin::CallSite;
                Ok(changed(replacement_tree(vec![replacement])))
            }
            "definition-site" => {
                let mut replacement =
                    generated_node(target, 0, "definition-site-expanded", "definition-site");
                replacement.context_origin = AstContextOrigin::DefinitionSite;
                Ok(changed(replacement_tree(vec![replacement])))
            }
            "nested" => Ok(changed(replacement_tree(vec![generated_node(
                target,
                0,
                "nested-step",
                "nested/step",
            )]))),
            "nested-step" => Ok(changed(replacement_tree(vec![generated_node(
                target,
                0,
                "nested-complete",
                "nested/complete",
            )]))),
            "cycle" => Ok(changed(cycle_fragment(&input.tree, target.id))),
            "invalid" => Ok(changed(AstTree {
                roots: vec![u64::MAX],
                nodes: Vec::new(),
            })),
            "reject" => Ok(rejected(target)),
            "addon-error" => Err(AddonError {
                kind: AddonErrorKind::InvalidPayload,
                message: "fixture addon error".to_owned(),
                diagnostics: vec![diagnostic(
                    target,
                    "fixture.ast-addon-error",
                    "the AST macro fixture returned an addon error",
                )],
            }),
            "trap" => panic!("AST macro fixture trap"),
            "state-write" => {
                write_explicit_state(target)?;
                Ok(unchanged())
            }
            _ => Ok(unchanged()),
        }
    }
}

fn record_invocation(input: &AstMacroInput, text: &str) -> Result<(), AddonError> {
    state_store::put(
        StateScope::Parse,
        StateNamespaceVisibility::Private,
        STATE_NAMESPACE,
        &format!(
            "invocation/{}/{}",
            input.context.invocation_id, input.target
        ),
        &StateValue {
            schema_id: STATE_SCHEMA.to_owned(),
            encoding: StateEncoding::Json,
            bytes: format!("{{\"text\":{text:?},\"depth\":{}}}", input.depth).into_bytes(),
        },
    )
    .map_err(|error| {
        addon_error(
            AddonErrorKind::Internal,
            format!("failed to record AST macro invocation: {}", error.message),
        )
    })
}

fn write_explicit_state(target: &AstNode) -> Result<(), AddonError> {
    state_store::put(
        StateScope::Parse,
        StateNamespaceVisibility::Private,
        STATE_NAMESPACE,
        "explicit",
        &StateValue {
            schema_id: STATE_SCHEMA.to_owned(),
            encoding: StateEncoding::Raw,
            bytes: format!("target={} text={}", target.id, target.text).into_bytes(),
        },
    )
    .map_err(|error| {
        addon_error(
            AddonErrorKind::Internal,
            format!(
                "failed to write explicit AST macro state: {}",
                error.message
            ),
        )
    })
}

fn generated_node(target: &AstNode, id: u64, text: &str, suffix: &str) -> AstNode {
    let mut node = target.clone();
    node.id = id;
    node.syntax_id = format!("macro:{COMPONENT_ID}/{suffix}");
    node.text = text.to_owned();
    node.syntax_context = 0;
    node.context_origin = AstContextOrigin::Macro;
    node.summary = None;
    node.captures = Vec::new();
    node.children = Vec::new();
    node.metadata = Vec::new();
    node
}

fn preserved_fragment(tree: &AstTree, target_id: u64, extra: Option<MetadataEntry>) -> AstTree {
    let mut nodes = Vec::new();
    copy_preserved_subtree(tree, target_id, target_id, extra.as_ref(), &mut nodes);
    AstTree {
        roots: vec![target_id],
        nodes,
    }
}

fn cycle_fragment(tree: &AstTree, target_id: u64) -> AstTree {
    preserved_fragment(tree, target_id, None)
}

fn has_fixture_metadata(target: &AstNode) -> bool {
    target
        .metadata
        .iter()
        .any(|entry| entry.key == "fixture.ast-macro.mode")
}

fn copy_preserved_subtree(
    tree: &AstTree,
    id: u64,
    target_id: u64,
    extra: Option<&MetadataEntry>,
    output: &mut Vec<AstNode>,
) {
    let Some(original) = tree.nodes.iter().find(|node| node.id == id) else {
        return;
    };
    let children = original.children.clone();
    let mut node = original.clone();
    node.context_origin = AstContextOrigin::Preserved;
    if id == target_id {
        if let Some(extra) = extra {
            node.metadata.push(extra.clone());
        }
    }
    output.push(node);
    for child in children {
        copy_preserved_subtree(tree, child, target_id, extra, output);
    }
}

fn replacement_tree(nodes: Vec<AstNode>) -> AstTree {
    let roots = nodes.iter().map(|node| node.id).collect();
    AstTree { roots, nodes }
}

fn empty_tree() -> AstTree {
    AstTree {
        roots: Vec::new(),
        nodes: Vec::new(),
    }
}

fn changed(replacement: AstTree) -> AstMacroOutput {
    AstMacroOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(replacement),
        effects: empty_effects(),
    }
}

fn unchanged() -> AstMacroOutput {
    AstMacroOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: None,
        effects: empty_effects(),
    }
}

fn rejected(target: &AstNode) -> AstMacroOutput {
    AstMacroOutput {
        decision: HookDecision::Reject(Rejection {
            reason: "fixture requested AST rollback".to_owned(),
            diagnostics: vec![diagnostic(
                target,
                "fixture.ast-reject",
                "the AST macro fixture rejected this expansion",
            )],
        }),
        replacement: None,
        effects: empty_effects(),
    }
}

fn diagnostic(target: &AstNode, code: &str, message: &str) -> Diagnostic {
    Diagnostic {
        code: code.to_owned(),
        message: message.to_owned(),
        severity: DiagnosticSeverity::Error,
        span: target.span.clone(),
        related: Vec::new(),
    }
}

fn empty_effects() -> HookEffects {
    HookEffects {
        diagnostics: Vec::new(),
        context_updates: Vec::new(),
        parse_requests: Vec::new(),
        parse_results: Vec::new(),
    }
}

impl hooks::Guest for AstMacroAddon {
    fn invoke(_input: HookInvocation) -> Result<HookOutput, AddonError> {
        Err(unsupported("generic hook"))
    }
}

impl text_macro::Guest for AstMacroAddon {
    fn expand(_input: TextMacroInput) -> Result<TextMacroOutput, AddonError> {
        Err(unsupported("Text macro"))
    }
}

impl tree_macro::Guest for AstMacroAddon {
    fn expand(_input: TreeMacroInput) -> Result<TreeMacroOutput, AddonError> {
        Err(unsupported("Tree macro"))
    }
}

fn unsupported(kind: &str) -> AddonError {
    addon_error(
        AddonErrorKind::UnsupportedCapability,
        format!("AST macro fixture does not register a {kind}"),
    )
}

fn addon_error(kind: AddonErrorKind, message: impl Into<String>) -> AddonError {
    AddonError {
        kind,
        message: message.into(),
        diagnostics: Vec::new(),
    }
}

fn map_compatibility_error(error: ParserCompatibilityError) -> CompatibilityError {
    let (kind, subject) = match &error {
        ParserCompatibilityError::AbiVersionMismatch { .. } => {
            (CompatibilityErrorKind::AbiVersionMismatch, "abi".to_owned())
        }
        ParserCompatibilityError::MissingRequiredCapability { id, .. } => {
            (CompatibilityErrorKind::MissingCapability, id.clone())
        }
        ParserCompatibilityError::CapabilityVersionTooOld { id, .. } => {
            (CompatibilityErrorKind::CapabilityVersionTooOld, id.clone())
        }
        ParserCompatibilityError::BlankCapabilityId
        | ParserCompatibilityError::DuplicateCapability { .. } => (
            CompatibilityErrorKind::InvalidManifest,
            "capabilities".to_owned(),
        ),
    };
    CompatibilityError {
        kind,
        subject,
        message: error.to_string(),
    }
}

#[cfg(target_arch = "wasm32")]
export!(AstMacroAddon);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_declares_ast_phase_and_private_state() {
        let manifest = <AstMacroAddon as addon::Guest>::manifest();
        assert_eq!(manifest.component_id, COMPONENT_ID);
        assert_eq!(manifest.subscriptions.len(), 1);
        assert!(matches!(manifest.subscriptions[0].phase, HookPhase::Ast));
        assert!(matches!(
            manifest.subscriptions[0].mode,
            HookMode::Transform
        ));
        assert_eq!(manifest.state_namespaces.len(), 1);
    }
}

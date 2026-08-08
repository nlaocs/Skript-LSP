#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

//! Test Component covering recursive Tree macro edits and provenance.
//!
//! Scenarios exercise node/body replacement, retained children, recursion, cycles,
//! quotas, diagnostics, StateStore rollback, and guest traps.
#![allow(missing_docs)] // `wit_bindgen` generates the exported guest API.

wit_bindgen::generate!({
    path: "../../parser-wasm/wit",
    world: "parser-addon",
    generate_unused_types: true,
});

use exports::nlaocs::skript_parser_addon::{addon, ast_macro, hooks, text_macro, tree_macro};
use nlaocs::skript_parser_addon::{
    state_store,
    types::{
        AbiVersion, AddonError, AddonErrorKind, AstMacroInput, AstMacroOutput,
        CapabilityRequirement, CompatibilityError, CompatibilityErrorKind, ComponentManifest,
        Diagnostic, DiagnosticSeverity, GeneratedRawNode, GeneratedRawNodeKind, GeneratedRawTree,
        HookDecision, HookEffects, HookInvocation, HookMode, HookOutput, HookPhase,
        HookSubscription, HookTarget, HostProfile, IndentKind, RawTriviaKind, Rejection,
        ReplaceNodeEdit, RetainedChildren, RetainedChildrenPlacement, StateEncoding,
        StateNamespaceDeclaration, StateNamespaceVisibility, StateScope, StateValue,
        TextMacroInput, TextMacroOutput, TreeEdit, TreeMacroInput, TreeMacroOutput,
    },
};
use parser_wasm::{
    ABI_VERSION, AbiVersion as ParserAbiVersion, CAPABILITY_STATE_STORE, CAPABILITY_TREE_MACRO,
    Capability as ParserCapability, CapabilityRequirement as ParserCapabilityRequirement,
    CompatibilityError as ParserCompatibilityError, validate_compatibility,
};

const COMPONENT_ID: &str = "nlaocs.test.tree-macro";
const SUBSCRIPTION: &str = "tree.expand";
const STATE_NAMESPACE: &str = "tree-state";
const STATE_SCHEMA: &str = "nlaocs.test.tree-macro-state";

struct TreeMacroAddon;

impl addon::Guest for TreeMacroAddon {
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
                    id: CAPABILITY_TREE_MACRO.to_owned(),
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
                phase: HookPhase::Tree,
                priority: 0,
                mode: HookMode::Transform,
                capability_id: CAPABILITY_TREE_MACRO.to_owned(),
            }],
            registered_expression_class_suffixes: Vec::new(),
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
            ParserCapabilityRequirement::required(CAPABILITY_TREE_MACRO, 1),
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

impl tree_macro::Guest for TreeMacroAddon {
    fn expand(input: TreeMacroInput) -> Result<TreeMacroOutput, AddonError> {
        let target = input
            .tree
            .nodes
            .iter()
            .find(|node| node.id == input.target)
            .ok_or_else(|| addon_error(AddonErrorKind::InvalidPayload, "target node is absent"))?;
        record_invocation(&input, &target.text)?;

        match target.text.as_str() {
            "inspect-wire" => {
                let indentation_ok = input.tree.indentation.as_ref().is_some_and(|indentation| {
                    matches!(indentation.kind, IndentKind::Space) && indentation.unit == "    "
                });
                let line_ok = target.line.number == 2
                    && target.line.raw_text == "inspect-wire # trailing"
                    && target
                        .line
                        .trailing_trivia
                        .iter()
                        .any(|trivia| matches!(trivia.kind, RawTriviaKind::LineComment));
                let span_ok = !target.span.origins.is_empty()
                    && target.code_span.is_some()
                    && target.syntax_context == 0;
                if indentation_ok && line_ok && span_ok && input.tree.diagnostics.is_empty() {
                    Ok(changed(replace_single("wire-ok")))
                } else {
                    Err(addon_error(
                        AddonErrorKind::InvalidPayload,
                        "lossless RawTree fields were not transported correctly",
                    ))
                }
            }
            "trap" => panic!("tree macro fixture trap"),
            "addon-error" => Err(AddonError {
                kind: AddonErrorKind::InvalidPayload,
                message: "fixture addon error".to_owned(),
                diagnostics: vec![Diagnostic {
                    code: "fixture.tree-addon-error".to_owned(),
                    message: "the fixture rejected this node".to_owned(),
                    severity: DiagnosticSeverity::Error,
                    span: target.span.clone(),
                    related: Vec::new(),
                }],
            }),
            "reject" => Ok(TreeMacroOutput {
                decision: HookDecision::Reject(Rejection {
                    reason: "fixture requested tree rollback".to_owned(),
                    diagnostics: vec![Diagnostic {
                        code: "fixture.tree-reject".to_owned(),
                        message: "the complete tree expansion is rejected".to_owned(),
                        severity: DiagnosticSeverity::Error,
                        span: target.span.clone(),
                        related: Vec::new(),
                    }],
                }),
                edit: Some(replace_nodes(Vec::new())),
                effects: empty_effects(),
            }),
            "delete" => Ok(changed(replace_nodes(Vec::new()))),
            "one" => Ok(changed(replace_nodes(vec![node(
                0,
                GeneratedRawNodeKind::Simple,
                "one-expanded",
                Vec::new(),
            )]))),
            "many" => Ok(changed(replace_nodes(vec![
                node(0, GeneratedRawNodeKind::Simple, "many-first", Vec::new()),
                node(1, GeneratedRawNodeKind::Simple, "many-second", Vec::new()),
            ]))),
            "preserve" => Ok(changed(TreeEdit::ReplaceNode(ReplaceNodeEdit {
                replacement: tree(
                    vec![0],
                    vec![node(
                        0,
                        GeneratedRawNodeKind::Section,
                        "preserved-section",
                        Vec::new(),
                    )],
                ),
                retained_children: Some(RetainedChildren {
                    target: 0,
                    placement: RetainedChildrenPlacement::Prepend,
                }),
            }))),
            "replace-body" => Ok(changed(TreeEdit::ReplaceChildren(tree(
                vec![0],
                vec![node(
                    0,
                    GeneratedRawNodeKind::Simple,
                    "replacement-child",
                    Vec::new(),
                )],
            )))),
            "deep-generated" => Ok(changed(replace_nodes(vec![
                node(0, GeneratedRawNodeKind::Section, "generated-outer", vec![1]),
                node(1, GeneratedRawNodeKind::Section, "generated-inner", vec![2]),
                node(
                    2,
                    GeneratedRawNodeKind::Simple,
                    "generated-leaf",
                    Vec::new(),
                ),
            ]))),
            "step-0" => Ok(changed(replace_single("step-1"))),
            "step-1" => Ok(changed(replace_single("step-2"))),
            "cycle-a" => Ok(changed(replace_single("cycle-b"))),
            "cycle-b" => Ok(changed(replace_single("cycle-a"))),
            "invalid" => Ok(changed(replace_nodes(vec![node(
                0,
                GeneratedRawNodeKind::Simple,
                " ",
                Vec::new(),
            )]))),
            "fragment-cycle" => Ok(changed(TreeEdit::ReplaceNode(ReplaceNodeEdit {
                replacement: tree(
                    vec![0],
                    vec![
                        node(0, GeneratedRawNodeKind::Section, "cycle-root", vec![1]),
                        node(1, GeneratedRawNodeKind::Section, "cycle-child", vec![0]),
                    ],
                ),
                retained_children: None,
            }))),
            _ => Ok(unchanged()),
        }
    }
}

fn record_invocation(input: &TreeMacroInput, text: &str) -> Result<(), AddonError> {
    state_store::put(
        StateScope::Parse,
        StateNamespaceVisibility::Private,
        STATE_NAMESPACE,
        &format!("{}:{text}", input.context.subscription_id),
        &StateValue {
            schema_id: STATE_SCHEMA.to_owned(),
            encoding: StateEncoding::Json,
            bytes: input.depth.to_string().into_bytes(),
        },
    )
    .map_err(|error| {
        addon_error(
            AddonErrorKind::Internal,
            format!("failed to record tree macro state: {}", error.message),
        )
    })
}

fn node(id: u64, kind: GeneratedRawNodeKind, text: &str, children: Vec<u64>) -> GeneratedRawNode {
    GeneratedRawNode {
        id,
        kind,
        text: text.to_owned(),
        children,
    }
}

fn tree(roots: Vec<u64>, nodes: Vec<GeneratedRawNode>) -> GeneratedRawTree {
    GeneratedRawTree { roots, nodes }
}

fn replace_nodes(nodes: Vec<GeneratedRawNode>) -> TreeEdit {
    let roots = nodes.iter().map(|node| node.id).collect();
    TreeEdit::ReplaceNode(ReplaceNodeEdit {
        replacement: tree(roots, nodes),
        retained_children: None,
    })
}

fn replace_single(text: &str) -> TreeEdit {
    replace_nodes(vec![node(
        0,
        GeneratedRawNodeKind::Simple,
        text,
        Vec::new(),
    )])
}

fn changed(edit: TreeEdit) -> TreeMacroOutput {
    TreeMacroOutput {
        decision: HookDecision::ContinueProcessing,
        edit: Some(edit),
        effects: empty_effects(),
    }
}

fn unchanged() -> TreeMacroOutput {
    TreeMacroOutput {
        decision: HookDecision::ContinueProcessing,
        edit: None,
        effects: empty_effects(),
    }
}

fn empty_effects() -> HookEffects {
    HookEffects {
        diagnostics: Vec::new(),
        context_updates: Vec::new(),
        parse_requests: Vec::new(),
    }
}

impl hooks::Guest for TreeMacroAddon {
    fn invoke(_input: HookInvocation) -> Result<HookOutput, AddonError> {
        Err(unsupported("hook"))
    }
}

impl text_macro::Guest for TreeMacroAddon {
    fn expand(_input: TextMacroInput) -> Result<TextMacroOutput, AddonError> {
        Err(unsupported("text macro"))
    }
}

impl ast_macro::Guest for TreeMacroAddon {
    fn expand(_input: AstMacroInput) -> Result<AstMacroOutput, AddonError> {
        Err(unsupported("AST macro"))
    }
}

fn unsupported(kind: &str) -> AddonError {
    addon_error(
        AddonErrorKind::UnsupportedCapability,
        format!("tree macro fixture does not register a {kind}"),
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
export!(TreeMacroAddon);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_declares_tree_phase_and_private_state() {
        let manifest = <TreeMacroAddon as addon::Guest>::manifest();
        assert_eq!(manifest.component_id, COMPONENT_ID);
        assert_eq!(manifest.subscriptions.len(), 1);
        assert!(matches!(manifest.subscriptions[0].phase, HookPhase::Tree));
        assert!(matches!(
            manifest.subscriptions[0].mode,
            HookMode::Transform
        ));
        assert_eq!(manifest.state_namespaces.len(), 1);
    }
}

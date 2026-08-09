#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

//! Test Component for Effect lifecycle replacement, rejection, and state rollback.
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
        AbiVersion, AddonError, AddonErrorKind, AstMacroInput, AstMacroOutput,
        CapabilityRequirement, CompatibilityError, ComponentManifest, Diagnostic,
        DiagnosticSeverity, EffectTiming, HookDecision, HookEffects, HookInvocation, HookMode,
        HookOutput, HookPayload, HookPhase, HookSubscription, HookTarget, HostProfile, Rejection,
        StateEncoding, StateNamespaceDeclaration, StateNamespaceVisibility, StateScope, StateValue,
        SyntaxKind, TextMacroInput, TextMacroOutput, TreeMacroInput, TreeMacroOutput,
    },
};
use parser_wasm::{
    ABI_VERSION, AbiVersion as ParserAbiVersion, CAPABILITY_EFFECT_PARSER, CAPABILITY_STATE_STORE,
    Capability as ParserCapability, CapabilityRequirement as ParserCapabilityRequirement,
    validate_compatibility,
};

const COMPONENT_ID: &str = "test.effect-addon";
const CATEGORY_SUBSCRIPTION: &str = "effect.category";
const REPLACE_SUBSCRIPTION: &str = "effect.replace";
const REJECT_SUBSCRIPTION: &str = "effect.reject";
const REPLACE_REGISTRATION: &str = "effect:skriptdummyaddon:e5a642b47ab7df334a25242d4626e480fcf4a4ecb07bd4a4124d973d7c337d5f:9c25d42b2baa05a39b30588bfc37b2996faba3d05060ba445e0c09893a96ccfc:0";
const REJECT_REGISTRATION: &str = "effect:skriptdummyaddon:224a969f6e9d408a3346b355ad040b4e4d82122708036cc40768e2d594725925:b3845096cfe66e4b677f17594ff4e1c1046c24b7526fe6452fbaabb0d9007f99:0";
const STATE_NAMESPACE: &str = "effect-state";
const STATE_SCHEMA: &str = "nlaocs.test.effect-state";

struct EffectAddon;

impl addon::Guest for EffectAddon {
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
                    id: CAPABILITY_EFFECT_PARSER.to_owned(),
                    minimum_version: 1,
                    required: true,
                },
                CapabilityRequirement {
                    id: CAPABILITY_STATE_STORE.to_owned(),
                    minimum_version: 1,
                    required: true,
                },
            ],
            subscriptions: vec![
                HookSubscription {
                    id: CATEGORY_SUBSCRIPTION.to_owned(),
                    target: HookTarget::SyntaxDefinition(SyntaxKind::Effect),
                    phase: HookPhase::Effect,
                    priority: 0,
                    mode: HookMode::Transform,
                    capability_id: CAPABILITY_EFFECT_PARSER.to_owned(),
                },
                HookSubscription {
                    id: REPLACE_SUBSCRIPTION.to_owned(),
                    target: HookTarget::ExactRegistration(REPLACE_REGISTRATION.to_owned()),
                    phase: HookPhase::Effect,
                    priority: -10,
                    mode: HookMode::Override,
                    capability_id: CAPABILITY_EFFECT_PARSER.to_owned(),
                },
                HookSubscription {
                    id: REJECT_SUBSCRIPTION.to_owned(),
                    target: HookTarget::ExactRegistration(REJECT_REGISTRATION.to_owned()),
                    phase: HookPhase::Effect,
                    priority: -10,
                    mode: HookMode::Override,
                    capability_id: CAPABILITY_EFFECT_PARSER.to_owned(),
                },
            ],
            registered_syntax_handlers: Vec::new(),
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
        let capabilities = profile
            .capabilities
            .into_iter()
            .map(|value| ParserCapability::new(value.id, value.version))
            .collect::<Vec<_>>();
        validate_compatibility(
            ABI_VERSION,
            ParserAbiVersion::new(profile.abi.major, profile.abi.minor),
            &[
                ParserCapabilityRequirement::required(CAPABILITY_EFFECT_PARSER, 1),
                ParserCapabilityRequirement::required(CAPABILITY_STATE_STORE, 1),
            ],
            &capabilities,
        )
        .map_err(|error| CompatibilityError {
            kind: nlaocs::skript_parser_addon::types::CompatibilityErrorKind::InvalidManifest,
            subject: COMPONENT_ID.to_owned(),
            message: error.to_string(),
        })
    }
}

impl hooks::Guest for EffectAddon {
    fn invoke(input: HookInvocation) -> Result<HookOutput, AddonError> {
        let HookPayload::Effect(mut payload) = input.payload else {
            return Err(addon_error(
                "Effect subscription received another payload kind",
            ));
        };
        match input.context.subscription_id.as_str() {
            CATEGORY_SUBSCRIPTION => {
                record_state(match payload.timing {
                    EffectTiming::Before => "category-before",
                    EffectTiming::After => "category-after",
                })?;
                Ok(HookOutput {
                    decision: HookDecision::ContinueProcessing,
                    replacement: None,
                    effects: empty_effects(),
                })
            }
            REPLACE_SUBSCRIPTION => {
                record_state("replace")?;
                let candidate = payload
                    .candidate
                    .as_mut()
                    .ok_or_else(|| addon_error("replace hook requires a candidate"))?;
                candidate
                    .metadata
                    .push(nlaocs::skript_parser_addon::types::MetadataEntry {
                        key: "wasm".to_owned(),
                        value: "replaced".to_owned(),
                    });
                Ok(HookOutput {
                    decision: HookDecision::Handled,
                    replacement: Some(HookPayload::Effect(payload)),
                    effects: empty_effects(),
                })
            }
            REJECT_SUBSCRIPTION => {
                record_state("reject")?;
                Ok(HookOutput {
                    decision: HookDecision::Reject(Rejection {
                        reason: "rejected by Effect fixture".to_owned(),
                        diagnostics: vec![Diagnostic {
                            code: "effect-fixture-reject".to_owned(),
                            message: "Effect rejected by the fixture addon".to_owned(),
                            severity: DiagnosticSeverity::Warning,
                            span: payload.span,
                            related: Vec::new(),
                        }],
                    }),
                    replacement: None,
                    effects: empty_effects(),
                })
            }
            _ => Err(addon_error("unknown Effect subscription")),
        }
    }
}

impl text_macro::Guest for EffectAddon {
    fn expand(_input: TextMacroInput) -> Result<TextMacroOutput, AddonError> {
        Err(addon_error("text macro is unsupported"))
    }
}

impl tree_macro::Guest for EffectAddon {
    fn expand(_input: TreeMacroInput) -> Result<TreeMacroOutput, AddonError> {
        Err(addon_error("tree macro is unsupported"))
    }
}

impl ast_macro::Guest for EffectAddon {
    fn expand(_input: AstMacroInput) -> Result<AstMacroOutput, AddonError> {
        Err(addon_error("AST macro is unsupported"))
    }
}

fn record_state(key: &str) -> Result<(), AddonError> {
    state_store::put(
        StateScope::Parse,
        StateNamespaceVisibility::Private,
        STATE_NAMESPACE,
        key,
        &StateValue {
            schema_id: STATE_SCHEMA.to_owned(),
            encoding: StateEncoding::Json,
            bytes: b"true".to_vec(),
        },
    )
    .map_err(|error| addon_error(error.message))
}

fn empty_effects() -> HookEffects {
    HookEffects {
        diagnostics: Vec::new(),
        context_updates: Vec::new(),
        parse_requests: Vec::new(),
    }
}

fn addon_error(message: impl Into<String>) -> AddonError {
    AddonError {
        kind: AddonErrorKind::InvalidPayload,
        message: message.into(),
        diagnostics: Vec::new(),
    }
}

#[cfg(target_arch = "wasm32")]
export!(EffectAddon);

#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

//! Test Component for dynamic syntax registration and override lifecycles.
//!
//! It intentionally exercises successful updates, invalid registrations, rollback,
//! document prepass behavior, and component unload cleanup through the real WIT ABI.
#![allow(missing_docs)] // `wit_bindgen` generates the exported guest API.

wit_bindgen::generate!({
    path: "../../parser-wasm/wit",
    world: "parser-addon",
    generate_unused_types: true,
});

use exports::nlaocs::skript_parser_addon::{addon, ast_macro, hooks, text_macro, tree_macro};
use nlaocs::skript_parser_addon::{
    dynamic_syntax_registry,
    types::{
        AbiVersion, AddonError, AddonErrorKind, AstMacroInput, AstMacroOutput,
        CapabilityRequirement, CompatibilityError, CompatibilityErrorKind, ComponentManifest,
        DynamicSyntaxDefinition, DynamicSyntaxId, DynamicSyntaxOverride,
        DynamicSyntaxOverrideTarget, DynamicSyntaxReference, HookDecision, HookEffects,
        HookInvocation, HookMode, HookOutput, HookPayload, HookPhase, HookSubscription, HookTarget,
        HostProfile, Rejection, SyntaxKind, TextMacroInput, TextMacroOutput, TreeMacroInput,
        TreeMacroOutput,
    },
};
use parser_wasm::{
    ABI_VERSION, AbiVersion as ParserAbiVersion, CAPABILITY_DYNAMIC_SYNTAX, CAPABILITY_HOOKS,
    Capability as ParserCapability, CapabilityRequirement as ParserCapabilityRequirement,
    CompatibilityError as ParserCompatibilityError, validate_compatibility,
};

const COMPONENT_ID: &str = "nlaocs.test.dynamic-syntax";
const PREPASS_SUBSCRIPTION_ID: &str = "dynamic.prepass";
const DELAY_DEFINITION_ID: &str =
    "effect:skript:751b28432979bd1f00e370ffe6f6c3279e4936b90071eda5ed732d7cda2c0504";

struct DynamicSyntaxAddon;

impl addon::Guest for DynamicSyntaxAddon {
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
                    id: CAPABILITY_HOOKS.to_owned(),
                    minimum_version: 1,
                    required: true,
                },
                CapabilityRequirement {
                    id: CAPABILITY_DYNAMIC_SYNTAX.to_owned(),
                    minimum_version: 1,
                    required: true,
                },
            ],
            subscriptions: vec![HookSubscription {
                id: PREPASS_SUBSCRIPTION_ID.to_owned(),
                target: HookTarget::ParseStage,
                phase: HookPhase::Document,
                priority: 0,
                mode: HookMode::Override,
                capability_id: CAPABILITY_HOOKS.to_owned(),
            }],
            registered_syntax_handlers: Vec::new(),
            state_namespaces: Vec::new(),
        }
    }

    fn initialize(profile: HostProfile) -> Result<(), CompatibilityError> {
        validate_profile(profile)?;
        dynamic_syntax_registry::register(&DynamicSyntaxDefinition {
            local_id: "initial-effect".to_owned(),
            kind: SyntaxKind::Effect,
            patterns: vec!["dummy initialize %string%".to_owned()],
            priority: -20,
            before: Vec::new(),
            after: Vec::new(),
            return_type: None,
            return_multiplicity: None,
            handler: "dynamic.initial-effect".to_owned(),
            metadata: Vec::new(),
        })
        .map_err(dynamic_error)?;
        dynamic_syntax_registry::register_override(&DynamicSyntaxOverride {
            local_id: "delay-override".to_owned(),
            target: DynamicSyntaxOverrideTarget::DefinitionId(DELAY_DEFINITION_ID.to_owned()),
            priority: -100,
            handler: "dynamic.delay-override".to_owned(),
            metadata: Vec::new(),
        })
        .map_err(dynamic_error)
    }
}

impl hooks::Guest for DynamicSyntaxAddon {
    fn invoke(input: HookInvocation) -> Result<HookOutput, AddonError> {
        if input.context.subscription_id != PREPASS_SUBSCRIPTION_ID
            || !matches!(input.phase, HookPhase::Document)
        {
            return Err(addon_error(
                AddonErrorKind::UnsupportedHook,
                "dynamic syntax fixture only handles its document prepass subscription",
            ));
        }

        let reject = matches!(
            &input.payload,
            HookPayload::Document(document) if document.text == "reject"
        );
        let _ = dynamic_syntax_registry::remove("prepass-effect");
        dynamic_syntax_registry::register(&DynamicSyntaxDefinition {
            local_id: "prepass-effect".to_owned(),
            kind: SyntaxKind::Effect,
            patterns: vec!["dummy prepass %string%".to_owned()],
            priority: -10,
            before: Vec::new(),
            after: vec![DynamicSyntaxReference::Dynamic(DynamicSyntaxId {
                component_id: None,
                local_id: "initial-effect".to_owned(),
            })],
            return_type: None,
            return_multiplicity: None,
            handler: "dynamic.prepass-effect".to_owned(),
            metadata: Vec::new(),
        })
        .map_err(|error| {
            addon_error(
                AddonErrorKind::Internal,
                format!("dynamic prepass registration failed: {}", error.message),
            )
        })?;

        Ok(HookOutput {
            decision: if reject {
                HookDecision::Reject(Rejection {
                    reason: "fixture requested rollback".to_owned(),
                    diagnostics: Vec::new(),
                })
            } else {
                HookDecision::ContinueProcessing
            },
            replacement: None,
            effects: empty_effects(),
        })
    }
}

impl text_macro::Guest for DynamicSyntaxAddon {
    fn expand(_input: TextMacroInput) -> Result<TextMacroOutput, AddonError> {
        Err(unsupported_macro("text"))
    }
}

impl tree_macro::Guest for DynamicSyntaxAddon {
    fn expand(_input: TreeMacroInput) -> Result<TreeMacroOutput, AddonError> {
        Err(unsupported_macro("tree"))
    }
}

impl ast_macro::Guest for DynamicSyntaxAddon {
    fn expand(_input: AstMacroInput) -> Result<AstMacroOutput, AddonError> {
        Err(unsupported_macro("AST"))
    }
}

fn validate_profile(profile: HostProfile) -> Result<(), CompatibilityError> {
    let requirements = [
        ParserCapabilityRequirement::required(CAPABILITY_HOOKS, 1),
        ParserCapabilityRequirement::required(CAPABILITY_DYNAMIC_SYNTAX, 1),
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

fn dynamic_error(
    error: nlaocs::skript_parser_addon::types::DynamicRegistryError,
) -> CompatibilityError {
    CompatibilityError {
        kind: CompatibilityErrorKind::InvalidManifest,
        subject: CAPABILITY_DYNAMIC_SYNTAX.to_owned(),
        message: error.message,
    }
}

fn empty_effects() -> HookEffects {
    HookEffects {
        diagnostics: Vec::new(),
        context_updates: Vec::new(),
        parse_requests: Vec::new(),
    }
}

fn unsupported_macro(kind: &str) -> AddonError {
    addon_error(
        AddonErrorKind::UnsupportedCapability,
        format!("dynamic syntax fixture does not register a {kind} macro"),
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
export!(DynamicSyntaxAddon);

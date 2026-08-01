#![cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
#![doc = include_str!("../README.md")]
#![warn(rustdoc::broken_intra_doc_links)]
#![allow(missing_docs)] // `wit_bindgen` owns the generated guest surface.

wit_bindgen::generate!({
    path: "../parser-wasm/wit",
    world: "parser-addon",
    generate_unused_types: true,
});

use exports::nlaocs::skript_parser_addon::{addon, ast_macro, hooks, text_macro, tree_macro};
use nlaocs::skript_parser_addon::types::{
    AbiVersion, AddonError, AddonErrorKind, AstMacroInput, AstMacroOutput, CapabilityRequirement,
    CompatibilityError, CompatibilityErrorKind, ComponentManifest, HookDecision, HookEffects,
    HookInvocation, HookMode, HookOutput, HookPayload, HookPhase, HookSubscription, HookTarget,
    HostProfile, TextMacroInput, TextMacroOutput, TreeMacroInput, TreeMacroOutput,
};
use parser_wasm::{
    ABI_VERSION, AbiVersion as ParserAbiVersion, CAPABILITY_HOOKS, Capability as ParserCapability,
    CapabilityRequirement as ParserCapabilityRequirement,
    CompatibilityError as ParserCompatibilityError, validate_compatibility,
};

const COMPONENT_ID: &str = "nlaocs.core-library";
const COMPONENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const HEALTH_CHECK_SUBSCRIPTION_ID: &str = "core.health-check";

struct CoreLibrary;

impl addon::Guest for CoreLibrary {
    fn manifest() -> ComponentManifest {
        ComponentManifest {
            component_id: COMPONENT_ID.to_owned(),
            component_version: COMPONENT_VERSION.to_owned(),
            abi: AbiVersion {
                major: ABI_VERSION.major,
                minor: ABI_VERSION.minor,
            },
            capabilities: vec![CapabilityRequirement {
                id: CAPABILITY_HOOKS.to_owned(),
                minimum_version: 1,
                required: true,
            }],
            subscriptions: vec![HookSubscription {
                id: HEALTH_CHECK_SUBSCRIPTION_ID.to_owned(),
                target: HookTarget::ParseStage,
                phase: HookPhase::Document,
                priority: i32::MIN,
                mode: HookMode::Observe,
                capability_id: CAPABILITY_HOOKS.to_owned(),
            }],
            state_namespaces: Vec::new(),
        }
    }

    fn initialize(profile: HostProfile) -> Result<(), CompatibilityError> {
        let requirements = [ParserCapabilityRequirement::required(CAPABILITY_HOOKS, 1)];
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

impl hooks::Guest for CoreLibrary {
    fn invoke(input: HookInvocation) -> Result<HookOutput, AddonError> {
        if input.context.subscription_id != HEALTH_CHECK_SUBSCRIPTION_ID {
            return Err(addon_error(
                AddonErrorKind::UnsupportedHook,
                format!(
                    "unknown CoreLibrary hook subscription: {}",
                    input.context.subscription_id
                ),
            ));
        }
        if !matches!(input.target, HookTarget::ParseStage)
            || !matches!(input.phase, HookPhase::Document)
            || !matches!(input.payload, HookPayload::Document(_))
        {
            return Err(addon_error(
                AddonErrorKind::InvalidPayload,
                "CoreLibrary health check requires a Document parse-stage payload",
            ));
        }

        Ok(HookOutput {
            decision: HookDecision::ContinueProcessing,
            replacement: None,
            effects: empty_effects(),
        })
    }
}

impl text_macro::Guest for CoreLibrary {
    fn expand(_input: TextMacroInput) -> Result<TextMacroOutput, AddonError> {
        Err(unsupported_macro("text"))
    }
}

impl tree_macro::Guest for CoreLibrary {
    fn expand(_input: TreeMacroInput) -> Result<TreeMacroOutput, AddonError> {
        Err(unsupported_macro("tree"))
    }
}

impl ast_macro::Guest for CoreLibrary {
    fn expand(_input: AstMacroInput) -> Result<AstMacroOutput, AddonError> {
        Err(unsupported_macro("AST"))
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
        format!("CoreLibrary does not register a {kind} macro"),
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
export!(CoreLibrary);

#[cfg(test)]
mod tests {
    use super::*;
    use nlaocs::skript_parser_addon::types::{
        Capability, DocumentPayload, HookPayload, InvocationContext,
    };

    #[test]
    fn manifest_exposes_identity_abi_and_health_check() {
        let manifest = <CoreLibrary as addon::Guest>::manifest();

        assert_eq!(manifest.component_id, COMPONENT_ID);
        assert_eq!(manifest.component_version, COMPONENT_VERSION);
        assert_eq!(manifest.abi.major, ABI_VERSION.major);
        assert_eq!(manifest.abi.minor, ABI_VERSION.minor);
        assert_eq!(manifest.capabilities.len(), 1);
        assert_eq!(manifest.capabilities[0].id, CAPABILITY_HOOKS);
        assert!(manifest.capabilities[0].required);
        assert_eq!(manifest.subscriptions.len(), 1);
        assert_eq!(manifest.subscriptions[0].id, HEALTH_CHECK_SUBSCRIPTION_ID);
        assert!(matches!(
            manifest.subscriptions[0].target,
            HookTarget::ParseStage
        ));
        assert!(matches!(
            manifest.subscriptions[0].phase,
            HookPhase::Document
        ));
    }

    #[test]
    fn initialization_rejects_incompatible_hosts() {
        let missing = <CoreLibrary as addon::Guest>::initialize(HostProfile {
            abi: AbiVersion {
                major: ABI_VERSION.major,
                minor: ABI_VERSION.minor,
            },
            capabilities: Vec::new(),
        })
        .unwrap_err();
        assert!(matches!(
            missing.kind,
            CompatibilityErrorKind::MissingCapability
        ));

        let wrong_abi = <CoreLibrary as addon::Guest>::initialize(HostProfile {
            abi: AbiVersion {
                major: ABI_VERSION.major + 1,
                minor: 0,
            },
            capabilities: vec![Capability {
                id: CAPABILITY_HOOKS.to_owned(),
                version: 1,
            }],
        })
        .unwrap_err();
        assert!(matches!(
            wrong_abi.kind,
            CompatibilityErrorKind::AbiVersionMismatch
        ));
    }

    #[test]
    fn health_check_continues_without_side_effects() {
        let output = <CoreLibrary as hooks::Guest>::invoke(health_check_invocation()).unwrap();

        assert!(matches!(output.decision, HookDecision::ContinueProcessing));
        assert!(output.replacement.is_none());
        assert!(output.effects.diagnostics.is_empty());
        assert!(output.effects.context_updates.is_empty());
        assert!(output.effects.parse_requests.is_empty());
    }

    #[test]
    fn health_check_rejects_unknown_subscriptions() {
        let mut input = health_check_invocation();
        input.context.subscription_id = "unknown".to_owned();

        let error = <CoreLibrary as hooks::Guest>::invoke(input).unwrap_err();
        assert!(matches!(error.kind, AddonErrorKind::UnsupportedHook));
    }

    fn health_check_invocation() -> HookInvocation {
        HookInvocation {
            context: InvocationContext {
                invocation_id: 1,
                subscription_id: HEALTH_CHECK_SUBSCRIPTION_ID.to_owned(),
                document_id: "file:///health-check.sk".to_owned(),
                document_revision: 1,
                expansion: None,
                syntax_context: 0,
            },
            target: HookTarget::ParseStage,
            phase: HookPhase::Document,
            payload: HookPayload::Document(DocumentPayload {
                text: String::new(),
            }),
        }
    }
}

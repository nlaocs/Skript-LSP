#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

//! Test Component for registered-pattern matching hooks.
//!
//! Its subscriptions observe nested matcher scopes, override selected candidates,
//! emit effects, and write state so host-side rollback can be asserted end to end.
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
        CapabilityRequirement, CompatibilityError, ComponentManifest, ContextUpdate, HookDecision,
        HookEffects, HookInvocation, HookMode, HookOutput, HookPayload, HookPhase, HookSelector,
        HookSubscription, HookTarget, HostProfile, MatchingScope, MatchingStatus, MatchingTiming,
        StateEncoding, StateNamespaceDeclaration, StateNamespaceVisibility, StateScope, StateValue,
        TextMacroInput, TextMacroOutput, TreeMacroInput, TreeMacroOutput,
    },
};
use parser_wasm::{
    ABI_VERSION, AbiVersion as ParserAbiVersion, CAPABILITY_HOOKS, CAPABILITY_STATE_STORE,
    Capability as ParserCapability, CapabilityRequirement as ParserCapabilityRequirement,
    validate_compatibility,
};

const COMPONENT_ID: &str = "test.matching-addon";
const SUBSCRIPTION_ID: &str = "matching.force-element";
const REGISTRATION_ID: &str = "effect:hook-override#0";
const STATE_NAMESPACE: &str = "matching-state";
const STATE_SCHEMA: &str = "nlaocs.test.matching-state";

struct MatchingAddon;

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

impl addon::Guest for MatchingAddon {
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
                    id: CAPABILITY_STATE_STORE.to_owned(),
                    minimum_version: 1,
                    required: true,
                },
            ],
            subscriptions: vec![HookSubscription {
                id: SUBSCRIPTION_ID.to_owned(),
                target: HookTarget::Registration(REGISTRATION_ID.to_owned()),
                phase: HookPhase::Matching,
                priority: 0,
                mode: HookMode::Override,
                capability_id: CAPABILITY_HOOKS.to_owned(),
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
        let capabilities = profile
            .capabilities
            .into_iter()
            .map(|value| ParserCapability::new(value.id, value.version))
            .collect::<Vec<_>>();
        validate_compatibility(
            ABI_VERSION,
            ParserAbiVersion::new(profile.abi.major, profile.abi.minor),
            &[
                ParserCapabilityRequirement::required(CAPABILITY_HOOKS, 1),
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

impl hooks::Guest for MatchingAddon {
    fn invoke(input: HookInvocation) -> Result<HookOutput, AddonError> {
        let HookPayload::Matching(mut payload) = input.payload else {
            return Err(addon_error(
                "matching subscription received a non-matching payload",
            ));
        };
        if input.context.subscription_id != SUBSCRIPTION_ID {
            return Err(addon_error("unknown matching subscription"));
        }
        record_invocation(&payload)?;
        let effects = invocation_effects(&payload);

        if payload.scope == MatchingScope::Element
            && payload.timing == MatchingTiming::Before
            && payload.status == MatchingStatus::Pending
        {
            payload.input_range.end = payload.input.len() as u64;
            payload.status = MatchingStatus::Matched;
            payload.failure_reason = None;
            return Ok(HookOutput {
                decision: HookDecision::Handled,
                replacement: Some(HookPayload::Matching(payload)),
                effects,
            });
        }

        Ok(HookOutput {
            decision: HookDecision::ContinueProcessing,
            replacement: None,
            effects,
        })
    }
}

impl text_macro::Guest for MatchingAddon {
    fn expand(_input: TextMacroInput) -> Result<TextMacroOutput, AddonError> {
        Err(addon_error("text macro is unsupported"))
    }
}

impl tree_macro::Guest for MatchingAddon {
    fn expand(_input: TreeMacroInput) -> Result<TreeMacroOutput, AddonError> {
        Err(addon_error("tree macro is unsupported"))
    }
}

impl ast_macro::Guest for MatchingAddon {
    fn expand(_input: AstMacroInput) -> Result<AstMacroOutput, AddonError> {
        Err(addon_error("AST macro is unsupported"))
    }
}

fn record_invocation(
    payload: &nlaocs::skript_parser_addon::types::MatchingPayload,
) -> Result<(), AddonError> {
    let scope = match payload.scope {
        MatchingScope::Definition => "definition",
        MatchingScope::Registration => "registration",
        MatchingScope::Pattern => "pattern",
        MatchingScope::Element => "element",
    };
    let timing = match payload.timing {
        MatchingTiming::Before => "before",
        MatchingTiming::After => "after",
    };
    state_store::put(
        StateScope::Parse,
        StateNamespaceVisibility::Private,
        STATE_NAMESPACE,
        &format!("{}:{scope}:{timing}", payload.definition_id),
        &StateValue {
            schema_id: STATE_SCHEMA.to_owned(),
            encoding: StateEncoding::Json,
            bytes: payload.input_range.end.to_string().into_bytes(),
        },
    )
    .map_err(|error| {
        addon_error(format!(
            "failed to record matching state: {}",
            error.message
        ))
    })
}

fn invocation_effects(
    payload: &nlaocs::skript_parser_addon::types::MatchingPayload,
) -> HookEffects {
    HookEffects {
        diagnostics: Vec::new(),
        context_updates: vec![ContextUpdate {
            syntax_context: 0,
            key: payload.definition_id.clone(),
            value: None,
        }],
        parse_requests: Vec::new(),
        parse_results: Vec::new(),
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
export!(MatchingAddon);

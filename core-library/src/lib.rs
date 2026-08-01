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
    CompatibilityError, CompatibilityErrorKind, ComponentManifest, DynamicMultiplicity,
    ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload, HookDecision, HookEffects,
    HookInvocation, HookMode, HookOutput, HookPayload, HookPhase, HookSubscription, HookTarget,
    HostProfile, MetadataEntry, TextMacroInput, TextMacroOutput, TextRange, TreeMacroInput,
    TreeMacroOutput,
};
use parser_wasm::{
    ABI_VERSION, AbiVersion as ParserAbiVersion, CAPABILITY_EXPRESSION_PARSER, CAPABILITY_HOOKS,
    Capability as ParserCapability, CapabilityRequirement as ParserCapabilityRequirement,
    CompatibilityError as ParserCompatibilityError, validate_compatibility,
};

const COMPONENT_ID: &str = "nlaocs.core-library";
const COMPONENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const HEALTH_CHECK_SUBSCRIPTION_ID: &str = "core.health-check";
const EXPRESSION_SUBSCRIPTION_ID: &str = "core.expression-leaves";

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
            capabilities: vec![
                CapabilityRequirement {
                    id: CAPABILITY_HOOKS.to_owned(),
                    minimum_version: 1,
                    required: true,
                },
                CapabilityRequirement {
                    id: CAPABILITY_EXPRESSION_PARSER.to_owned(),
                    minimum_version: 1,
                    required: true,
                },
            ],
            subscriptions: vec![
                HookSubscription {
                    id: HEALTH_CHECK_SUBSCRIPTION_ID.to_owned(),
                    target: HookTarget::ParseStage,
                    phase: HookPhase::Document,
                    priority: i32::MIN,
                    mode: HookMode::Observe,
                    capability_id: CAPABILITY_HOOKS.to_owned(),
                },
                HookSubscription {
                    id: EXPRESSION_SUBSCRIPTION_ID.to_owned(),
                    target: HookTarget::ParseStage,
                    phase: HookPhase::Expression,
                    priority: i32::MIN,
                    mode: HookMode::Transform,
                    capability_id: CAPABILITY_EXPRESSION_PARSER.to_owned(),
                },
            ],
            state_namespaces: Vec::new(),
        }
    }

    fn initialize(profile: HostProfile) -> Result<(), CompatibilityError> {
        let requirements = [
            ParserCapabilityRequirement::required(CAPABILITY_HOOKS, 1),
            ParserCapabilityRequirement::required(CAPABILITY_EXPRESSION_PARSER, 1),
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

impl hooks::Guest for CoreLibrary {
    fn invoke(input: HookInvocation) -> Result<HookOutput, AddonError> {
        match input.context.subscription_id.as_str() {
            HEALTH_CHECK_SUBSCRIPTION_ID => health_check(input),
            EXPRESSION_SUBSCRIPTION_ID => parse_expression_leaves(input),
            _ => Err(addon_error(
                AddonErrorKind::UnsupportedHook,
                format!(
                    "unknown CoreLibrary hook subscription: {}",
                    input.context.subscription_id
                ),
            )),
        }
    }
}

fn health_check(input: HookInvocation) -> Result<HookOutput, AddonError> {
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

fn parse_expression_leaves(input: HookInvocation) -> Result<HookOutput, AddonError> {
    if !matches!(input.target, HookTarget::ParseStage)
        || !matches!(input.phase, HookPhase::Expression)
    {
        return Err(addon_error(
            AddonErrorKind::InvalidPayload,
            "CoreLibrary Expression parser requires an Expression parse-stage payload",
        ));
    }
    let HookPayload::Expression(mut payload) = input.payload else {
        return Err(addon_error(
            AddonErrorKind::InvalidPayload,
            "CoreLibrary Expression parser requires an Expression payload",
        ));
    };

    if let Some(candidate) = core_expression_candidate(&payload) {
        payload.candidates.push(candidate);
        Ok(HookOutput {
            decision: HookDecision::ContinueProcessing,
            replacement: Some(HookPayload::Expression(payload)),
            effects: empty_effects(),
        })
    } else {
        Ok(HookOutput {
            decision: HookDecision::ContinueProcessing,
            replacement: None,
            effects: empty_effects(),
        })
    }
}

fn core_expression_candidate(payload: &ExpressionPayload) -> Option<ExpressionLeafCandidate> {
    let candidates = payload
        .candidate_ends
        .iter()
        .copied()
        .rev()
        .filter_map(|end| expression_slice(payload, end).map(|text| (end, text)));

    for (end, text) in candidates {
        if payload.allow_expressions && is_variable(text) {
            let plural = text.contains("::*")
                || payload
                    .expected_types
                    .first()
                    .is_some_and(|expected| expected.plural);
            return Some(expression_candidate(
                "core.variable",
                ExpressionLeafKind::Variable,
                payload.remaining.start,
                end,
                payload
                    .expected_types
                    .first()
                    .map_or("java.lang.Object", |expected| expected.class_name.as_str()),
                if plural {
                    DynamicMultiplicity::Multiple
                } else {
                    DynamicMultiplicity::Single
                },
            ));
        }
        if payload.allow_literals && is_string_literal(text) {
            return Some(expression_candidate(
                "core.literal.string",
                ExpressionLeafKind::Literal,
                payload.remaining.start,
                end,
                "java.lang.String",
                DynamicMultiplicity::Single,
            ));
        }
        if payload.allow_literals && is_number_literal(text) {
            let return_type = if text.contains(['.', 'e', 'E']) {
                "java.lang.Double"
            } else {
                "java.lang.Long"
            };
            return Some(expression_candidate(
                "core.literal.number",
                ExpressionLeafKind::Literal,
                payload.remaining.start,
                end,
                return_type,
                DynamicMultiplicity::Single,
            ));
        }
    }
    None
}

fn expression_slice(payload: &ExpressionPayload, end: u64) -> Option<&str> {
    let start = usize::try_from(payload.remaining.start).ok()?;
    let end = usize::try_from(end).ok()?;
    let remaining_end = usize::try_from(payload.remaining.end).ok()?;
    if start > end || end > remaining_end {
        return None;
    }
    payload.input.get(start..end)
}

fn is_variable(text: &str) -> bool {
    text.len() >= 3
        && text.starts_with('{')
        && text.ends_with('}')
        && !text[1..text.len() - 1].trim().is_empty()
}

fn is_string_literal(text: &str) -> bool {
    if text.len() < 2 || !text.starts_with('"') || !text.ends_with('"') {
        return false;
    }
    let inner = &text[1..text.len() - 1];
    let mut chars = inner.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' && chars.next_if_eq(&'"').is_none() {
            return false;
        }
    }
    true
}

fn is_number_literal(text: &str) -> bool {
    !text.is_empty() && text.parse::<f64>().is_ok_and(|value| value.is_finite())
}

fn expression_candidate(
    parser_id: &str,
    kind: ExpressionLeafKind,
    start: u64,
    end: u64,
    return_type: &str,
    multiplicity: DynamicMultiplicity,
) -> ExpressionLeafCandidate {
    ExpressionLeafCandidate {
        parser_id: parser_id.to_owned(),
        kind,
        range: TextRange { start, end },
        return_type: Some(return_type.to_owned()),
        multiplicity: Some(multiplicity),
        metadata: Vec::<MetadataEntry>::new(),
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
        Capability, DocumentPayload, ExpressionExpectedType, HookPayload, InvocationContext,
        MappedSpan, OriginKind, SourceOrigin,
    };

    #[test]
    fn manifest_exposes_identity_abi_and_health_check() {
        let manifest = <CoreLibrary as addon::Guest>::manifest();

        assert_eq!(manifest.component_id, COMPONENT_ID);
        assert_eq!(manifest.component_version, COMPONENT_VERSION);
        assert_eq!(manifest.abi.major, ABI_VERSION.major);
        assert_eq!(manifest.abi.minor, ABI_VERSION.minor);
        assert_eq!(manifest.capabilities.len(), 2);
        assert_eq!(manifest.capabilities[0].id, CAPABILITY_HOOKS);
        assert!(manifest.capabilities[0].required);
        assert_eq!(manifest.subscriptions.len(), 2);
        assert_eq!(manifest.subscriptions[0].id, HEALTH_CHECK_SUBSCRIPTION_ID);
        assert!(matches!(
            manifest.subscriptions[0].target,
            HookTarget::ParseStage
        ));
        assert!(matches!(
            manifest.subscriptions[0].phase,
            HookPhase::Document
        ));
        assert_eq!(manifest.capabilities[1].id, CAPABILITY_EXPRESSION_PARSER);
        assert_eq!(manifest.subscriptions[1].id, EXPRESSION_SUBSCRIPTION_ID);
        assert!(matches!(
            manifest.subscriptions[1].phase,
            HookPhase::Expression
        ));
        assert!(matches!(
            manifest.subscriptions[1].mode,
            HookMode::Transform
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
    fn expression_hook_recognizes_variable_string_and_number_leaves() {
        let cases = [
            ("{value}", "core.variable", ExpressionLeafKind::Variable),
            (
                "\"hello\"",
                "core.literal.string",
                ExpressionLeafKind::Literal,
            ),
            ("-42.5", "core.literal.number", ExpressionLeafKind::Literal),
        ];
        for (text, parser_id, kind) in cases {
            let output = <CoreLibrary as hooks::Guest>::invoke(expression_invocation(text))
                .expect("CoreLibrary Expression invocation must succeed");
            let Some(HookPayload::Expression(payload)) = output.replacement else {
                panic!("recognized leaf must replace the Expression payload");
            };
            assert_eq!(payload.candidates.len(), 1);
            assert_eq!(payload.candidates[0].parser_id, parser_id);
            assert_eq!(payload.candidates[0].kind, kind);
            assert_eq!(payload.candidates[0].range.end, text.len() as u64);
        }
    }

    #[test]
    fn expression_hook_leaves_unknown_input_untouched() {
        let output = <CoreLibrary as hooks::Guest>::invoke(expression_invocation("not a leaf"))
            .expect("unknown input is a normal no-match");
        assert!(output.replacement.is_none());
        assert!(matches!(output.decision, HookDecision::ContinueProcessing));
    }

    #[test]
    fn health_check_rejects_unknown_subscriptions() {
        let mut input = health_check_invocation();
        input.context.subscription_id = "unknown".to_owned();

        let error = <CoreLibrary as hooks::Guest>::invoke(input).unwrap_err();
        assert!(matches!(error.kind, AddonErrorKind::UnsupportedHook));
    }

    fn expression_invocation(text: &str) -> HookInvocation {
        let range = TextRange {
            start: 0,
            end: text.len() as u64,
        };
        HookInvocation {
            context: InvocationContext {
                invocation_id: 2,
                subscription_id: EXPRESSION_SUBSCRIPTION_ID.to_owned(),
                document_id: "file:///expression.sk".to_owned(),
                document_revision: 1,
                expansion: None,
                syntax_context: 0,
            },
            target: HookTarget::ParseStage,
            phase: HookPhase::Expression,
            payload: HookPayload::Expression(ExpressionPayload {
                input: text.to_owned(),
                remaining: range,
                span: MappedSpan {
                    virtual_range: range,
                    origins: vec![SourceOrigin {
                        original_range: range,
                        kind: OriginKind::Exact,
                        expansion: None,
                    }],
                },
                expected_types: vec![ExpressionExpectedType {
                    class_name: "java.lang.Object".to_owned(),
                    plural: false,
                }],
                candidate_ends: vec![range.end],
                allow_literals: true,
                allow_expressions: true,
                time: 0,
                depth: 0,
                candidates: Vec::new(),
            }),
        }
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

#![cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
#![doc = include_str!("../README.md")]
#![warn(rustdoc::broken_intra_doc_links)]
#![allow(missing_docs)] // `wit_bindgen` owns the generated guest surface.

mod catalog;
mod conditions;
mod effects;
mod experiments;
mod expression_candidates;
mod expressions;
mod language;
mod loop_context;
mod primitives;
mod runtime;
mod sections;
mod structures;
mod types;

wit_bindgen::generate!({
    path: "../parser-wasm/wit",
    world: "parser-addon",
    generate_unused_types: true,
});

use exports::nlaocs::skript_parser_addon::{addon, ast_macro, hooks, text_macro, tree_macro};
use nlaocs::skript_parser_addon::types::{
    AbiVersion, AddonError, AddonErrorKind, AstMacroInput, AstMacroOutput, CapabilityRequirement,
    CompatibilityError, CompatibilityErrorKind, ComponentManifest, Diagnostic, DiagnosticSeverity,
    ExpressionPayload, HookDecision, HookEffects, HookInvocation, HookMode, HookOutput,
    HookPayload, HookPhase, HookSelector, HookSubscription, HookTarget, HostProfile, ParseResult,
    RegisteredExpressionPayload, StateNamespaceDeclaration, StateNamespaceVisibility, SyntaxKind,
    TextMacroInput, TextMacroOutput, TreeMacroInput, TreeMacroOutput,
};
#[cfg(test)]
use nlaocs::skript_parser_addon::types::{RuntimePlugin, RuntimeProfile};
use parser_wasm::{
    ABI_VERSION, AbiVersion as ParserAbiVersion, CAPABILITY_CATALOG_DATA,
    CAPABILITY_CONDITION_PARSER, CAPABILITY_DYNAMIC_SYNTAX, CAPABILITY_EFFECT_PARSER,
    CAPABILITY_EXPRESSION_PARSER, CAPABILITY_HOOKS, CAPABILITY_SECTION_PARSER,
    CAPABILITY_STATE_STORE, CAPABILITY_STRUCTURE_PARSER, CAPABILITY_TREE_MACRO,
    Capability as ParserCapability, CapabilityRequirement as ParserCapabilityRequirement,
    CompatibilityError as ParserCompatibilityError, validate_compatibility,
};

const COMPONENT_ID: &str = "nlaocs.core-library";
const COMPONENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const HEALTH_CHECK_SUBSCRIPTION_ID: &str = "core.health-check";
const EXPRESSION_SUBSCRIPTION_ID: &str = "core.expression-candidates";
const TYPE_SUBSCRIPTION_ID: &str = "core.type-candidates";
const REGISTERED_EXPRESSION_SUBSCRIPTION_ID: &str = "core.registered-expression-semantics";
const CONDITION_SUBSCRIPTION_ID: &str = "core.condition-semantics";
const EFFECT_SUBSCRIPTION_ID: &str = "core.effect-semantics";
const SECTION_SUBSCRIPTION_ID: &str = "core.section-semantics";
const STRUCTURE_SUBSCRIPTION_ID: &str = "core.structure-semantics";
const OPTIONS_MACRO_SUBSCRIPTION_ID: &str = "core.options-preprocessor";

fn empty_hook_selector() -> HookSelector {
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
                CapabilityRequirement {
                    id: CAPABILITY_EFFECT_PARSER.to_owned(),
                    minimum_version: 1,
                    required: true,
                },
                CapabilityRequirement {
                    id: CAPABILITY_CONDITION_PARSER.to_owned(),
                    minimum_version: 1,
                    required: true,
                },
                CapabilityRequirement {
                    id: CAPABILITY_SECTION_PARSER.to_owned(),
                    minimum_version: 1,
                    required: true,
                },
                CapabilityRequirement {
                    id: CAPABILITY_STRUCTURE_PARSER.to_owned(),
                    minimum_version: 1,
                    required: true,
                },
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
                CapabilityRequirement {
                    id: CAPABILITY_DYNAMIC_SYNTAX.to_owned(),
                    minimum_version: 1,
                    required: false,
                },
                CapabilityRequirement {
                    id: CAPABILITY_CATALOG_DATA.to_owned(),
                    minimum_version: 2,
                    required: false,
                },
            ],
            subscriptions: vec![
                HookSubscription {
                    id: HEALTH_CHECK_SUBSCRIPTION_ID.to_owned(),
                    target: HookTarget::ParseStage,
                    phase: HookPhase::Document,
                    priority: 0,
                    mode: HookMode::Observe,
                    capability_id: CAPABILITY_HOOKS.to_owned(),
                    selector: empty_hook_selector(),
                },
                HookSubscription {
                    id: EXPRESSION_SUBSCRIPTION_ID.to_owned(),
                    target: HookTarget::ParseStage,
                    phase: HookPhase::Expression,
                    priority: 0,
                    mode: HookMode::Transform,
                    capability_id: CAPABILITY_EXPRESSION_PARSER.to_owned(),
                    selector: empty_hook_selector(),
                },
                HookSubscription {
                    id: REGISTERED_EXPRESSION_SUBSCRIPTION_ID.to_owned(),
                    target: HookTarget::SyntaxKind(SyntaxKind::Expression),
                    phase: HookPhase::Expression,
                    priority: 0,
                    mode: HookMode::Transform,
                    capability_id: CAPABILITY_EXPRESSION_PARSER.to_owned(),
                    selector: empty_hook_selector(),
                },
                HookSubscription {
                    id: TYPE_SUBSCRIPTION_ID.to_owned(),
                    target: HookTarget::SyntaxKind(SyntaxKind::Type),
                    phase: HookPhase::Expression,
                    priority: 0,
                    mode: HookMode::Transform,
                    capability_id: CAPABILITY_EXPRESSION_PARSER.to_owned(),
                    selector: empty_hook_selector(),
                },
                HookSubscription {
                    id: CONDITION_SUBSCRIPTION_ID.to_owned(),
                    target: HookTarget::SyntaxKind(SyntaxKind::Condition),
                    phase: HookPhase::Condition,
                    priority: 0,
                    mode: HookMode::Transform,
                    capability_id: CAPABILITY_CONDITION_PARSER.to_owned(),
                    selector: empty_hook_selector(),
                },
                HookSubscription {
                    id: EFFECT_SUBSCRIPTION_ID.to_owned(),
                    target: HookTarget::SyntaxKind(SyntaxKind::Effect),
                    phase: HookPhase::Effect,
                    priority: 0,
                    mode: HookMode::Transform,
                    capability_id: CAPABILITY_EFFECT_PARSER.to_owned(),
                    selector: empty_hook_selector(),
                },
                HookSubscription {
                    id: SECTION_SUBSCRIPTION_ID.to_owned(),
                    target: HookTarget::SyntaxKind(SyntaxKind::Section),
                    phase: HookPhase::Section,
                    priority: 0,
                    mode: HookMode::Transform,
                    capability_id: CAPABILITY_SECTION_PARSER.to_owned(),
                    selector: empty_hook_selector(),
                },
                HookSubscription {
                    id: STRUCTURE_SUBSCRIPTION_ID.to_owned(),
                    target: HookTarget::SyntaxKind(SyntaxKind::Structure),
                    phase: HookPhase::Structure,
                    priority: 0,
                    mode: HookMode::Transform,
                    capability_id: CAPABILITY_STRUCTURE_PARSER.to_owned(),
                    selector: empty_hook_selector(),
                },
                HookSubscription {
                    id: OPTIONS_MACRO_SUBSCRIPTION_ID.to_owned(),
                    target: HookTarget::ParseStage,
                    phase: HookPhase::Tree,
                    priority: i32::MIN,
                    mode: HookMode::Transform,
                    capability_id: CAPABILITY_TREE_MACRO.to_owned(),
                    selector: empty_hook_selector(),
                },
            ],
            registered_syntax_handlers: {
                let mut handlers = expressions::handlers();
                handlers.extend(conditions::handlers());
                handlers.extend(effects::handlers());
                handlers.extend(sections::handlers());
                handlers.extend(structures::handlers());
                handlers.extend(types::handlers());
                handlers
            },
            catalog_annotations: Vec::new(),
            state_namespaces: vec![
                StateNamespaceDeclaration {
                    name: "commands".to_owned(),
                    visibility: StateNamespaceVisibility::Private,
                    schema_id: "nlaocs.core-library.commands".to_owned(),
                    schema_version: 1,
                    readers: Vec::new(),
                    writers: Vec::new(),
                },
                StateNamespaceDeclaration {
                    name: "aliases".to_owned(),
                    visibility: StateNamespaceVisibility::Private,
                    schema_id: "nlaocs.core-library.aliases".to_owned(),
                    schema_version: 1,
                    readers: Vec::new(),
                    writers: Vec::new(),
                },
            ],
        }
    }

    fn initialize(profile: HostProfile) -> Result<(), CompatibilityError> {
        let requirements = [
            ParserCapabilityRequirement::required(CAPABILITY_HOOKS, 1),
            ParserCapabilityRequirement::required(CAPABILITY_EXPRESSION_PARSER, 1),
            ParserCapabilityRequirement::required(CAPABILITY_EFFECT_PARSER, 1),
            ParserCapabilityRequirement::required(CAPABILITY_CONDITION_PARSER, 1),
            ParserCapabilityRequirement::required(CAPABILITY_SECTION_PARSER, 1),
            ParserCapabilityRequirement::required(CAPABILITY_STRUCTURE_PARSER, 1),
            ParserCapabilityRequirement::required(CAPABILITY_TREE_MACRO, 1),
            ParserCapabilityRequirement::required(CAPABILITY_STATE_STORE, 1),
            ParserCapabilityRequirement::optional(CAPABILITY_DYNAMIC_SYNTAX, 1),
            ParserCapabilityRequirement::optional(CAPABILITY_CATALOG_DATA, 2),
        ];
        let capabilities = profile
            .capabilities
            .into_iter()
            .map(|capability| ParserCapability::new(capability.id, capability.version))
            .collect::<Vec<_>>();

        let runtime_profile = profile.runtime.clone();
        let registered_handler_bindings = profile.registered_handler_bindings.clone();
        validate_compatibility(
            ABI_VERSION,
            ParserAbiVersion::new(profile.abi.major, profile.abi.minor),
            &requirements,
            &capabilities,
        )
        .map_err(map_compatibility_error)?;

        let skript_version = require_skript_version(runtime_profile.skript_version.as_deref())?;

        if capabilities
            .iter()
            .any(|capability| capability.id == CAPABILITY_DYNAMIC_SYNTAX && capability.version >= 1)
        {
            structures::register_missing(skript_version).map_err(|message| CompatibilityError {
                kind: CompatibilityErrorKind::InvalidManifest,
                subject: CAPABILITY_DYNAMIC_SYNTAX.to_owned(),
                message,
            })?;
        }
        runtime::replace(runtime_profile, registered_handler_bindings);
        Ok(())
    }
}

fn require_skript_version(version: Option<&str>) -> Result<&str, CompatibilityError> {
    let version = version
        .filter(|version| !version.trim().is_empty())
        .ok_or_else(|| CompatibilityError {
            kind: CompatibilityErrorKind::InvalidManifest,
            subject: "runtime.skript-version".to_owned(),
            message: "CoreLibrary requires the Skript version from the SSG snapshot".to_owned(),
        })?;
    if runtime::parse_skript_version(version).is_none() {
        return Err(CompatibilityError {
            kind: CompatibilityErrorKind::InvalidManifest,
            subject: "runtime.skript-version".to_owned(),
            message: format!("CoreLibrary cannot parse the Skript version `{version}`"),
        });
    }
    Ok(version)
}

impl hooks::Guest for CoreLibrary {
    fn invoke(input: HookInvocation) -> Result<HookOutput, AddonError> {
        match input.context.subscription_id.as_str() {
            HEALTH_CHECK_SUBSCRIPTION_ID => health_check(input),
            EXPRESSION_SUBSCRIPTION_ID => parse_expressions(input),
            TYPE_SUBSCRIPTION_ID => parse_type(input),
            REGISTERED_EXPRESSION_SUBSCRIPTION_ID => parse_registered_expression(input),
            CONDITION_SUBSCRIPTION_ID => parse_condition_semantics(input),
            EFFECT_SUBSCRIPTION_ID => parse_effect_semantics(input),
            SECTION_SUBSCRIPTION_ID => parse_section_semantics(input),
            STRUCTURE_SUBSCRIPTION_ID => parse_structure_semantics(input),
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

fn parse_expressions(input: HookInvocation) -> Result<HookOutput, AddonError> {
    if !matches!(input.target, HookTarget::ParseStage)
        || !matches!(input.phase, HookPhase::Expression)
    {
        return Err(addon_error(
            AddonErrorKind::InvalidPayload,
            "CoreLibrary Expression parser requires an Expression parse-stage payload",
        ));
    }
    match input.payload {
        HookPayload::Expression(payload) => {
            parse_expression_candidates(payload, &input.parse_results)
        }
        _ => Err(addon_error(
            AddonErrorKind::InvalidPayload,
            "CoreLibrary Expression parser requires an Expression payload",
        )),
    }
}

fn parse_registered_expression(input: HookInvocation) -> Result<HookOutput, AddonError> {
    if !matches!(input.target, HookTarget::SyntaxKind(SyntaxKind::Expression))
        || !matches!(input.phase, HookPhase::Expression)
    {
        return Err(addon_error(
            AddonErrorKind::InvalidPayload,
            "CoreLibrary registered Expression semantics require an Expression syntax target",
        ));
    }
    match input.payload {
        HookPayload::RegisteredExpression(payload) => resolve_registered_expression(payload),
        _ => Err(addon_error(
            AddonErrorKind::InvalidPayload,
            "CoreLibrary registered Expression semantics require a registered Expression payload",
        )),
    }
}

fn parse_type(input: HookInvocation) -> Result<HookOutput, AddonError> {
    if !matches!(input.phase, HookPhase::Expression)
        || !matches!(
            input.target,
            HookTarget::SyntaxKind(SyntaxKind::Expression)
                | HookTarget::SyntaxKind(SyntaxKind::Type)
                | HookTarget::Definition(_)
                | HookTarget::Registration(_)
        )
    {
        return Err(addon_error(
            AddonErrorKind::InvalidPayload,
            "CoreLibrary Type parser requires a Type syntax target",
        ));
    }
    let HookPayload::Expression(mut payload) = input.payload else {
        return Err(addon_error(
            AddonErrorKind::InvalidPayload,
            "CoreLibrary Type parser requires an Expression payload",
        ));
    };
    if let Some(candidate) = expression_candidates::parse_types(&payload) {
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

fn parse_condition_semantics(input: HookInvocation) -> Result<HookOutput, AddonError> {
    conditions::parse(input)
}

fn parse_effect_semantics(input: HookInvocation) -> Result<HookOutput, AddonError> {
    effects::parse(input)
}

fn parse_section_semantics(input: HookInvocation) -> Result<HookOutput, AddonError> {
    sections::parse(input)
}

fn parse_structure_semantics(input: HookInvocation) -> Result<HookOutput, AddonError> {
    structures::parse(input)
}

fn parse_expression_candidates(
    mut payload: ExpressionPayload,
    parse_results: &[ParseResult],
) -> Result<HookOutput, AddonError> {
    if let Some(outcome) = primitives::interpolation::parse(&payload, parse_results) {
        return Ok(match outcome {
            primitives::interpolation::Outcome::Requests(parse_requests) => HookOutput {
                decision: HookDecision::ContinueProcessing,
                replacement: None,
                effects: HookEffects {
                    parse_requests,
                    ..empty_effects()
                },
            },
            primitives::interpolation::Outcome::Candidate(candidate, parse_results) => {
                payload.candidates.push(candidate);
                HookOutput {
                    decision: HookDecision::ContinueProcessing,
                    replacement: Some(HookPayload::Expression(payload)),
                    effects: HookEffects {
                        parse_results,
                        ..empty_effects()
                    },
                }
            }
            primitives::interpolation::Outcome::Invalid(diagnostic) => HookOutput {
                decision: HookDecision::ContinueProcessing,
                replacement: None,
                effects: HookEffects {
                    diagnostics: vec![diagnostic],
                    ..empty_effects()
                },
            },
        });
    }
    if let Some(candidate) = expression_candidates::parse(&payload) {
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

fn resolve_registered_expression(
    mut payload: RegisteredExpressionPayload,
) -> Result<HookOutput, AddonError> {
    let Some(resolution) = expressions::resolve(&payload) else {
        return Ok(HookOutput {
            decision: HookDecision::NotApplicable,
            replacement: None,
            effects: empty_effects(),
        });
    };
    let (return_type, possible_return_types, possible_return_types_state, multiplicity, metadata) =
        match resolution {
            expressions::SemanticResolution::Resolved {
                return_type,
                possible_return_types,
                possible_return_types_state,
                multiplicity,
                metadata,
            } => (
                return_type,
                possible_return_types,
                possible_return_types_state,
                multiplicity,
                metadata,
            ),
            expressions::SemanticResolution::Unresolved { reason, metadata } => {
                payload.effective_possible_return_types_state =
                    nlaocs::skript_parser_addon::types::ExpressionPossibleReturnTypesState::Unresolved;
                payload.metadata.extend(metadata);
                payload
                    .metadata
                    .push(nlaocs::skript_parser_addon::types::MetadataEntry {
                        owner_component_id: None,
                        key: "semantic-state".to_owned(),
                        value: "unresolved".to_owned(),
                    });
                let mut effects = empty_effects();
                effects.diagnostics.push(Diagnostic {
                    code: "core.expression.unresolved-semantics".to_owned(),
                    message: reason,
                    severity: DiagnosticSeverity::Warning,
                    span: payload.span.clone(),
                    related: Vec::new(),
                });
                return Ok(HookOutput {
                    decision: HookDecision::ContinueProcessing,
                    replacement: Some(HookPayload::RegisteredExpression(payload)),
                    effects,
                });
            }
            expressions::SemanticResolution::Reject(reason) => {
                let diagnostic = Diagnostic {
                    code: "core.expression.semantic-rejected".to_owned(),
                    message: reason.clone(),
                    severity: DiagnosticSeverity::Error,
                    span: payload.span.clone(),
                    related: Vec::new(),
                };
                return Ok(HookOutput {
                    decision: HookDecision::Reject(nlaocs::skript_parser_addon::types::Rejection {
                        reason,
                        diagnostics: vec![diagnostic],
                    }),
                    replacement: None,
                    effects: empty_effects(),
                });
            }
        };
    payload.effective_return_type = Some(return_type);
    payload.effective_possible_return_types = possible_return_types;
    payload.effective_possible_return_types_state = possible_return_types_state;
    payload.effective_multiplicity = Some(multiplicity);
    payload.metadata.extend(metadata);
    Ok(HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::RegisteredExpression(payload)),
        effects: empty_effects(),
    })
}

#[cfg(test)]
fn metadata(key: &str, value: &str) -> nlaocs::skript_parser_addon::types::MetadataEntry {
    nlaocs::skript_parser_addon::types::MetadataEntry {
        key: key.to_owned(),
        value: value.to_owned(),
        owner_component_id: None,
    }
}

#[cfg(test)]
fn metadata_value<'a>(
    metadata: &'a [nlaocs::skript_parser_addon::types::MetadataEntry],
    key: &str,
) -> Option<&'a str> {
    metadata
        .iter()
        .find(|entry| entry.key == key)
        .map(|entry| entry.value.as_str())
}

impl text_macro::Guest for CoreLibrary {
    fn expand(_input: TextMacroInput) -> Result<TextMacroOutput, AddonError> {
        Err(unsupported_macro("text"))
    }
}

impl tree_macro::Guest for CoreLibrary {
    fn expand(input: TreeMacroInput) -> Result<TreeMacroOutput, AddonError> {
        Ok(structures::expand_options(input))
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
        parse_results: Vec::new(),
    }
}

fn not_applicable() -> HookOutput {
    HookOutput {
        decision: HookDecision::NotApplicable,
        replacement: None,
        effects: empty_effects(),
    }
}

fn reject(reason: &str) -> HookOutput {
    HookOutput {
        decision: HookDecision::Reject(nlaocs::skript_parser_addon::types::Rejection {
            reason: reason.to_owned(),
            diagnostics: Vec::new(),
        }),
        replacement: None,
        effects: empty_effects(),
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
        Capability, DocumentPayload, DynamicMultiplicity, ExpressionExpectedType,
        ExpressionLeafKind, ExpressionLiteralOption, ExpressionLiteralSource,
        ExpressionPossibleReturnTypesState, ExpressionReturnTypeState, ExpressionTypeOption,
        HookPayload, InvocationContext, MappedSpan, OriginKind, ParseResultStatus,
        RegisteredExpressionChild, RegisteredExpressionPropertyOption, RegisteredExpressionTag,
        RegisteredHandlerBinding, SourceOrigin, TextRange,
    };

    #[test]
    fn manifest_exposes_identity_abi_and_health_check() {
        let manifest = <CoreLibrary as addon::Guest>::manifest();

        assert_eq!(manifest.component_id, COMPONENT_ID);
        assert_eq!(manifest.component_version, COMPONENT_VERSION);
        assert_eq!(manifest.abi.major, ABI_VERSION.major);
        assert_eq!(manifest.abi.minor, ABI_VERSION.minor);
        assert_eq!(manifest.capabilities.len(), 10);
        assert_eq!(manifest.capabilities[0].id, CAPABILITY_HOOKS);
        assert!(manifest.capabilities[0].required);
        assert_eq!(manifest.subscriptions.len(), 9);
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
        assert_eq!(manifest.capabilities[2].id, CAPABILITY_EFFECT_PARSER);
        assert_eq!(manifest.subscriptions[3].id, TYPE_SUBSCRIPTION_ID);
        assert!(matches!(
            manifest.subscriptions[3].target,
            HookTarget::SyntaxKind(SyntaxKind::Type)
        ));
        assert_eq!(manifest.capabilities[3].id, CAPABILITY_CONDITION_PARSER);
        assert_eq!(manifest.subscriptions[4].id, CONDITION_SUBSCRIPTION_ID);
        assert_eq!(manifest.subscriptions[5].id, EFFECT_SUBSCRIPTION_ID);
        assert_eq!(manifest.capabilities[4].id, CAPABILITY_SECTION_PARSER);
        assert_eq!(manifest.subscriptions[6].id, SECTION_SUBSCRIPTION_ID);
        assert_eq!(manifest.capabilities[5].id, CAPABILITY_STRUCTURE_PARSER);
        assert_eq!(manifest.subscriptions[7].id, STRUCTURE_SUBSCRIPTION_ID);
        assert_eq!(manifest.capabilities[6].id, CAPABILITY_TREE_MACRO);
        assert!(manifest.capabilities[6].required);
        assert_eq!(manifest.subscriptions[8].id, OPTIONS_MACRO_SUBSCRIPTION_ID);
        assert_eq!(manifest.capabilities[7].id, CAPABILITY_STATE_STORE);
        assert!(manifest.capabilities[7].required);
        assert_eq!(manifest.capabilities[8].id, CAPABILITY_DYNAMIC_SYNTAX);
        assert!(!manifest.capabilities[8].required);
        assert_eq!(manifest.capabilities[9].id, CAPABILITY_CATALOG_DATA);
        assert!(!manifest.capabilities[9].required);
        assert_eq!(manifest.state_namespaces.len(), 2);
        assert_eq!(manifest.state_namespaces[0].name, "commands");
        assert_eq!(manifest.state_namespaces[1].name, "aliases");
        assert_eq!(manifest.registered_syntax_handlers.len(), 120);
        for handler_id in [
            "core.condition.cond-compare",
            "core.condition.prop-cond-contains",
            "core.effect.eff-continue",
            "core.effect.eff-exit",
            "core.effect.eff-sort",
            "core.effect.eff-suppress-type-hints",
            "core.effect.eff-transform",
            "core.expression.expr-event-expression",
            "core.expression.expr-filter",
            "core.expression.expr-input",
            "core.expression.expr-length",
            "core.structure.struct-example",
        ] {
            assert!(
                manifest
                    .registered_syntax_handlers
                    .iter()
                    .any(|handler| handler.handler_id == handler_id),
                "missing {handler_id}"
            );
        }
        let conditional_capture = manifest
            .registered_syntax_handlers
            .iter()
            .find(|handler| handler.handler_id == "core.section.sec-conditional.condition")
            .expect("condition-bearing SecConditional patterns need their own capture binding");
        assert_eq!(conditional_capture.capture_parsers.len(), 1);
        assert!(
            conditional_capture
                .pattern_sources
                .iter()
                .any(|pattern| pattern == "[:parse] if <.+>")
        );
    }

    #[test]
    fn initialization_rejects_incompatible_hosts() {
        let missing = <CoreLibrary as addon::Guest>::initialize(HostProfile {
            abi: AbiVersion {
                major: ABI_VERSION.major,
                minor: ABI_VERSION.minor,
            },
            capabilities: Vec::new(),
            registered_handler_bindings: Vec::new(),
            runtime: RuntimeProfile {
                snapshot_schema_version: None,
                snapshot_id: None,
                server_name: None,
                server_version: None,
                minecraft_version: None,
                java_version: None,
                language: None,
                skript_version: None,
                plugins: Vec::new(),
            },
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
            registered_handler_bindings: Vec::new(),
            runtime: RuntimeProfile {
                snapshot_schema_version: None,
                snapshot_id: None,
                server_name: None,
                server_version: None,
                minecraft_version: None,
                java_version: None,
                language: None,
                skript_version: None,
                plugins: Vec::new(),
            },
        })
        .unwrap_err();
        assert!(matches!(
            wrong_abi.kind,
            CompatibilityErrorKind::AbiVersionMismatch
        ));
    }

    #[test]
    fn core_library_requires_an_explicit_skript_version() {
        let missing = require_skript_version(None).unwrap_err();
        assert!(matches!(
            missing.kind,
            CompatibilityErrorKind::InvalidManifest
        ));
        assert_eq!(missing.subject, "runtime.skript-version");
        assert!(require_skript_version(Some("2.6.4")).is_ok());
        assert!(require_skript_version(Some("2.16.0-pre1")).is_ok());
        assert!(require_skript_version(Some("unknown")).is_err());
    }

    #[test]
    fn initialization_retains_runtime_version_and_plugins() {
        let profile = HostProfile {
            abi: AbiVersion {
                major: ABI_VERSION.major,
                minor: ABI_VERSION.minor,
            },
            capabilities: vec![
                Capability {
                    id: CAPABILITY_HOOKS.to_owned(),
                    version: 1,
                },
                Capability {
                    id: CAPABILITY_EXPRESSION_PARSER.to_owned(),
                    version: 1,
                },
                Capability {
                    id: CAPABILITY_EFFECT_PARSER.to_owned(),
                    version: 1,
                },
                Capability {
                    id: CAPABILITY_CONDITION_PARSER.to_owned(),
                    version: 1,
                },
                Capability {
                    id: CAPABILITY_SECTION_PARSER.to_owned(),
                    version: 1,
                },
                Capability {
                    id: CAPABILITY_STRUCTURE_PARSER.to_owned(),
                    version: 1,
                },
                Capability {
                    id: CAPABILITY_TREE_MACRO.to_owned(),
                    version: 1,
                },
                Capability {
                    id: CAPABILITY_STATE_STORE.to_owned(),
                    version: 1,
                },
                Capability {
                    id: CAPABILITY_DYNAMIC_SYNTAX.to_owned(),
                    version: 1,
                },
            ],
            registered_handler_bindings: vec![RegisteredHandlerBinding {
                handler_id: "core.expression.expr-parse".to_owned(),
                definition_ids: vec!["expression:test".to_owned()],
                registration_ids: vec!["expression:test:0".to_owned()],
            }],
            runtime: RuntimeProfile {
                snapshot_schema_version: Some(4),
                snapshot_id: Some("snapshot-test".to_owned()),
                server_name: Some("Paper".to_owned()),
                server_version: Some("1.21.11".to_owned()),
                minecraft_version: Some("1.21.11".to_owned()),
                java_version: Some("21".to_owned()),
                language: Some("en".to_owned()),
                skript_version: Some("2.15.4".to_owned()),
                plugins: vec![RuntimePlugin {
                    load_order: 7,
                    name: "Skript".to_owned(),
                    version: "2.15.4".to_owned(),
                    main: "ch.njol.skript.Skript".to_owned(),
                }],
            },
        };

        <CoreLibrary as addon::Guest>::initialize(profile).expect("profile must be accepted");

        let retained = runtime::current().expect("accepted profile must be retained");
        assert_eq!(retained.skript_version.as_deref(), Some("2.15.4"));
        assert_eq!(retained.minecraft_version.as_deref(), Some("1.21.11"));
        assert_eq!(retained.plugins.len(), 1);
        assert_eq!(retained.plugins[0].load_order, 7);
        assert_eq!(retained.plugins[0].name, "Skript");
        assert_eq!(retained.plugins[0].main, "ch.njol.skript.Skript");
        assert!(runtime::handler_matches(
            "core.expression.expr-parse",
            "expression:test:0",
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
    fn expression_hook_recognizes_variable_string_and_number_candidates() {
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
            let output = invoke_expression_pipeline(expression_invocation(text))
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
    fn interpolated_string_requests_and_accepts_its_embedded_expression() {
        let mut invocation = expression_invocation("\"value: %42%\"");
        let first = <CoreLibrary as hooks::Guest>::invoke(invocation.clone()).unwrap();
        assert_eq!(first.effects.parse_requests.len(), 1);
        assert!(first.replacement.is_none());

        let request = first.effects.parse_requests[0].clone();
        assert!(
            request
                .options
                .iter()
                .any(|option| { option.key == "parse.mode" && option.value == "expressions-only" })
        );
        invocation.parse_results.push(ParseResult {
            host_token: 7,
            request_id: request.request_id,
            parser_id: request.parser_id,
            status: ParseResultStatus::Success,
            roots: vec![0],
            nodes: Vec::new(),
            diagnostics: Vec::new(),
        });
        let second = <CoreLibrary as hooks::Guest>::invoke(invocation).unwrap();
        let Some(HookPayload::Expression(payload)) = second.replacement else {
            panic!("successful interpolation must produce a leaf");
        };
        assert_eq!(
            payload.candidates[0].parser_id,
            "core.literal.variable-string"
        );
        assert_eq!(payload.candidates[0].children[0].host_token, 7);
        assert_eq!(second.effects.parse_results.len(), 1);
    }

    #[test]
    fn boolean_literals_use_skript_english_spellings() {
        for (text, value) in [("true", "true"), ("yes", "true"), ("off", "false")] {
            let output = invoke_expression_pipeline(expression_invocation(text)).unwrap();
            let Some(HookPayload::Expression(payload)) = output.replacement else {
                panic!("boolean literal must be returned");
            };
            assert_eq!(payload.candidates[0].parser_id, "core.literal.boolean");
            assert_eq!(
                payload.candidates[0].return_type.as_deref(),
                Some("java.lang.Boolean")
            );
            assert_eq!(
                metadata_value(&payload.candidates[0].metadata, "boolean-value"),
                Some(value)
            );
        }
    }

    #[test]
    fn type_parsers_follow_ssg_registration_order_instead_of_core_dispatch_order() {
        for (boolean_order, custom_order, expected_parser) in [
            (20, 10, "core.literal.type"),
            (1, 10, "core.literal.boolean"),
        ] {
            let mut invocation = expression_invocation("true");
            let HookPayload::Expression(payload) = &mut invocation.payload else {
                unreachable!();
            };
            payload.type_options.push(ExpressionTypeOption {
                source_record: None,
                definition_id: "type:boolean".to_owned(),
                registration_id: "type:boolean:0".to_owned(),
                code_name: "boolean".to_owned(),
                class_name: "java.lang.Boolean".to_owned(),
                type_parse_order: boolean_order,
                singular: "boolean".to_owned(),
                plural: "booleans".to_owned(),
                user_input_patterns: vec!["booleans?".to_owned()],
                has_parser: true,
                parse_contexts: vec!["DEFAULT".to_owned()],
                has_supplier: false,
            });
            payload.literal_options.push(ExpressionLiteralOption {
                source_record: None,
                literal_index: None,
                code_name: "custom".to_owned(),
                class_name: "test.Custom".to_owned(),
                type_parse_order: custom_order,
                range: TextRange { start: 0, end: 4 },
                canonical_value: "true".to_owned(),
                source: ExpressionLiteralSource::Supplier,
                plural: false,
                addon_name: "fixture".to_owned(),
                addon_version: "1.0.0".to_owned(),
                parser_class: None,
                parse_contexts: Vec::new(),
                value_class: None,
                represented_class: None,
                variable_name: None,
                debug_text: None,
                enum_constant: None,
                alias_all: None,
                alias_type_count: None,
            });

            let output = invoke_expression_pipeline(invocation).unwrap();
            let Some(HookPayload::Expression(payload)) = output.replacement else {
                panic!("a type literal must be returned");
            };
            assert_eq!(payload.candidates[0].parser_id, expected_parser);
        }
    }

    #[test]
    fn finite_registered_literal_uses_the_earliest_type_parse_order() {
        let mut invocation = expression_invocation("shared");
        let HookPayload::Expression(payload) = &mut invocation.payload else {
            unreachable!();
        };
        for (code_name, class_name, type_parse_order) in
            [("later", "test.Later", 20), ("earlier", "test.Earlier", 10)]
        {
            payload.literal_options.push(ExpressionLiteralOption {
                source_record: None,
                literal_index: None,
                code_name: code_name.to_owned(),
                class_name: class_name.to_owned(),
                type_parse_order,
                range: TextRange { start: 0, end: 6 },
                canonical_value: "shared".to_owned(),
                source: ExpressionLiteralSource::Supplier,
                plural: false,
                addon_name: "fixture".to_owned(),
                addon_version: "1.0.0".to_owned(),
                parser_class: None,
                parse_contexts: Vec::new(),
                value_class: None,
                represented_class: None,
                variable_name: None,
                debug_text: None,
                enum_constant: None,
                alias_all: None,
                alias_type_count: None,
            });
        }

        let output = invoke_expression_pipeline(invocation).unwrap();
        let Some(HookPayload::Expression(payload)) = output.replacement else {
            panic!("finite type literal must be returned");
        };
        assert_eq!(payload.candidates[0].parser_id, "core.literal.type");
        assert_eq!(
            payload.candidates[0].return_type.as_deref(),
            Some("test.Earlier")
        );
        assert_eq!(
            metadata_value(&payload.candidates[0].metadata, "literal-canonical"),
            Some("shared")
        );
        assert_eq!(
            metadata_value(&payload.candidates[0].metadata, "literal-source"),
            Some("supplier")
        );
    }

    #[test]
    fn item_type_literal_accepts_bare_and_amount_prefixed_aliases() {
        for (input, literal_start, amount) in [("stone", 0, None), ("2 stone", 2, Some("2"))] {
            let mut invocation = expression_invocation(input);
            let HookPayload::Expression(payload) = &mut invocation.payload else {
                unreachable!();
            };
            payload.literal_options.push(ExpressionLiteralOption {
                source_record: None,
                literal_index: None,
                code_name: "itemtype".to_owned(),
                class_name: "ch.njol.skript.aliases.ItemType".to_owned(),
                type_parse_order: 10,
                range: TextRange {
                    start: literal_start,
                    end: input.len() as u64,
                },
                canonical_value: "stone".to_owned(),
                source: ExpressionLiteralSource::Alias,
                plural: false,
                addon_name: "Skript".to_owned(),
                addon_version: "2.15.4".to_owned(),
                parser_class: Some(
                    "org.skriptlang.skript.bukkit.base.types.ItemTypeClassInfo$ItemTypeParser"
                        .to_owned(),
                ),
                parse_contexts: vec!["DEFAULT".to_owned()],
                value_class: None,
                represented_class: None,
                variable_name: None,
                debug_text: None,
                enum_constant: None,
                alias_all: Some(false),
                alias_type_count: Some(1),
            });

            let output = invoke_expression_pipeline(invocation).unwrap();
            let Some(HookPayload::Expression(payload)) = output.replacement else {
                panic!("item type alias must be returned");
            };
            let candidate = &payload.candidates[0];
            assert_eq!(candidate.parser_id, "core.literal.item-type");
            assert_eq!(candidate.range.start, 0);
            assert_eq!(candidate.range.end, input.len() as u64);
            assert_eq!(
                candidate.return_type.as_deref(),
                Some("ch.njol.skript.aliases.ItemType")
            );
            assert_eq!(
                metadata_value(&candidate.metadata, "literal-amount"),
                amount
            );
            assert_eq!(
                metadata_value(&candidate.metadata, "literal-canonical"),
                Some("stone")
            );
            let expected_start = literal_start.to_string();
            assert_eq!(
                metadata_value(&candidate.metadata, "literal-range-start"),
                Some(expected_start.as_str())
            );
        }
    }

    #[test]
    fn variable_multiplicity_follows_the_variable_shape() {
        for (text, expected) in [
            ("{value}", DynamicMultiplicity::Single),
            ("{values::*}", DynamicMultiplicity::Multiple),
        ] {
            let mut invocation = expression_invocation(text);
            let HookPayload::Expression(payload) = &mut invocation.payload else {
                unreachable!();
            };
            payload.expected_types[0].plural = true;

            let output = <CoreLibrary as hooks::Guest>::invoke(invocation)
                .expect("CoreLibrary Expression invocation must succeed");
            let Some(HookPayload::Expression(payload)) = output.replacement else {
                panic!("variable must replace the Expression payload");
            };
            assert_eq!(payload.candidates[0].multiplicity, Some(expected));
        }
    }

    #[test]
    fn expression_hook_candidates_leave_unknown_input_untouched() {
        let output = invoke_expression_pipeline(expression_invocation("not a leaf"))
            .expect("unknown input is a normal no-match");
        assert!(output.replacement.is_none());
        assert!(matches!(output.decision, HookDecision::ContinueProcessing));
    }

    #[test]
    fn class_info_literal_carries_the_selected_runtime_type() {
        let mut invocation = expression_invocation("number");
        let HookPayload::Expression(payload) = &mut invocation.payload else {
            unreachable!();
        };
        payload.expected_types[0].class_name = "ch.njol.skript.classes.ClassInfo".to_owned();
        payload.type_options.push(ExpressionTypeOption {
            source_record: None,
            definition_id: "type:number".to_owned(),
            registration_id: "type:number:0".to_owned(),
            code_name: "number".to_owned(),
            class_name: "java.lang.Number".to_owned(),
            type_parse_order: 10,
            singular: "number".to_owned(),
            plural: "numbers".to_owned(),
            user_input_patterns: vec!["number".to_owned(), "numbers".to_owned()],
            has_parser: true,
            parse_contexts: vec!["PARSE".to_owned()],
            has_supplier: true,
        });

        let output = invoke_expression_pipeline(invocation).unwrap();
        let Some(HookPayload::Expression(payload)) = output.replacement else {
            panic!("ClassInfo literal must be returned");
        };
        assert_eq!(
            metadata_value(&payload.candidates[0].metadata, "target-class"),
            Some("java.lang.Number")
        );
        assert_eq!(
            metadata_value(&payload.candidates[0].metadata, "has-supplier"),
            Some("true")
        );
    }

    #[test]
    fn class_info_wins_when_a_type_literal_has_the_same_spelling() {
        let mut invocation = expression_invocation("player");
        let HookPayload::Expression(payload) = &mut invocation.payload else {
            unreachable!();
        };
        payload.expected_types[0].class_name = "ch.njol.skript.classes.ClassInfo".to_owned();
        payload.type_options.push(ExpressionTypeOption {
            source_record: None,
            definition_id: "type:player".to_owned(),
            registration_id: "type:player:0".to_owned(),
            code_name: "player".to_owned(),
            class_name: "org.bukkit.entity.Player".to_owned(),
            type_parse_order: 20,
            singular: "player".to_owned(),
            plural: "players".to_owned(),
            user_input_patterns: vec!["player".to_owned(), "players".to_owned()],
            has_parser: true,
            parse_contexts: vec!["DEFAULT".to_owned(), "COMMAND".to_owned()],
            has_supplier: false,
        });
        payload.type_options.push(ExpressionTypeOption {
            source_record: None,
            definition_id: "type:classinfo".to_owned(),
            registration_id: "type:classinfo:0".to_owned(),
            code_name: "classinfo".to_owned(),
            class_name: "ch.njol.skript.classes.ClassInfo".to_owned(),
            type_parse_order: 117,
            singular: "type".to_owned(),
            plural: "types".to_owned(),
            user_input_patterns: vec!["types?".to_owned()],
            has_parser: true,
            parse_contexts: vec!["PARSE".to_owned()],
            has_supplier: true,
        });
        payload.literal_options.push(ExpressionLiteralOption {
            source_record: None,
            literal_index: None,
            code_name: "classinfo".to_owned(),
            // SSG's literal option describes the ClassInfo value itself. The
            // represented runtime class is carried by type_options.
            class_name: "ch.njol.skript.classes.ClassInfo".to_owned(),
            type_parse_order: 117,
            range: TextRange { start: 0, end: 6 },
            canonical_value: "player".to_owned(),
            source: ExpressionLiteralSource::Supplier,
            plural: false,
            addon_name: "Skript".to_owned(),
            addon_version: "2.15.4".to_owned(),
            parser_class: Some("ch.njol.skript.classes.data.SkriptClasses$2".to_owned()),
            parse_contexts: Vec::new(),
            value_class: Some("org.skriptlang.skript.bukkit.types.PlayerClassInfo".to_owned()),
            represented_class: None,
            variable_name: None,
            debug_text: None,
            enum_constant: None,
            alias_all: None,
            alias_type_count: None,
        });

        let output = invoke_expression_pipeline(invocation).unwrap();
        let Some(HookPayload::Expression(payload)) = output.replacement else {
            panic!("ClassInfo parser must win over a same-spelled value literal");
        };
        assert_eq!(payload.candidates[0].parser_id, "core.literal.class-info");
        assert_eq!(
            metadata_value(&payload.candidates[0].metadata, "target-class"),
            Some("org.bukkit.entity.Player")
        );
    }

    #[test]
    fn player_entity_data_literal_carries_its_runtime_class_and_plurality() {
        let mut invocation = expression_invocation("players");
        let HookPayload::Expression(payload) = &mut invocation.payload else {
            unreachable!();
        };
        payload.expected_types[0].class_name = "ch.njol.skript.entity.EntityData".to_owned();
        payload.literal_options.push(ExpressionLiteralOption {
            source_record: None,
            literal_index: Some(0),
            code_name: "entitydata".to_owned(),
            class_name: "ch.njol.skript.entity.EntityData".to_owned(),
            type_parse_order: 107,
            range: TextRange { start: 0, end: 7 },
            canonical_value: "players".to_owned(),
            source: ExpressionLiteralSource::Supplier,
            plural: true,
            addon_name: "Skript".to_owned(),
            addon_version: "2.15.4".to_owned(),
            parser_class: Some("ch.njol.skript.entity.EntityData$1".to_owned()),
            parse_contexts: Vec::new(),
            value_class: Some("ch.njol.skript.entity.EntityData".to_owned()),
            represented_class: Some("org.bukkit.entity.Player".to_owned()),
            variable_name: None,
            debug_text: Some("players".to_owned()),
            enum_constant: None,
            alias_all: None,
            alias_type_count: None,
        });

        let output = invoke_expression_pipeline(invocation).unwrap();
        let Some(HookPayload::Expression(payload)) = output.replacement else {
            panic!("entity data literal must be returned");
        };
        assert_eq!(payload.candidates[0].parser_id, "core.literal.entity-data");
        assert_eq!(
            metadata_value(&payload.candidates[0].metadata, "entity-class"),
            Some("org.bukkit.entity.Player")
        );
        assert_eq!(
            metadata_value(&payload.candidates[0].metadata, "entity-plural"),
            Some("true")
        );
    }

    #[test]
    fn entity_data_literal_accepts_skript_indefinite_articles() {
        let mut invocation = expression_invocation("a player");
        let HookPayload::Expression(payload) = &mut invocation.payload else {
            unreachable!();
        };
        payload.expected_types[0].class_name = "ch.njol.skript.entity.EntityData".to_owned();
        payload.literal_options.push(ExpressionLiteralOption {
            source_record: None,
            literal_index: Some(0),
            code_name: "entitydata".to_owned(),
            class_name: "ch.njol.skript.entity.EntityData".to_owned(),
            type_parse_order: 107,
            range: TextRange { start: 2, end: 8 },
            canonical_value: "player".to_owned(),
            source: ExpressionLiteralSource::Supplier,
            plural: false,
            addon_name: "Skript".to_owned(),
            addon_version: "2.16.0".to_owned(),
            parser_class: Some("ch.njol.skript.entity.EntityData$1".to_owned()),
            parse_contexts: Vec::new(),
            value_class: Some("ch.njol.skript.entity.EntityData".to_owned()),
            represented_class: Some("org.bukkit.entity.Player".to_owned()),
            variable_name: None,
            debug_text: Some("player".to_owned()),
            enum_constant: None,
            alias_all: None,
            alias_type_count: None,
        });

        let output = invoke_expression_pipeline(invocation).unwrap();
        let Some(HookPayload::Expression(payload)) = output.replacement else {
            panic!("article-prefixed entity data literal must be returned");
        };
        let candidate = &payload.candidates[0];
        assert_eq!(candidate.parser_id, "core.literal.entity-data");
        assert_eq!(candidate.range.start, 0);
        assert_eq!(candidate.range.end, 8);
        assert_eq!(
            metadata_value(&candidate.metadata, "entity-class"),
            Some("org.bukkit.entity.Player")
        );
    }

    #[test]
    fn size_count_expression_resolves_dynamic_metadata() {
        let mut size = registered_expression(
            "org.skriptlang.skript.common.properties.elements.expressions.PropExprSize",
        );
        size.children.push(RegisteredExpressionChild {
            text: "all offline players".to_owned(),
            kind: "registered-expression".to_owned(),
            parser_id: None,
            definition_id: None,
            registration_id: None,
            pattern_index: None,
            element_class: None,
            return_type: Some("org.bukkit.OfflinePlayer".to_owned()),
            possible_return_types: vec!["org.bukkit.OfflinePlayer".to_owned()],
            possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
            multiplicity: Some(DynamicMultiplicity::Multiple),
            metadata: Vec::new(),
        });
        let expressions::SemanticResolution::Resolved {
            return_type,
            multiplicity,
            ..
        } = expressions::resolve(&size).expect("PropExprSize handler must be registered")
        else {
            panic!("list size must resolve");
        };
        assert_eq!(return_type, "java.lang.Long");
        assert_eq!(multiplicity, DynamicMultiplicity::Single);
    }

    #[test]
    fn property_expressions_resolve_registered_types_and_axes() {
        let mut wxyz = registered_expression(
            "org.skriptlang.skript.common.properties.elements.expressions.PropExprWXYZ",
        );
        wxyz.tags.push(RegisteredExpressionTag {
            value: "x".to_owned(),
            implicit: false,
        });
        wxyz.children.push(expression_child(
            "player's location",
            "org.bukkit.Location",
            DynamicMultiplicity::Both,
        ));
        wxyz.property_options.push(property_option(
            "org.bukkit.Location",
            &["java.lang.Double"],
            &["x", "y", "z"],
        ));

        let expressions::SemanticResolution::Resolved {
            return_type,
            multiplicity,
            metadata: resolved_metadata,
            ..
        } = expressions::resolve(&wxyz).expect("PropExprWXYZ handler must be registered")
        else {
            panic!("location x coordinate must resolve");
        };
        assert_eq!(return_type, "java.lang.Double");
        assert_eq!(multiplicity, DynamicMultiplicity::Both);
        assert_eq!(metadata_value(&resolved_metadata, "wxyz-axis"), Some("x"));

        wxyz.tags[0].value = "w".to_owned();
        assert!(matches!(
            expressions::resolve(&wxyz),
            Some(expressions::SemanticResolution::Reject(_))
        ));

        for (class, source_type, return_type) in [
            (
                "PropExprCustomName",
                "org.bukkit.entity.Player",
                "net.kyori.adventure.text.Component",
            ),
            (
                "PropExprName",
                "org.bukkit.entity.Player",
                "net.kyori.adventure.text.Component",
            ),
            (
                "PropExprScale",
                "org.bukkit.entity.Display",
                "org.bukkit.util.Vector",
            ),
        ] {
            let mut property = registered_expression(&format!(
                "org.skriptlang.skript.common.properties.elements.expressions.{class}"
            ));
            property.children.push(expression_child(
                "source",
                source_type,
                DynamicMultiplicity::Single,
            ));
            property
                .property_options
                .push(property_option(source_type, &[return_type], &[]));
            let Some(expressions::SemanticResolution::Resolved {
                return_type: actual,
                multiplicity,
                ..
            }) = expressions::resolve(&property)
            else {
                panic!("{class} must resolve");
            };
            assert_eq!(actual, return_type);
            assert_eq!(multiplicity, DynamicMultiplicity::Single);
        }

        let mut ambiguous = registered_expression(
            "org.skriptlang.skript.common.properties.elements.expressions.PropExprName",
        );
        ambiguous.children.push(expression_child(
            "source",
            "org.bukkit.entity.Player",
            DynamicMultiplicity::Single,
        ));
        ambiguous.property_options.push(property_option(
            "org.bukkit.entity.Player",
            &["java.lang.String"],
            &[],
        ));
        let mut conflicting =
            property_option("org.bukkit.entity.Player", &["java.lang.Number"], &[]);
        conflicting.property_registration_id = "property:other".to_owned();
        conflicting.property_source_index = 1;
        ambiguous.property_options.push(conflicting);
        assert!(matches!(
            expressions::resolve(&ambiguous),
            Some(expressions::SemanticResolution::Reject(reason))
                if reason.contains("multiple Property registrations")
        ));
        ambiguous.selected_property_option_indices = vec![1];
        assert!(matches!(
            expressions::resolve(&ambiguous),
            Some(expressions::SemanticResolution::Resolved { return_type, .. })
                if return_type == "java.lang.Number"
        ));

        let mut mixed_sources = ambiguous.clone();
        mixed_sources.selected_property_option_indices.clear();
        mixed_sources.children.push(expression_child(
            "other source",
            "org.bukkit.entity.Player",
            DynamicMultiplicity::Multiple,
        ));
        mixed_sources.property_options[1].property_registration_id = mixed_sources.property_options
            [0]
        .property_registration_id
        .clone();
        mixed_sources.property_options[1].property_source_index =
            mixed_sources.property_options[0].property_source_index;
        mixed_sources.property_options[1].source_child_index = 1;
        assert!(matches!(
            expressions::resolve(&mixed_sources),
            Some(expressions::SemanticResolution::Reject(reason))
                if reason.contains("different source Expressions")
        ));
    }

    #[test]
    fn amount_and_typed_value_keep_their_java_specific_branches() {
        let mut amount = registered_expression(
            "org.skriptlang.skript.common.properties.elements.expressions.PropExprAmount",
        );
        amount.children.push(expression_child(
            "all players",
            "org.bukkit.entity.Player",
            DynamicMultiplicity::Multiple,
        ));
        let Some(expressions::SemanticResolution::Resolved {
            return_type,
            multiplicity,
            ..
        }) = expressions::resolve(&amount)
        else {
            panic!("singular amount of a list must count its elements");
        };
        assert_eq!(return_type, "java.lang.Long");
        assert_eq!(multiplicity, DynamicMultiplicity::Single);

        let mut number = registered_expression(
            "org.skriptlang.skript.common.properties.elements.expressions.PropExprNumber",
        );
        number.children.push(expression_child(
            "amount holder",
            "ch.njol.skript.lang.util.common.AnyAmount",
            DynamicMultiplicity::Single,
        ));
        number.property_options.push(property_option(
            "ch.njol.skript.lang.util.common.AnyAmount",
            &["java.lang.Number"],
            &[],
        ));
        let Some(expressions::SemanticResolution::Resolved {
            return_type,
            multiplicity,
            ..
        }) = expressions::resolve(&number)
        else {
            panic!("number of a single value must use its property handler");
        };
        assert_eq!(return_type, "java.lang.Number");
        assert_eq!(multiplicity, DynamicMultiplicity::Single);

        let mut size = registered_expression(
            "org.skriptlang.skript.common.properties.elements.expressions.PropExprSize",
        );
        size.tags.push(RegisteredExpressionTag {
            value: "s".to_owned(),
            implicit: false,
        });
        size.children.push(expression_child(
            "amount holders",
            "ch.njol.skript.lang.util.common.AnyAmount",
            DynamicMultiplicity::Multiple,
        ));
        size.property_options.push(property_option(
            "ch.njol.skript.lang.util.common.AnyAmount",
            &["java.lang.Number"],
            &[],
        ));
        let Some(expressions::SemanticResolution::Resolved {
            return_type,
            multiplicity,
            ..
        }) = expressions::resolve(&size)
        else {
            panic!("plural sizes must use their property handlers");
        };
        assert_eq!(return_type, "java.lang.Number");
        assert_eq!(multiplicity, DynamicMultiplicity::Multiple);

        let mut value = registered_expression(
            "org.skriptlang.skript.common.properties.elements.expressions.PropExprValueOf",
        );
        value.children.push(RegisteredExpressionChild {
            text: "number".to_owned(),
            kind: "literal".to_owned(),
            parser_id: Some("core.literal.class-info".to_owned()),
            definition_id: None,
            registration_id: None,
            pattern_index: None,
            element_class: None,
            return_type: Some("ch.njol.skript.classes.ClassInfo".to_owned()),
            possible_return_types: vec!["ch.njol.skript.classes.ClassInfo".to_owned()],
            possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
            multiplicity: Some(DynamicMultiplicity::Single),
            metadata: vec![metadata("target-class", "java.lang.Number")],
        });
        value.children.push(expression_child(
            "{_node}",
            "ch.njol.skript.config.Node",
            DynamicMultiplicity::Single,
        ));
        value.property_options.push(property_option(
            "ch.njol.skript.config.Node",
            &["java.lang.String"],
            &[],
        ));
        let mut competing_value =
            property_option("ch.njol.skript.config.Node", &["java.lang.Object"], &[]);
        competing_value.property_source_index = 1;
        competing_value.property_registration_id = "property:competing-value".to_owned();
        value.property_options.push(competing_value);
        assert!(matches!(
            expressions::resolve(&value),
            Some(expressions::SemanticResolution::Reject(reason))
                if reason.contains("multiple Property registrations")
        ));
        value.selected_property_option_indices = vec![0];
        let Some(expressions::SemanticResolution::Resolved { return_type, .. }) =
            expressions::resolve(&value)
        else {
            panic!("typed value target must determine its return type");
        };
        assert_eq!(return_type, "java.lang.Number");
    }

    #[test]
    fn entities_expression_uses_the_entity_data_runtime_class() {
        let mut entities = registered_expression("ch.njol.skript.expressions.ExprEntities");
        entities.children.push(RegisteredExpressionChild {
            text: "players".to_owned(),
            kind: "literal".to_owned(),
            parser_id: Some("core.literal.entity-data".to_owned()),
            definition_id: None,
            registration_id: None,
            pattern_index: None,
            element_class: None,
            return_type: Some("ch.njol.skript.entity.EntityData".to_owned()),
            possible_return_types: vec!["ch.njol.skript.entity.EntityData".to_owned()],
            possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
            multiplicity: Some(DynamicMultiplicity::Single),
            metadata: vec![
                metadata("literal-represented-class", "org.bukkit.entity.Player"),
                metadata("literal-plural", "true"),
            ],
        });

        let expressions::SemanticResolution::Resolved {
            return_type,
            multiplicity,
            ..
        } = expressions::resolve(&entities).expect("ExprEntities handler must be registered")
        else {
            panic!("plural player entity data must resolve");
        };
        assert_eq!(return_type, "org.bukkit.entity.Player");
        assert_eq!(multiplicity, DynamicMultiplicity::Multiple);
    }

    #[test]
    fn element_expression_preserves_the_source_type_and_selected_amount() {
        let mut element = registered_expression("ch.njol.skript.expressions.ExprElement");
        element.input = "a random element out of all players".to_owned();
        element.pattern = "[a] random element (of|out of) %objects%".to_owned();
        element.span.virtual_range.end = u64::try_from(element.input.len()).unwrap();
        element.children.push(expression_child(
            "all players",
            "org.bukkit.entity.Player",
            DynamicMultiplicity::Multiple,
        ));

        let Some(expressions::SemanticResolution::Resolved {
            return_type,
            multiplicity,
            ..
        }) = expressions::resolve(&element)
        else {
            panic!("ExprElement handler must resolve a typed source");
        };
        assert_eq!(return_type, "org.bukkit.entity.Player");
        assert_eq!(multiplicity, DynamicMultiplicity::Single);

        element.input = "the first 2 elements out of all players".to_owned();
        element.pattern_index = 1;
        element.pattern = "[the] first %integer% elements (of|out of) %objects%".to_owned();
        element.span.virtual_range.end = u64::try_from(element.input.len()).unwrap();
        let Some(expressions::SemanticResolution::Resolved { multiplicity, .. }) =
            expressions::resolve(&element)
        else {
            panic!("plural ExprElement pattern must resolve");
        };
        assert_eq!(multiplicity, DynamicMultiplicity::Multiple);
    }

    #[test]
    fn inventory_slot_multiplicity_follows_the_number_expression() {
        let mut slot = registered_expression("ch.njol.skript.expressions.ExprInventorySlot");
        slot.pattern = "[the] slot[s] %numbers% of %inventory%".to_owned();
        slot.children.push(expression_child(
            "0",
            "java.lang.Long",
            DynamicMultiplicity::Single,
        ));
        slot.children.push(expression_child(
            "player",
            "org.bukkit.inventory.Inventory",
            DynamicMultiplicity::Single,
        ));

        let Some(expressions::SemanticResolution::Resolved {
            return_type,
            multiplicity,
            ..
        }) = expressions::resolve(&slot)
        else {
            panic!("ExprInventorySlot handler must resolve slot numbers");
        };
        assert_eq!(return_type, "ch.njol.skript.util.slot.Slot");
        assert_eq!(multiplicity, DynamicMultiplicity::Single);

        slot.pattern_index = 1;
        slot.pattern = "%inventory%'[s] slot[s] %numbers%".to_owned();
        slot.children.swap(0, 1);
        slot.children[1].multiplicity = Some(DynamicMultiplicity::Multiple);
        let Some(expressions::SemanticResolution::Resolved { multiplicity, .. }) =
            expressions::resolve(&slot)
        else {
            panic!("reversed inventory slot pattern must resolve");
        };
        assert_eq!(multiplicity, DynamicMultiplicity::Multiple);
    }

    #[test]
    fn random_expression_uses_the_conversion_target_as_a_single_value() {
        let mut random = registered_expression("ch.njol.skript.expressions.ExprRandom");
        random.children.push(RegisteredExpressionChild {
            text: "element".to_owned(),
            kind: "literal".to_owned(),
            parser_id: Some("core.literal.class-info".to_owned()),
            definition_id: None,
            registration_id: None,
            pattern_index: None,
            element_class: None,
            return_type: Some("ch.njol.skript.classes.ClassInfo".to_owned()),
            possible_return_types: vec!["ch.njol.skript.classes.ClassInfo".to_owned()],
            possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
            multiplicity: Some(DynamicMultiplicity::Single),
            metadata: vec![metadata("target-class", "java.lang.Object")],
        });
        random.children.push(expression_child(
            "all players",
            "org.bukkit.entity.Player",
            DynamicMultiplicity::Multiple,
        ));

        let Some(expressions::SemanticResolution::Resolved {
            return_type,
            multiplicity,
            metadata,
            ..
        }) = expressions::resolve(&random)
        else {
            panic!("ExprRandom handler must resolve a typed source");
        };
        assert_eq!(return_type, "java.lang.Object");
        assert_eq!(multiplicity, DynamicMultiplicity::Single);
        assert!(
            metadata.iter().any(|entry| {
                entry.key == "selection-class" && entry.value == "java.lang.Object"
            })
        );

        random.children.pop();
        assert!(matches!(
            expressions::resolve(&random),
            Some(expressions::SemanticResolution::Reject(_))
        ));
    }

    #[test]
    fn sets_expression_requires_a_supplied_plural_class_info() {
        let class_info = |input: &str, plural: &str, supplier: &str| RegisteredExpressionChild {
            text: input.to_owned(),
            kind: "literal".to_owned(),
            parser_id: Some("core.literal.class-info".to_owned()),
            definition_id: None,
            registration_id: None,
            pattern_index: None,
            element_class: None,
            return_type: Some("ch.njol.skript.classes.ClassInfo".to_owned()),
            possible_return_types: vec!["ch.njol.skript.classes.ClassInfo".to_owned()],
            possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
            multiplicity: Some(DynamicMultiplicity::Single),
            metadata: vec![
                metadata("target-class", "java.awt.Color"),
                metadata("type-plural", plural),
                metadata("has-supplier", supplier),
            ],
        };

        let mut sets = registered_expression("ch.njol.skript.expressions.ExprSets");
        sets.input = "all colors".to_owned();
        sets.span.virtual_range.end = u64::try_from(sets.input.len()).unwrap();
        sets.children.push(class_info("colors", "true", "true"));
        let Some(expressions::SemanticResolution::Resolved {
            return_type,
            multiplicity,
            ..
        }) = expressions::resolve(&sets)
        else {
            panic!("plural supplied ClassInfo must resolve");
        };
        assert_eq!(return_type, "java.awt.Color");
        assert_eq!(multiplicity, DynamicMultiplicity::Multiple);

        sets.input = "every color".to_owned();
        sets.span.virtual_range.end = u64::try_from(sets.input.len()).unwrap();
        sets.children[0].metadata[1].value = "false".to_owned();
        let Some(expressions::SemanticResolution::Resolved { .. }) = expressions::resolve(&sets)
        else {
            panic!("every singular ClassInfo must resolve");
        };

        for (input, return_type, type_plural, has_supplier, target_class) in [
            (
                "color",
                Some("ch.njol.skript.classes.ClassInfo"),
                "false",
                "true",
                Some("java.awt.Color"),
            ),
            (
                "colors",
                Some("ch.njol.skript.classes.ClassInfo"),
                "true",
                "false",
                Some("java.awt.Color"),
            ),
            (
                "colors",
                Some("ch.njol.skript.classes.ClassInfo"),
                "true",
                "true",
                None,
            ),
            ("colors", None, "true", "true", Some("java.awt.Color")),
        ] {
            let mut invalid = registered_expression("ch.njol.skript.expressions.ExprSets");
            invalid.input = input.to_owned();
            invalid.span.virtual_range.end = u64::try_from(invalid.input.len()).unwrap();
            let mut child_metadata = vec![
                metadata("type-plural", type_plural),
                metadata("has-supplier", has_supplier),
            ];
            if let Some(target_class) = target_class {
                child_metadata.insert(0, metadata("target-class", target_class));
            }
            invalid.children.push(RegisteredExpressionChild {
                text: "color".to_owned(),
                kind: "literal".to_owned(),
                parser_id: Some("core.literal.class-info".to_owned()),
                definition_id: None,
                registration_id: None,
                pattern_index: None,
                element_class: None,
                return_type: return_type.map(str::to_owned),
                possible_return_types: return_type.into_iter().map(str::to_owned).collect(),
                possible_return_types_state: if return_type.is_some() {
                    ExpressionPossibleReturnTypesState::Complete
                } else {
                    ExpressionPossibleReturnTypesState::Unresolved
                },
                multiplicity: Some(DynamicMultiplicity::Single),
                metadata: child_metadata,
            });
            assert!(matches!(
                expressions::resolve(&invalid),
                Some(expressions::SemanticResolution::Reject(_))
            ));
        }
    }

    #[test]
    fn health_check_rejects_unknown_subscriptions() {
        let mut input = health_check_invocation();
        input.context.subscription_id = "unknown".to_owned();

        let error = <CoreLibrary as hooks::Guest>::invoke(input).unwrap_err();
        assert!(matches!(error.kind, AddonErrorKind::UnsupportedHook));
    }

    fn invoke_expression_pipeline(
        mut invocation: HookInvocation,
    ) -> Result<HookOutput, AddonError> {
        let expression = <CoreLibrary as hooks::Guest>::invoke(invocation.clone())?;
        if !expression.effects.parse_requests.is_empty() {
            return Ok(expression);
        }
        let expression_replacement = expression.replacement.clone();
        if let Some(payload) = expression_replacement.clone() {
            invocation.payload = payload;
        }
        invocation.context.subscription_id = TYPE_SUBSCRIPTION_ID.to_owned();
        invocation.target = HookTarget::SyntaxKind(SyntaxKind::Type);
        let typed = <CoreLibrary as hooks::Guest>::invoke(invocation)?;
        if typed.replacement.is_some() {
            Ok(typed)
        } else if expression_replacement.is_some() {
            Ok(expression)
        } else {
            Ok(typed)
        }
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
            parse_results: Vec::new(),
            payload: HookPayload::Expression(ExpressionPayload {
                input: text.to_owned(),
                context: crate::nlaocs::skript_parser_addon::types::ParseContext {
                    syntax_context: 0,
                    event_classes: Vec::new(),
                    values: Vec::new(),
                },
                active_type: None,
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
                type_options: Vec::new(),
                literal_options: Vec::new(),
                candidates: Vec::new(),
            }),
        }
    }

    fn registered_expression(element_class: &str) -> RegisteredExpressionPayload {
        let range = TextRange { start: 0, end: 1 };
        RegisteredExpressionPayload {
            context: crate::nlaocs::skript_parser_addon::types::ParseContext {
                syntax_context: 0,
                event_classes: Vec::new(),
                values: Vec::new(),
            },
            input: "x".to_owned(),
            definition_id: "expression:test".to_owned(),
            registration_id: "expression:test:0".to_owned(),
            element_class: element_class.to_owned(),
            related_property: None,
            pattern_index: 0,
            pattern: "x".to_owned(),
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
            declared_return_type: Some("java.lang.Object".to_owned()),
            declared_multiplicity: Some(DynamicMultiplicity::Both),
            return_type_state: ExpressionReturnTypeState::Dynamic,
            possible_return_types: Vec::new(),
            possible_return_types_state: ExpressionPossibleReturnTypesState::Unresolved,
            time: 0,
            regex_captures: Vec::new(),
            tags: Vec::<RegisteredExpressionTag>::new(),
            mark: 0,
            children: Vec::new(),
            parsed_captures: Vec::new(),
            common_child_return_type: None,
            type_options: Vec::new(),
            property_options: Vec::<RegisteredExpressionPropertyOption>::new(),
            selected_property_option_indices: Vec::new(),
            effective_return_type: Some("java.lang.Object".to_owned()),
            effective_possible_return_types: Vec::new(),
            effective_possible_return_types_state: ExpressionPossibleReturnTypesState::Unresolved,
            effective_multiplicity: Some(DynamicMultiplicity::Both),
            metadata: Vec::new(),
        }
    }

    fn expression_child(
        text: &str,
        return_type: &str,
        multiplicity: DynamicMultiplicity,
    ) -> RegisteredExpressionChild {
        RegisteredExpressionChild {
            text: text.to_owned(),
            kind: "custom".to_owned(),
            parser_id: None,
            definition_id: None,
            registration_id: None,
            pattern_index: None,
            element_class: None,
            return_type: Some(return_type.to_owned()),
            possible_return_types: vec![return_type.to_owned()],
            possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
            multiplicity: Some(multiplicity),
            metadata: Vec::new(),
        }
    }

    fn property_option(
        input_class: &str,
        return_types: &[&str],
        supported_axes: &[&str],
    ) -> RegisteredExpressionPropertyOption {
        RegisteredExpressionPropertyOption {
            source_record: None,
            property_source_index: 0,
            related_type_index: 0,
            source_child_index: 0,
            match_kind: "exact".to_owned(),
            property_registration_id: "property:test".to_owned(),
            property_name: "test".to_owned(),
            property_handler_class: "test.PropertyHandler".to_owned(),
            property_addon_name: "TestAddon".to_owned(),
            property_addon_version: "1.0.0".to_owned(),
            input_class: input_class.to_owned(),
            handler_class: "test.TypePropertyHandler".to_owned(),
            handler_kind: "expression".to_owned(),
            provider_addon_name: Some("TestAddon".to_owned()),
            provider_addon_version: Some("1.0.0".to_owned()),
            type_code_name: "test".to_owned(),
            element_types: Vec::new(),
            return_types: return_types
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            supported_axes: supported_axes
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            accepted_changers: Vec::new(),
            accepted_changers_state: None,
            requires_source_expression_change: None,
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
            parse_results: Vec::new(),
            payload: HookPayload::Document(DocumentPayload {
                text: String::new(),
            }),
        }
    }
}

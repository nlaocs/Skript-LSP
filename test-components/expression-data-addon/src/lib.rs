#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#![allow(missing_docs)]

wit_bindgen::generate!({
    path: "../../parser-wasm/wit",
    world: "parser-addon",
    generate_unused_types: true,
});

use exports::nlaocs::skript_parser_addon::{addon, ast_macro, hooks, text_macro, tree_macro};
#[cfg(not(feature = "addon-b"))]
use nlaocs::skript_parser_addon::state_store;
use nlaocs::skript_parser_addon::types::{
    AbiVersion, AddonError, AddonErrorKind, AstMacroInput, AstMacroOutput, CapabilityRequirement,
    CompatibilityError, CompatibilityErrorKind, ComponentManifest, ExpressionLeafCandidate,
    ExpressionPayload, HookDecision, HookEffects, HookInvocation, HookMode, HookOutput,
    HookPayload, HookPhase, HookSelector, HookSubscription, HookTarget, HostProfile,
    TextMacroInput, TextMacroOutput, TreeMacroInput, TreeMacroOutput,
};
#[cfg(not(feature = "addon-b"))]
use nlaocs::skript_parser_addon::types::{
    ExpressionPossibleReturnTypesState, ExpressionPublicData, ParseRequest, ParseResult,
    ParseResultNode, ParseResultStatus, ParseSummary, Rejection,
};
#[cfg(not(feature = "addon-b"))]
use nlaocs::skript_parser_addon::types::{
    StateEncoding, StateNamespaceDeclaration, StateNamespaceVisibility, StateScope, StateValue,
};
use parser_wasm::{
    ABI_VERSION, AbiVersion as ParserAbiVersion, CAPABILITY_EXPRESSION_PARSER, CAPABILITY_HOOKS,
    Capability as ParserCapability, CapabilityRequirement as ParserCapabilityRequirement,
    validate_compatibility,
};
#[cfg(not(feature = "addon-b"))]
use parser_wasm::{CAPABILITY_ADDITIONAL_PARSE, CAPABILITY_STATE_STORE};

#[cfg(feature = "addon-b")]
const COMPONENT_ID: &str = "test.expression-data-b";
#[cfg(not(feature = "addon-b"))]
const COMPONENT_ID: &str = "test.expression-data-a";

const VARIABLE_SCHEMA_ID: &str = "nlaocs.skript.variable";
const VARIABLE_SCHEMA_VERSION: u32 = 1;
#[cfg(not(feature = "addon-b"))]
const CORE_COMPONENT_ID: &str = "nlaocs.core-library";
const VARIABLE_PARSER_ID: &str = "core.variable";

#[cfg(not(feature = "addon-b"))]
const EDIT_INPUT: &str = "{_balances::*}";
#[cfg(not(feature = "addon-b"))]
const ROLLBACK_INPUT: &str = "{rollback::*}";
#[cfg(not(feature = "addon-b"))]
const INVALID_JSON_INPUT: &str = "{invalid-json::*}";
#[cfg(not(feature = "addon-b"))]
const REPEATED_SCHEMA_INPUT: &str = "{repeated-schema::*}";
#[cfg(not(feature = "addon-b"))]
const SUMMARY_INPUT: &str = "{invalid-summary::*}";
#[cfg(not(feature = "addon-b"))]
const SUMMARY_PARSER_ID: &str = "test.expression-data.summary";

#[cfg(not(feature = "addon-b"))]
const EDIT_SUBSCRIPTION_ID: &str = "expression-data.a.edit";
#[cfg(not(feature = "addon-b"))]
const METADATA_SUBSCRIPTION_ID: &str = "expression-data.a.metadata";
#[cfg(not(feature = "addon-b"))]
const REJECT_SUBSCRIPTION_ID: &str = "expression-data.a.reject";
#[cfg(not(feature = "addon-b"))]
const INVALID_SUBSCRIPTION_ID: &str = "expression-data.a.invalid";
#[cfg(not(feature = "addon-b"))]
const SUMMARY_REQUEST_SUBSCRIPTION_ID: &str = "expression-data.a.summary-request";
#[cfg(not(feature = "addon-b"))]
const SUMMARY_PROVIDER_SUBSCRIPTION_ID: &str = "expression-data.a.summary-provider";
#[cfg(feature = "addon-b")]
const OBSERVE_SUBSCRIPTION_ID: &str = "expression-data.b.observe";

#[cfg(not(feature = "addon-b"))]
const STATE_NAMESPACE: &str = "expression-data-state";
#[cfg(not(feature = "addon-b"))]
const STATE_SCHEMA_ID: &str = "nlaocs.test.expression-data-state";

const EDITED_VARIABLE_JSON: &str =
    r#"{"scope":"global","name":[{"kind":"text","text":"wallet::balances::*"}]}"#;
#[cfg(not(feature = "addon-b"))]
const REJECTED_VARIABLE_JSON: &str =
    r#"{"scope":"global","name":[{"kind":"text","text":"discarded"}]}"#;

struct ExpressionDataAddon;

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

fn expression_subscription(id: &str, priority: i32) -> HookSubscription {
    HookSubscription {
        id: id.to_owned(),
        target: HookTarget::ParseStage,
        phase: HookPhase::Expression,
        priority,
        mode: HookMode::Transform,
        capability_id: CAPABILITY_EXPRESSION_PARSER.to_owned(),
        selector: empty_selector(),
    }
}

#[cfg(not(feature = "addon-b"))]
fn parser_subscription() -> HookSubscription {
    HookSubscription {
        id: SUMMARY_PROVIDER_SUBSCRIPTION_ID.to_owned(),
        target: HookTarget::Parser(SUMMARY_PARSER_ID.to_owned()),
        phase: HookPhase::Parser,
        priority: 0,
        mode: HookMode::Override,
        capability_id: parser_wasm::CAPABILITY_ADDITIONAL_PARSE.to_owned(),
        selector: empty_selector(),
    }
}

fn manifest_capabilities() -> Vec<CapabilityRequirement> {
    let capabilities = vec![
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
    ];
    #[cfg(not(feature = "addon-b"))]
    {
        let mut capabilities = capabilities;
        capabilities.extend([
            CapabilityRequirement {
                id: CAPABILITY_ADDITIONAL_PARSE.to_owned(),
                minimum_version: 1,
                required: true,
            },
            CapabilityRequirement {
                id: CAPABILITY_STATE_STORE.to_owned(),
                minimum_version: 1,
                required: true,
            },
        ]);
        capabilities
    }
    #[cfg(feature = "addon-b")]
    {
        capabilities
    }
}

fn manifest_subscriptions() -> Vec<HookSubscription> {
    #[cfg(feature = "addon-b")]
    {
        vec![expression_subscription(OBSERVE_SUBSCRIPTION_ID, 2)]
    }
    #[cfg(not(feature = "addon-b"))]
    {
        vec![
            expression_subscription(EDIT_SUBSCRIPTION_ID, 0),
            expression_subscription(METADATA_SUBSCRIPTION_ID, 0),
            expression_subscription(REJECT_SUBSCRIPTION_ID, 1),
            expression_subscription(INVALID_SUBSCRIPTION_ID, 1),
            expression_subscription(SUMMARY_REQUEST_SUBSCRIPTION_ID, 1),
            parser_subscription(),
        ]
    }
}

#[cfg(not(feature = "addon-b"))]
fn manifest_state_namespaces() -> Vec<StateNamespaceDeclaration> {
    vec![StateNamespaceDeclaration {
        name: STATE_NAMESPACE.to_owned(),
        visibility: StateNamespaceVisibility::Private,
        schema_id: STATE_SCHEMA_ID.to_owned(),
        schema_version: 1,
        readers: Vec::new(),
        writers: Vec::new(),
    }]
}

#[cfg(feature = "addon-b")]
fn manifest_state_namespaces() -> Vec<nlaocs::skript_parser_addon::types::StateNamespaceDeclaration>
{
    Vec::new()
}

fn parser_requirements() -> Vec<ParserCapabilityRequirement> {
    let requirements = vec![
        ParserCapabilityRequirement::required(CAPABILITY_HOOKS, 1),
        ParserCapabilityRequirement::required(CAPABILITY_EXPRESSION_PARSER, 1),
    ];
    #[cfg(not(feature = "addon-b"))]
    {
        let mut requirements = requirements;
        requirements.extend([
            ParserCapabilityRequirement::required(CAPABILITY_ADDITIONAL_PARSE, 1),
            ParserCapabilityRequirement::required(CAPABILITY_STATE_STORE, 1),
        ]);
        requirements
    }
    #[cfg(feature = "addon-b")]
    {
        requirements
    }
}

impl addon::Guest for ExpressionDataAddon {
    fn manifest() -> ComponentManifest {
        ComponentManifest {
            component_id: COMPONENT_ID.to_owned(),
            component_version: env!("CARGO_PKG_VERSION").to_owned(),
            abi: AbiVersion {
                major: ABI_VERSION.major,
                minor: ABI_VERSION.minor,
            },
            capabilities: manifest_capabilities(),
            subscriptions: manifest_subscriptions(),
            registered_syntax_handlers: Vec::new(),
            catalog_annotations: Vec::new(),
            state_namespaces: manifest_state_namespaces(),
        }
    }

    fn initialize(profile: HostProfile) -> Result<(), CompatibilityError> {
        let capabilities = profile
            .capabilities
            .into_iter()
            .map(|capability| ParserCapability::new(capability.id, capability.version))
            .collect::<Vec<_>>();
        validate_compatibility(
            ABI_VERSION,
            ParserAbiVersion::new(profile.abi.major, profile.abi.minor),
            &parser_requirements(),
            &capabilities,
        )
        .map_err(|error| CompatibilityError {
            kind: CompatibilityErrorKind::InvalidManifest,
            subject: COMPONENT_ID.to_owned(),
            message: error.to_string(),
        })
    }
}

impl hooks::Guest for ExpressionDataAddon {
    fn invoke(input: HookInvocation) -> Result<HookOutput, AddonError> {
        let subscription_id = input.context.subscription_id;
        match input.payload {
            HookPayload::Expression(payload) => invoke_expression(&subscription_id, payload),
            #[cfg(not(feature = "addon-b"))]
            HookPayload::Parser(request) => invoke_parser(&subscription_id, request),
            _ => Err(addon_error(
                "expression-data fixture received an unsupported payload",
            )),
        }
    }
}

#[cfg(feature = "addon-b")]
fn invoke_expression(
    subscription_id: &str,
    mut payload: ExpressionPayload,
) -> Result<HookOutput, AddonError> {
    if subscription_id != OBSERVE_SUBSCRIPTION_ID
        || expression_text(&payload) != Some("{_balances::*}")
    {
        return Ok(noop());
    }
    let Some(candidate) = variable_candidate(&mut payload) else {
        return Ok(noop());
    };
    if candidate.public_data.len() != 1
        || candidate.public_data[0].schema_id != VARIABLE_SCHEMA_ID
        || candidate.public_data[0].schema_version != VARIABLE_SCHEMA_VERSION
        || candidate.public_data[0].json != EDITED_VARIABLE_JSON
        || candidate.return_type.as_deref() != Some("java.lang.Number")
    {
        return Err(addon_error(
            "addon B did not receive addon A's public variable edit",
        ));
    }
    candidate.return_type = Some("java.lang.Long".to_owned());
    Ok(replace(payload))
}

#[cfg(not(feature = "addon-b"))]
fn invoke_expression(
    subscription_id: &str,
    mut payload: ExpressionPayload,
) -> Result<HookOutput, AddonError> {
    match subscription_id {
        EDIT_SUBSCRIPTION_ID => {
            if expression_text(&payload) != Some(EDIT_INPUT) {
                return Ok(noop());
            }
            let Some(candidate) = variable_candidate(&mut payload) else {
                return Ok(noop());
            };
            candidate.return_type = Some("java.lang.Number".to_owned());
            candidate.public_data = vec![public_data(
                VARIABLE_SCHEMA_ID,
                VARIABLE_SCHEMA_VERSION,
                EDITED_VARIABLE_JSON,
            )];
            Ok(replace(payload))
        }
        METADATA_SUBSCRIPTION_ID => {
            if expression_text(&payload) != Some(EDIT_INPUT) {
                return Ok(noop());
            }
            let Some(candidate) = variable_candidate(&mut payload) else {
                return Ok(noop());
            };
            let Some(entry) = candidate.metadata.iter_mut().find(|entry| {
                entry.key == "expression.capability.key-provider"
                    && entry.owner_component_id.as_deref() == Some(CORE_COMPONENT_ID)
            }) else {
                return Ok(noop());
            };
            entry.value = "forged-by-addon-a".to_owned();
            Ok(replace(payload))
        }
        REJECT_SUBSCRIPTION_ID => {
            if expression_text(&payload) != Some(ROLLBACK_INPUT) {
                return Ok(noop());
            }
            let Some(candidate) = variable_candidate(&mut payload) else {
                return Ok(noop());
            };
            record_state("rejected-candidate", "must be rolled back")?;
            candidate.return_type = Some("java.lang.String".to_owned());
            candidate.public_data = vec![public_data(
                VARIABLE_SCHEMA_ID,
                VARIABLE_SCHEMA_VERSION,
                REJECTED_VARIABLE_JSON,
            )];
            Ok(HookOutput {
                decision: HookDecision::Reject(Rejection {
                    reason: "expression-data fixture rejected the candidate".to_owned(),
                    diagnostics: Vec::new(),
                }),
                replacement: Some(HookPayload::Expression(payload)),
                effects: empty_effects(),
            })
        }
        INVALID_SUBSCRIPTION_ID => {
            let invalid_json = expression_text(&payload) == Some(INVALID_JSON_INPUT);
            let repeated_schema = expression_text(&payload) == Some(REPEATED_SCHEMA_INPUT);
            if invalid_json {
                let Some(candidate) = variable_candidate(&mut payload) else {
                    return Ok(noop());
                };
                record_state("invalid-json", "must be rolled back")?;
                candidate.public_data = vec![public_data(VARIABLE_SCHEMA_ID, 1, "[]")];
                return Ok(replace(payload));
            }
            if repeated_schema {
                let Some(candidate) = variable_candidate(&mut payload) else {
                    return Ok(noop());
                };
                record_state("repeated-schema", "must be rolled back")?;
                candidate.public_data = vec![
                    public_data(VARIABLE_SCHEMA_ID, 1, r#"{"first":true}"#),
                    public_data(VARIABLE_SCHEMA_ID, 1, r#"{"second":true}"#),
                ];
                return Ok(replace(payload));
            }
            Ok(noop())
        }
        SUMMARY_REQUEST_SUBSCRIPTION_ID => {
            if expression_text(&payload) != Some(SUMMARY_INPUT) {
                return Ok(noop());
            }
            let mut effects = empty_effects();
            effects.parse_requests.push(ParseRequest {
                request_id: 7,
                parser_id: SUMMARY_PARSER_ID.to_owned(),
                input: "invalid summary".to_owned(),
                expected_types: Vec::new(),
                span: payload.span.clone(),
                options: Vec::new(),
            });
            Ok(replace_with_effects(payload, effects))
        }
        _ => Err(addon_error(format!(
            "unknown expression-data subscription {subscription_id}"
        ))),
    }
}

#[cfg(not(feature = "addon-b"))]
fn invoke_parser(subscription_id: &str, request: ParseRequest) -> Result<HookOutput, AddonError> {
    if subscription_id != SUMMARY_PROVIDER_SUBSCRIPTION_ID {
        return Err(addon_error("unknown expression-data parser subscription"));
    }
    Ok(HookOutput {
        decision: HookDecision::Handled,
        replacement: Some(HookPayload::Parser(request.clone())),
        effects: HookEffects {
            diagnostics: Vec::new(),
            context_updates: Vec::new(),
            parse_requests: Vec::new(),
            parse_results: vec![invalid_summary_result(&request)],
        },
    })
}

#[cfg(not(feature = "addon-b"))]
fn invalid_summary_result(request: &ParseRequest) -> ParseResult {
    ParseResult {
        host_token: 0,
        request_id: request.request_id,
        parser_id: request.parser_id.clone(),
        status: ParseResultStatus::Success,
        roots: vec![0],
        nodes: vec![ParseResultNode {
            node_id: 0,
            parser_id: request.parser_id.clone(),
            kind: "fixture-summary".to_owned(),
            status: ParseResultStatus::Success,
            text: request.input.clone(),
            span: request.span.clone(),
            expected_types: request.expected_types.clone(),
            summary: Some(ParseSummary {
                kind: "fixture-summary".to_owned(),
                definition_id: None,
                registration_id: None,
                element_class: None,
                pattern_index: None,
                return_type: None,
                possible_return_types: Vec::new(),
                possible_return_types_state: ExpressionPossibleReturnTypesState::Complete,
                multiplicity: None,
                public_data: vec![public_data(VARIABLE_SCHEMA_ID, 1, "[]")],
                metadata: Vec::new(),
            }),
            children: Vec::new(),
            attachments: Vec::new(),
            diagnostics: Vec::new(),
            metadata: Vec::new(),
        }],
        diagnostics: Vec::new(),
    }
}

fn variable_candidate(payload: &mut ExpressionPayload) -> Option<&mut ExpressionLeafCandidate> {
    payload
        .candidates
        .iter_mut()
        .find(|candidate| candidate.parser_id == VARIABLE_PARSER_ID)
}

fn expression_text(payload: &ExpressionPayload) -> Option<&str> {
    let start = usize::try_from(payload.remaining.start).ok()?;
    let end = usize::try_from(payload.remaining.end).ok()?;
    payload.input.get(start..end)
}

#[cfg(not(feature = "addon-b"))]
fn public_data(schema_id: &str, schema_version: u32, json: &str) -> ExpressionPublicData {
    ExpressionPublicData {
        schema_id: schema_id.to_owned(),
        schema_version,
        json: json.to_owned(),
    }
}

fn noop() -> HookOutput {
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: None,
        effects: empty_effects(),
    }
}

fn replace(payload: ExpressionPayload) -> HookOutput {
    replace_with_effects(payload, empty_effects())
}

fn replace_with_effects(payload: ExpressionPayload, effects: HookEffects) -> HookOutput {
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Expression(payload)),
        effects,
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

#[cfg(not(feature = "addon-b"))]
fn record_state(key: &str, _value: &str) -> Result<(), AddonError> {
    state_store::put(
        StateScope::Parse,
        StateNamespaceVisibility::Private,
        STATE_NAMESPACE,
        key,
        &StateValue {
            schema_id: STATE_SCHEMA_ID.to_owned(),
            encoding: StateEncoding::Json,
            bytes: b"true".to_vec(),
        },
    )
    .map_err(|error| addon_error(format!("failed to record fixture state: {}", error.message)))
}

fn addon_error(message: impl Into<String>) -> AddonError {
    AddonError {
        kind: AddonErrorKind::InvalidPayload,
        message: message.into(),
        diagnostics: Vec::new(),
    }
}

impl text_macro::Guest for ExpressionDataAddon {
    fn expand(_input: TextMacroInput) -> Result<TextMacroOutput, AddonError> {
        Err(addon_error(
            "expression-data fixture does not implement text macros",
        ))
    }
}

impl tree_macro::Guest for ExpressionDataAddon {
    fn expand(_input: TreeMacroInput) -> Result<TreeMacroOutput, AddonError> {
        Err(addon_error(
            "expression-data fixture does not implement tree macros",
        ))
    }
}

impl ast_macro::Guest for ExpressionDataAddon {
    fn expand(_input: AstMacroInput) -> Result<AstMacroOutput, AddonError> {
        Err(addon_error(
            "expression-data fixture does not implement AST macros",
        ))
    }
}

#[cfg(target_arch = "wasm32")]
export!(ExpressionDataAddon);

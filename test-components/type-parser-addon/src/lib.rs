#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

//! Test Component for an external Type registration parser.
//!
//! The fixture targets Skript's existing Number registration through a
//! `RegisteredSyntaxHandler`. It accepts one word-form number that the
//! standard Number parser deliberately does not recognize.
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
        Diagnostic, DiagnosticSeverity, DynamicMultiplicity, ExpressionLeafCandidate,
        ExpressionLeafKind, ExpressionLeafTiming, ExpressionPayload, HookDecision, HookEffects,
        HookInvocation, HookMode, HookOutput, HookPayload, HookPhase, HookSelector,
        HookSubscription, HookTarget, HostProfile, MetadataEntry, RegisteredSyntaxHandler,
        RegisteredSyntaxHandlerTarget, StateEncoding, StateNamespaceDeclaration,
        StateNamespaceVisibility, StateScope, StateValue, SyntaxKind, TextMacroInput,
        TextMacroOutput, TextRange, TreeMacroInput, TreeMacroOutput, TypeParserOutcome,
        TypeParserUnresolved,
    },
};
use parser_wasm::{
    ABI_VERSION, AbiVersion as ParserAbiVersion, CAPABILITY_EXPRESSION_PARSER,
    CAPABILITY_STATE_STORE, Capability as ParserCapability,
    CapabilityRequirement as ParserCapabilityRequirement, validate_compatibility,
};

const COMPONENT_ID: &str = "test.type-parser-addon";
const SUBSCRIPTION_ID: &str = "type-parser.number";
const DIRECT_NO_MATCH_SUBSCRIPTION_ID: &str = "type-parser.direct-registration-no-match";
const DIRECT_NO_MATCH_REGISTRATION_ID: &str =
    "type:skript:0151e0f7e39258b0337f6fd153d81e9ebfefd51fbdd4724fd25b31a6278495ce";
const DIRECT_NO_MATCH_INPUT: &str = "direct-unmatched-enchantment";
const HANDLER_ID: &str = "test.type-parser-addon.number";
const PARSER_ID: &str = "test.type-parser-addon.number-word";
const NUMBER_PARSER_CLASS: &str = "fixture.NumberParser";
const SPECIAL_INPUT: &str = "forty-two";
const UNRESOLVED_INPUT: &str = "registry-number";
const REQUIRED_PROVIDER: &str = "fixture.number-registry";
const NUMBER_CODE_NAME: &str = "number";
const NUMBER_CLASS: &str = "java.lang.Number";
const BLOCK_DATA_PARSER_CLASS: &str = "fixture.BlockDataParser";
const BLOCK_DATA_INPUT: &str = "fixture:block[axis=x]";
const BLOCK_DATA_PARSER_ID: &str = "test.type-parser-addon.block-data";
const BLOCK_DATA_REQUIRED_PROVIDER: &str = "fixture.block-data-registry";
const LOOT_TABLE_PARSER_CLASS: &str = "fixture.LootTableParser";
const LOOT_TABLE_INPUT: &str = "fixture:loot";
const LOOT_TABLE_PARSER_ID: &str = "test.type-parser-addon.loot-table";
const ENCHANTMENT_TYPE_PARSER_CLASS: &str = "fixture.EnchantmentTypeParser";
const ENCHANTMENT_TYPE_INPUT: &str = "fixture enchantment 5";
const ENCHANTMENT_TYPE_PARSER_ID: &str = "test.type-parser-addon.enchantment-type";
const TYPE_PROVIDER_INPUT: &str = "shared-type-input";
const NATIVE_INVALID_INPUT: &str = "native-invalid-type-input";
const REGISTERED_EXPRESSION_INPUT: &str = "pi";
const TYPE_A_PARSER_ID: &str = "test.type-parser-addon.type-a";
const TYPE_A_STATE_KEY: &str = "type-a";
const TYPE_A_DIAGNOSTIC_CODE: &str = "type-parser-test.type-a";
const TYPE_B_PARSER_ID: &str = "test.type-parser-addon.type-b";
const TYPE_B_STATE_KEY: &str = "type-b";
const TYPE_B_DIAGNOSTIC_CODE: &str = "type-parser-test.type-b";
const INVALID_TYPE_A_STATE_KEY: &str = "type-a-invalid";
const INVALID_TYPE_A_DIAGNOSTIC_CODE: &str = "type-parser-test.type-a-invalid";
const DEFERRED_TYPE_STATE_KEY: &str = "deferred-type";
const DEFERRED_TYPE_DIAGNOSTIC_CODE: &str = "type-parser-test.deferred-type";
const STATE_NAMESPACE: &str = "type-parser-test";
const STATE_SCHEMA: &str = "test.type-parser-addon.state";
const UNRESOLVED_STATE_KEY: &str = "unresolved-number";

struct TypeParserAddon;

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

impl addon::Guest for TypeParserAddon {
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
                    id: CAPABILITY_EXPRESSION_PARSER.to_owned(),
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
                    id: SUBSCRIPTION_ID.to_owned(),
                    target: HookTarget::SyntaxKind(SyntaxKind::Type),
                    phase: HookPhase::Expression,
                    priority: 0,
                    mode: HookMode::Transform,
                    capability_id: CAPABILITY_EXPRESSION_PARSER.to_owned(),
                    selector: empty_selector(),
                },
                HookSubscription {
                    id: DIRECT_NO_MATCH_SUBSCRIPTION_ID.to_owned(),
                    target: HookTarget::Registration(DIRECT_NO_MATCH_REGISTRATION_ID.to_owned()),
                    phase: HookPhase::Expression,
                    priority: 0,
                    mode: HookMode::Transform,
                    capability_id: CAPABILITY_EXPRESSION_PARSER.to_owned(),
                    selector: empty_selector(),
                },
            ],
            registered_syntax_handlers: vec![
                registered_handler(HANDLER_ID, NUMBER_PARSER_CLASS),
                registered_handler("test.type-parser-addon.block-data", BLOCK_DATA_PARSER_CLASS),
                registered_handler("test.type-parser-addon.loot-table", LOOT_TABLE_PARSER_CLASS),
                registered_handler(
                    "test.type-parser-addon.enchantment-type",
                    ENCHANTMENT_TYPE_PARSER_CLASS,
                ),
            ],
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
            &[ParserCapabilityRequirement::required(
                CAPABILITY_EXPRESSION_PARSER,
                1,
            )],
            &capabilities,
        )
        .map_err(|error| CompatibilityError {
            kind: CompatibilityErrorKind::InvalidManifest,
            subject: COMPONENT_ID.to_owned(),
            message: error.to_string(),
        })?;

        for handler_id in [
            HANDLER_ID,
            "test.type-parser-addon.block-data",
            "test.type-parser-addon.loot-table",
            "test.type-parser-addon.enchantment-type",
        ] {
            let binding = profile
                .registered_handler_bindings
                .iter()
                .find(|binding| binding.handler_id == handler_id)
                .ok_or_else(|| CompatibilityError {
                    kind: CompatibilityErrorKind::InvalidManifest,
                    subject: handler_id.to_owned(),
                    message: "Type parser handler binding was not provided by the host".to_owned(),
                })?;
            if binding.definition_ids.is_empty() || binding.registration_ids.is_empty() {
                return Err(CompatibilityError {
                    kind: CompatibilityErrorKind::InvalidManifest,
                    subject: handler_id.to_owned(),
                    message: "Type parser handler did not resolve a Type registration".to_owned(),
                });
            }
        }
        Ok(())
    }
}

impl hooks::Guest for TypeParserAddon {
    fn invoke(input: HookInvocation) -> Result<HookOutput, AddonError> {
        let direct_no_match = input.context.subscription_id == DIRECT_NO_MATCH_SUBSCRIPTION_ID;
        if input.context.subscription_id != SUBSCRIPTION_ID && !direct_no_match {
            return Err(addon_error("unknown Type parser subscription"));
        }
        let HookPayload::Expression(mut payload) = input.payload else {
            return Err(addon_error(
                "Type parser subscription received a non-Expression payload",
            ));
        };

        let Some(active_type) = payload.active_type.as_ref() else {
            return Ok(noop());
        };
        if direct_no_match {
            if active_type.registration_id == DIRECT_NO_MATCH_REGISTRATION_ID
                && matching_range(&payload, DIRECT_NO_MATCH_INPUT).is_some()
            {
                return Ok(no_match(payload));
            }
            return Ok(noop());
        }
        let parser_class = active_type.parser_class.as_deref();
        let (expected, parser_id, canonical) = match parser_class {
            Some(NUMBER_PARSER_CLASS)
                if active_type.code_name == NUMBER_CODE_NAME
                    && active_type.class_name == NUMBER_CLASS =>
            {
                if matching_range(&payload, UNRESOLVED_INPUT).is_some() {
                    record_state(UNRESOLVED_STATE_KEY)?;
                    payload.type_parser_unresolved.push(TypeParserUnresolved {
                        reason: "the fixture registry is not connected".to_owned(),
                        required_provider: Some(REQUIRED_PROVIDER.to_owned()),
                    });
                    payload.type_parser_outcome = Some(TypeParserOutcome::Handled);
                    return Ok(replaced(payload));
                }
                if let Some(range) = matching_range(&payload, TYPE_PROVIDER_INPUT) {
                    return candidate_with_effects(
                        payload,
                        range,
                        TYPE_A_PARSER_ID,
                        DynamicMultiplicity::Single,
                        TYPE_A_STATE_KEY,
                        TYPE_A_DIAGNOSTIC_CODE,
                        "Type A accepted the shared input",
                    );
                }
                if let Some(range) = matching_range(&payload, NATIVE_INVALID_INPUT) {
                    return candidate_with_effects(
                        payload,
                        range,
                        TYPE_A_PARSER_ID,
                        DynamicMultiplicity::Multiple,
                        INVALID_TYPE_A_STATE_KEY,
                        INVALID_TYPE_A_DIAGNOSTIC_CODE,
                        "Type A returned a native-invalid candidate",
                    );
                }
                if let Some(range) = matching_range(&payload, REGISTERED_EXPRESSION_INPUT) {
                    return candidate_with_effects(
                        payload,
                        range,
                        "test.type-parser-addon.deferred-type",
                        DynamicMultiplicity::Single,
                        DEFERRED_TYPE_STATE_KEY,
                        DEFERRED_TYPE_DIAGNOSTIC_CODE,
                        "deferred Type candidate was considered",
                    );
                }
                (SPECIAL_INPUT, PARSER_ID, SPECIAL_INPUT)
            }
            Some(BLOCK_DATA_PARSER_CLASS) => {
                if matching_range(&payload, UNRESOLVED_INPUT).is_some() {
                    payload.type_parser_unresolved.push(TypeParserUnresolved {
                        reason: "the fixture block data registry is not connected".to_owned(),
                        required_provider: Some(BLOCK_DATA_REQUIRED_PROVIDER.to_owned()),
                    });
                    payload.type_parser_outcome = Some(TypeParserOutcome::Handled);
                    return Ok(replaced(payload));
                }
                if let Some(range) = matching_range(&payload, TYPE_PROVIDER_INPUT) {
                    return candidate_with_effects(
                        payload,
                        range,
                        TYPE_B_PARSER_ID,
                        DynamicMultiplicity::Single,
                        TYPE_B_STATE_KEY,
                        TYPE_B_DIAGNOSTIC_CODE,
                        "Type B accepted the shared input",
                    );
                }
                if let Some(range) = matching_range(&payload, NATIVE_INVALID_INPUT) {
                    return candidate_with_effects(
                        payload,
                        range,
                        TYPE_B_PARSER_ID,
                        DynamicMultiplicity::Single,
                        TYPE_B_STATE_KEY,
                        TYPE_B_DIAGNOSTIC_CODE,
                        "Type B accepted the native-invalid input",
                    );
                }
                (BLOCK_DATA_INPUT, BLOCK_DATA_PARSER_ID, "fixture:block")
            }
            Some(LOOT_TABLE_PARSER_CLASS) => {
                (LOOT_TABLE_INPUT, LOOT_TABLE_PARSER_ID, LOOT_TABLE_INPUT)
            }
            Some(ENCHANTMENT_TYPE_PARSER_CLASS) => (
                ENCHANTMENT_TYPE_INPUT,
                ENCHANTMENT_TYPE_PARSER_ID,
                "fixture enchantment",
            ),
            _ => return Ok(noop()),
        };
        let Some(range) = matching_range(&payload, expected) else {
            return Ok(no_match(payload));
        };
        let active_type = payload
            .active_type
            .as_ref()
            .expect("active Type was checked");
        let mut metadata_entries = vec![
            metadata("provider-identity", COMPONENT_ID),
            metadata("canonical-value", canonical),
            metadata("active-type-code-name", &active_type.code_name),
            metadata("active-type-definition-id", &active_type.definition_id),
            metadata("active-type-registration-id", &active_type.registration_id),
            metadata(
                "active-type-parser-class",
                active_type.parser_class.as_deref().unwrap_or("<missing>"),
            ),
        ];
        if parser_class == Some(ENCHANTMENT_TYPE_PARSER_CLASS) {
            metadata_entries.push(metadata("enchantment-level", "5"));
        } else if parser_class == Some(NUMBER_PARSER_CLASS) {
            metadata_entries.push(metadata("special-input", SPECIAL_INPUT));
        }
        payload.type_parser_outcome = Some(TypeParserOutcome::Handled);
        payload.candidates.push(ExpressionLeafCandidate {
            parser_id: parser_id.to_owned(),
            kind: ExpressionLeafKind::Literal,
            timing: ExpressionLeafTiming::AfterRegistered,
            range,
            return_type: Some(active_type.class_name.clone()),
            multiplicity: Some(DynamicMultiplicity::Single),
            children: Vec::new(),
            public_data: Vec::new(),
            metadata: metadata_entries,
        });
        Ok(replaced(payload))
    }
}

fn candidate_with_effects(
    mut payload: ExpressionPayload,
    range: TextRange,
    parser_id: &str,
    multiplicity: DynamicMultiplicity,
    state_key: &str,
    diagnostic_code: &str,
    diagnostic_message: &str,
) -> Result<HookOutput, AddonError> {
    let return_type = payload
        .active_type
        .as_ref()
        .map(|active_type| active_type.class_name.clone())
        .ok_or_else(|| addon_error("effectful Type candidate requires an active Type"))?;
    record_state(state_key)?;
    payload.type_parser_outcome = Some(TypeParserOutcome::Handled);
    payload.candidates.push(ExpressionLeafCandidate {
        parser_id: parser_id.to_owned(),
        kind: ExpressionLeafKind::Literal,
        timing: ExpressionLeafTiming::AfterRegistered,
        range,
        return_type: Some(return_type),
        multiplicity: Some(multiplicity),
        children: Vec::new(),
        public_data: Vec::new(),
        metadata: vec![metadata("candidate-role", parser_id)],
    });
    let mut effects = empty_effects();
    effects.diagnostics.push(Diagnostic {
        code: diagnostic_code.to_owned(),
        message: diagnostic_message.to_owned(),
        severity: DiagnosticSeverity::Warning,
        span: payload.span.clone(),
        related: Vec::new(),
    });
    Ok(HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Expression(payload)),
        effects,
    })
}

impl text_macro::Guest for TypeParserAddon {
    fn expand(_input: TextMacroInput) -> Result<TextMacroOutput, AddonError> {
        Err(addon_error("text macro is unsupported"))
    }
}

impl tree_macro::Guest for TypeParserAddon {
    fn expand(_input: TreeMacroInput) -> Result<TreeMacroOutput, AddonError> {
        Err(addon_error("tree macro is unsupported"))
    }
}

impl ast_macro::Guest for TypeParserAddon {
    fn expand(_input: AstMacroInput) -> Result<AstMacroOutput, AddonError> {
        Err(addon_error("AST macro is unsupported"))
    }
}

fn matching_range(payload: &ExpressionPayload, expected: &str) -> Option<TextRange> {
    let start = usize::try_from(payload.remaining.start).ok()?;
    payload.candidate_ends.iter().rev().find_map(|end| {
        let end_index = usize::try_from(*end).ok()?;
        (payload.input.get(start..end_index) == Some(expected)).then_some(TextRange {
            start: payload.remaining.start,
            end: *end,
        })
    })
}

fn metadata(key: &str, value: &str) -> MetadataEntry {
    MetadataEntry {
        key: key.to_owned(),
        value: value.to_owned(),
        owner_component_id: None,
    }
}

fn registered_handler(handler_id: &str, parser_class: &str) -> RegisteredSyntaxHandler {
    RegisteredSyntaxHandler {
        handler_id: handler_id.to_owned(),
        kind: SyntaxKind::Type,
        phase: crate::nlaocs::skript_parser_addon::types::HookPhase::Expression,
        targets: vec![RegisteredSyntaxHandlerTarget::ParserClass(
            parser_class.to_owned(),
        )],
        pattern_indices: Vec::new(),
        pattern_sources: Vec::new(),
        required_tags: Vec::new(),
        forbidden_tags: Vec::new(),
        marks: Vec::new(),
        capture_parsers: Vec::new(),
        context_requirements: Vec::new(),
    }
}

fn replaced(payload: ExpressionPayload) -> HookOutput {
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::Expression(payload)),
        effects: empty_effects(),
    }
}

fn no_match(mut payload: ExpressionPayload) -> HookOutput {
    payload.type_parser_outcome = Some(TypeParserOutcome::NoMatch);
    replaced(payload)
}

fn noop() -> HookOutput {
    HookOutput {
        decision: HookDecision::NotApplicable,
        replacement: None,
        effects: empty_effects(),
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

fn addon_error(message: impl Into<String>) -> AddonError {
    AddonError {
        kind: AddonErrorKind::InvalidPayload,
        message: message.into(),
        diagnostics: Vec::new(),
    }
}

#[cfg(target_arch = "wasm32")]
export!(TypeParserAddon);

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
    ExpressionLeafCandidate, ExpressionLeafKind, ExpressionPayload, ExpressionTypeOption,
    HookDecision, HookEffects, HookInvocation, HookMode, HookOutput, HookPayload, HookPhase,
    HookSubscription, HookTarget, HostProfile, MetadataEntry, RegisteredExpressionPayload,
    TextMacroInput, TextMacroOutput, TextRange, TreeMacroInput, TreeMacroOutput,
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
            registered_expression_class_suffixes: vec![
                ".PropExprSize".to_owned(),
                ".ExprParse".to_owned(),
                ".ExprEntities".to_owned(),
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
            EXPRESSION_SUBSCRIPTION_ID => parse_expressions(input),
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
        HookPayload::Expression(payload) => parse_expression_leaves(payload),
        HookPayload::RegisteredExpression(payload) => resolve_registered_expression(payload),
        _ => Err(addon_error(
            AddonErrorKind::InvalidPayload,
            "CoreLibrary Expression parser requires an Expression payload",
        )),
    }
}

fn parse_expression_leaves(mut payload: ExpressionPayload) -> Result<HookOutput, AddonError> {
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

fn resolve_registered_expression(
    mut payload: RegisteredExpressionPayload,
) -> Result<HookOutput, AddonError> {
    let resolution = if payload.element_class.ends_with(".PropExprSize") {
        resolve_size_expression(&payload)
    } else if payload.element_class.ends_with(".ExprParse") {
        resolve_parse_expression(&payload)
    } else if payload.element_class.ends_with(".ExprEntities") {
        resolve_entities_expression(&payload)
    } else {
        return Ok(HookOutput {
            decision: HookDecision::ContinueProcessing,
            replacement: None,
            effects: empty_effects(),
        });
    };
    let (return_type, multiplicity, metadata) = match resolution {
        SemanticResolution::Resolved {
            return_type,
            multiplicity,
            metadata,
        } => (return_type, multiplicity, metadata),
        SemanticResolution::Reject(reason) => {
            return Ok(HookOutput {
                decision: HookDecision::Reject(nlaocs::skript_parser_addon::types::Rejection {
                    reason,
                    diagnostics: Vec::new(),
                }),
                replacement: None,
                effects: empty_effects(),
            });
        }
    };
    payload.effective_return_type = Some(return_type);
    payload.effective_multiplicity = Some(multiplicity);
    payload.metadata.extend(metadata);
    Ok(HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::RegisteredExpression(payload)),
        effects: empty_effects(),
    })
}

fn resolve_entities_expression(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let Some(entity_data) = payload.children.first() else {
        return SemanticResolution::Reject(
            "entities Expression requires an entity data literal".to_owned(),
        );
    };
    if metadata_value(&entity_data.metadata, "entity-plural") != Some("true") {
        return SemanticResolution::Reject(
            "entities Expression requires a plural entity data literal".to_owned(),
        );
    }
    let Some(return_type) = metadata_value(&entity_data.metadata, "entity-class") else {
        return SemanticResolution::Reject(
            "entity data literal has no runtime entity class".to_owned(),
        );
    };
    resolved(
        return_type,
        DynamicMultiplicity::Multiple,
        "entities-literal-type",
    )
}

enum SemanticResolution {
    Resolved {
        return_type: String,
        multiplicity: DynamicMultiplicity,
        metadata: Vec<MetadataEntry>,
    },
    Reject(String),
}

fn resolve_size_expression(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    let Some(source) = payload.children.first() else {
        return SemanticResolution::Reject(
            "size Expression requires a source Expression".to_owned(),
        );
    };
    let use_properties = payload
        .tags
        .iter()
        .any(|tag| tag.value == "s" && !tag.implicit)
        || matches!(source.multiplicity, Some(DynamicMultiplicity::Single));
    if !use_properties {
        return resolved("java.lang.Long", DynamicMultiplicity::Single, "size-count");
    }
    if matches!(source.multiplicity, None | Some(DynamicMultiplicity::Both)) {
        return SemanticResolution::Reject(
            "size Expression source multiplicity is unresolved".to_owned(),
        );
    }
    let mut return_types = payload
        .property_options
        .iter()
        .flat_map(|option| option.return_types.iter().cloned())
        .collect::<Vec<_>>();
    return_types.sort();
    return_types.dedup();
    let return_type = match return_types.as_slice() {
        [] => {
            return SemanticResolution::Reject(
                "source type has no registered size property".to_owned(),
            );
        }
        [only] => only.clone(),
        _ => "java.lang.Object".to_owned(),
    };
    SemanticResolution::Resolved {
        return_type,
        multiplicity: source.multiplicity.unwrap_or(DynamicMultiplicity::Multiple),
        metadata: vec![metadata("semantic-mode", "size-property")],
    }
}

fn resolve_parse_expression(payload: &RegisteredExpressionPayload) -> SemanticResolution {
    if let Some(class_info) = payload.children.iter().find_map(|child| {
        let target = metadata_value(&child.metadata, "target-class")?;
        Some((
            target,
            metadata_value(&child.metadata, "has-parser") == Some("true"),
        ))
    }) {
        if class_info.0 == "java.lang.String" {
            return SemanticResolution::Reject("parsing text as text is not supported".to_owned());
        }
        if !class_info.1 {
            return SemanticResolution::Reject("target type has no parser".to_owned());
        }
        return SemanticResolution::Resolved {
            return_type: class_info.0.to_owned(),
            multiplicity: DynamicMultiplicity::Single,
            metadata: vec![metadata("semantic-mode", "parse-type")],
        };
    }
    let Some(pattern) = payload.regex_captures.first().map(|value| unquote(value)) else {
        return SemanticResolution::Reject("parse Expression has no static target".to_owned());
    };
    let placeholders = match parse_pattern_placeholders(&pattern, &payload.type_options) {
        Ok(placeholders) => placeholders,
        Err(reason) => return SemanticResolution::Reject(reason),
    };
    let plural = placeholders.iter().any(|placeholder| placeholder.plural);
    let return_type = match placeholders.as_slice() {
        [only] => only.class_name.clone(),
        _ => "java.lang.Object".to_owned(),
    };
    SemanticResolution::Resolved {
        return_type,
        multiplicity: if placeholders.len() <= 1 && !plural {
            DynamicMultiplicity::Single
        } else {
            DynamicMultiplicity::Multiple
        },
        metadata: vec![metadata("semantic-mode", "parse-pattern")],
    }
}

struct ParsedPlaceholder {
    class_name: String,
    plural: bool,
}

fn parse_pattern_placeholders(
    pattern: &str,
    options: &[ExpressionTypeOption],
) -> Result<Vec<ParsedPlaceholder>, String> {
    let mut placeholders = Vec::new();
    let mut remaining = pattern;
    while let Some(start) = remaining.find('%') {
        remaining = &remaining[start + 1..];
        let Some(end) = remaining.find('%') else {
            return Err("parse pattern has an unclosed type placeholder".to_owned());
        };
        let mut body = &remaining[..end];
        remaining = &remaining[end + 1..];
        body = body.trim_start_matches(['-', '*', '~']);
        if let Some((without_time, _)) = body.split_once('@') {
            body = without_time;
        }
        let alternatives = body.split('/').collect::<Vec<_>>();
        if alternatives.len() != 1 {
            placeholders.push(ParsedPlaceholder {
                class_name: "java.lang.Object".to_owned(),
                plural: alternatives
                    .iter()
                    .any(|name| type_option(name, options).is_some_and(|(_, plural)| plural)),
            });
            continue;
        }
        let (option, plural) = type_option(alternatives[0], options)
            .ok_or_else(|| format!("unknown type in parse pattern: {}", alternatives[0]))?;
        if !option.has_parser {
            return Err(format!("type has no parser: {}", option.code_name));
        }
        placeholders.push(ParsedPlaceholder {
            class_name: option.class_name.clone(),
            plural,
        });
    }
    Ok(placeholders)
}

fn type_option<'a>(
    name: &str,
    options: &'a [ExpressionTypeOption],
) -> Option<(&'a ExpressionTypeOption, bool)> {
    let name = name.trim();
    options.iter().find_map(|option| {
        if name.eq_ignore_ascii_case(&option.plural) {
            Some((option, true))
        } else if name.eq_ignore_ascii_case(&option.code_name)
            || name.eq_ignore_ascii_case(&option.singular)
        {
            Some((option, false))
        } else {
            None
        }
    })
}

fn unquote(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
        .replace("\"\"", "\"")
}

fn resolved(
    return_type: &str,
    multiplicity: DynamicMultiplicity,
    mode: &str,
) -> SemanticResolution {
    SemanticResolution::Resolved {
        return_type: return_type.to_owned(),
        multiplicity,
        metadata: vec![metadata("semantic-mode", mode)],
    }
}

fn metadata(key: &str, value: &str) -> MetadataEntry {
    MetadataEntry {
        key: key.to_owned(),
        value: value.to_owned(),
    }
}

fn metadata_value<'a>(metadata: &'a [MetadataEntry], key: &str) -> Option<&'a str> {
    metadata
        .iter()
        .find(|entry| entry.key == key)
        .map(|entry| entry.value.as_str())
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
            return Some(expression_candidate(
                "core.variable",
                ExpressionLeafKind::Variable,
                payload.remaining.start,
                end,
                payload
                    .expected_types
                    .first()
                    .map_or("java.lang.Object", |expected| expected.class_name.as_str()),
                if is_list_variable(text) {
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
        if payload.allow_literals
            && payload
                .expected_types
                .iter()
                .any(|expected| expected.class_name == "ch.njol.skript.entity.EntityData")
            && matches!(text.to_ascii_lowercase().as_str(), "player" | "players")
        {
            let plural = text.eq_ignore_ascii_case("players");
            let mut candidate = expression_candidate(
                "core.literal.entity-data",
                ExpressionLeafKind::Literal,
                payload.remaining.start,
                end,
                "ch.njol.skript.entity.EntityData",
                DynamicMultiplicity::Single,
            );
            candidate.metadata = vec![
                metadata("entity-class", "org.bukkit.entity.Player"),
                metadata("entity-plural", if plural { "true" } else { "false" }),
            ];
            return Some(candidate);
        }
        if payload.allow_literals
            && let Some((option, plural)) = type_option(text, &payload.type_options)
        {
            let mut candidate = expression_candidate(
                "core.literal.class-info",
                ExpressionLeafKind::Literal,
                payload.remaining.start,
                end,
                payload
                    .expected_types
                    .first()
                    .map_or("ch.njol.skript.classes.ClassInfo", |expected| {
                        expected.class_name.as_str()
                    }),
                DynamicMultiplicity::Single,
            );
            candidate.metadata = vec![
                metadata("target-class", &option.class_name),
                metadata("type-code-name", &option.code_name),
                metadata("type-plural", if plural { "true" } else { "false" }),
                metadata(
                    "has-parser",
                    if option.has_parser { "true" } else { "false" },
                ),
            ];
            return Some(candidate);
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

fn is_list_variable(text: &str) -> bool {
    text[1..text.len() - 1].trim_end().ends_with("::*")
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
        Capability, DocumentPayload, ExpressionExpectedType, ExpressionPossibleReturnTypesState,
        ExpressionReturnTypeState, HookPayload, InvocationContext, MappedSpan, OriginKind,
        RegisteredExpressionChild, RegisteredExpressionPropertyOption, RegisteredExpressionTag,
        SourceOrigin,
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
        assert_eq!(
            manifest.registered_expression_class_suffixes,
            [".PropExprSize", ".ExprParse", ".ExprEntities"]
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
    fn expression_hook_leaves_unknown_input_untouched() {
        let output = <CoreLibrary as hooks::Guest>::invoke(expression_invocation("not a leaf"))
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
            code_name: "number".to_owned(),
            class_name: "java.lang.Number".to_owned(),
            singular: "number".to_owned(),
            plural: "numbers".to_owned(),
            has_parser: true,
        });

        let output = <CoreLibrary as hooks::Guest>::invoke(invocation).unwrap();
        let Some(HookPayload::Expression(payload)) = output.replacement else {
            panic!("ClassInfo literal must be returned");
        };
        assert_eq!(
            metadata_value(&payload.candidates[0].metadata, "target-class"),
            Some("java.lang.Number")
        );
    }

    #[test]
    fn player_entity_data_literal_carries_its_runtime_class_and_plurality() {
        let mut invocation = expression_invocation("players");
        let HookPayload::Expression(payload) = &mut invocation.payload else {
            unreachable!();
        };
        payload.expected_types[0].class_name = "ch.njol.skript.entity.EntityData".to_owned();

        let output = <CoreLibrary as hooks::Guest>::invoke(invocation).unwrap();
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
    fn size_count_and_parse_expression_resolve_dynamic_metadata() {
        let mut size = registered_expression(
            "org.skriptlang.skript.common.properties.elements.expressions.PropExprSize",
        );
        size.children.push(RegisteredExpressionChild {
            text: "all offline players".to_owned(),
            return_type: Some("org.bukkit.OfflinePlayer".to_owned()),
            multiplicity: Some(DynamicMultiplicity::Multiple),
            metadata: Vec::new(),
        });
        let SemanticResolution::Resolved {
            return_type,
            multiplicity,
            ..
        } = resolve_size_expression(&size)
        else {
            panic!("list size must resolve");
        };
        assert_eq!(return_type, "java.lang.Long");
        assert_eq!(multiplicity, DynamicMultiplicity::Single);

        let mut parse = registered_expression("ch.njol.skript.expressions.ExprParse");
        parse.children.push(RegisteredExpressionChild {
            text: "number".to_owned(),
            return_type: Some("ch.njol.skript.classes.ClassInfo".to_owned()),
            multiplicity: Some(DynamicMultiplicity::Single),
            metadata: vec![
                metadata("target-class", "java.lang.Number"),
                metadata("has-parser", "true"),
            ],
        });
        let SemanticResolution::Resolved {
            return_type,
            multiplicity,
            ..
        } = resolve_parse_expression(&parse)
        else {
            panic!("typed parse must resolve");
        };
        assert_eq!(return_type, "java.lang.Number");
        assert_eq!(multiplicity, DynamicMultiplicity::Single);
    }

    #[test]
    fn entities_expression_uses_the_entity_data_runtime_class() {
        let mut entities = registered_expression("ch.njol.skript.expressions.ExprEntities");
        entities.children.push(RegisteredExpressionChild {
            text: "players".to_owned(),
            return_type: Some("ch.njol.skript.entity.EntityData".to_owned()),
            multiplicity: Some(DynamicMultiplicity::Single),
            metadata: vec![
                metadata("entity-class", "org.bukkit.entity.Player"),
                metadata("entity-plural", "true"),
            ],
        });

        let SemanticResolution::Resolved {
            return_type,
            multiplicity,
            ..
        } = resolve_entities_expression(&entities)
        else {
            panic!("plural player entity data must resolve");
        };
        assert_eq!(return_type, "org.bukkit.entity.Player");
        assert_eq!(multiplicity, DynamicMultiplicity::Multiple);
    }

    #[test]
    fn parse_pattern_derives_plural_result_shape() {
        let mut parse = registered_expression("ch.njol.skript.expressions.ExprParse");
        parse.regex_captures.push("\"value: %numbers%\"".to_owned());
        parse.type_options.push(ExpressionTypeOption {
            code_name: "number".to_owned(),
            class_name: "java.lang.Number".to_owned(),
            singular: "number".to_owned(),
            plural: "numbers".to_owned(),
            has_parser: true,
        });
        let SemanticResolution::Resolved {
            return_type,
            multiplicity,
            ..
        } = resolve_parse_expression(&parse)
        else {
            panic!("pattern parse must resolve");
        };
        assert_eq!(return_type, "java.lang.Number");
        assert_eq!(multiplicity, DynamicMultiplicity::Multiple);
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
                type_options: Vec::new(),
                candidates: Vec::new(),
            }),
        }
    }

    fn registered_expression(element_class: &str) -> RegisteredExpressionPayload {
        let range = TextRange { start: 0, end: 1 };
        RegisteredExpressionPayload {
            input: "x".to_owned(),
            definition_id: "expression:test".to_owned(),
            registration_id: "expression:test:0".to_owned(),
            element_class: element_class.to_owned(),
            related_property: None,
            pattern_index: 0,
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
            regex_captures: Vec::new(),
            tags: Vec::<RegisteredExpressionTag>::new(),
            mark: 0,
            children: Vec::new(),
            type_options: Vec::new(),
            property_options: Vec::<RegisteredExpressionPropertyOption>::new(),
            effective_return_type: Some("java.lang.Object".to_owned()),
            effective_multiplicity: Some(DynamicMultiplicity::Both),
            metadata: Vec::new(),
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

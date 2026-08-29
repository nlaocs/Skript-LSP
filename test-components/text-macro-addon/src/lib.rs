#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

//! Test Component covering ordered Text macro expansion over UTF-8 source.
//!
//! Scenarios include replacements, anchored insertions, diagnostics, StateStore
//! changes, rejection, invalid output, resource limits, and guest traps.
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
        ContextUpdate, Diagnostic, DiagnosticSeverity, ExpressionExpectedType, HookDecision,
        HookEffects, HookInvocation, HookMode, HookOutput, HookPhase, HookSelector,
        HookSubscription, HookTarget, HostProfile, MappedSpan, MetadataEntry, OriginKind,
        ParseRequest, Rejection, RelatedSpan, SourceOrigin, StateEncoding,
        StateNamespaceDeclaration, StateNamespaceVisibility, StateScope, StateValue, TextEdit,
        TextMacroInput, TextMacroOutput, TextRange, TreeMacroInput, TreeMacroOutput,
    },
};
use parser_wasm::{
    ABI_VERSION, AbiVersion as ParserAbiVersion, CAPABILITY_STATE_STORE, CAPABILITY_TEXT_MACRO,
    Capability as ParserCapability, CapabilityRequirement as ParserCapabilityRequirement,
    CompatibilityError as ParserCompatibilityError, validate_compatibility,
};

const COMPONENT_ID: &str = "nlaocs.test.text-macro";
const FIRST_SUBSCRIPTION: &str = "text.first";
const SECOND_SUBSCRIPTION: &str = "text.second";
const STATE_NAMESPACE: &str = "macro-state";
const STATE_SCHEMA: &str = "nlaocs.test.text-macro-state";

struct TextMacroAddon;

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

impl addon::Guest for TextMacroAddon {
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
                    id: CAPABILITY_TEXT_MACRO.to_owned(),
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
                macro_subscription(FIRST_SUBSCRIPTION, -10),
                macro_subscription(SECOND_SUBSCRIPTION, 10),
            ],
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
        let requirements = [
            ParserCapabilityRequirement::required(CAPABILITY_TEXT_MACRO, 1),
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

impl text_macro::Guest for TextMacroAddon {
    fn expand(input: TextMacroInput) -> Result<TextMacroOutput, AddonError> {
        record_invocation(&input)?;
        match input.context.subscription_id.as_str() {
            FIRST_SUBSCRIPTION => first_macro(input),
            SECOND_SUBSCRIPTION => second_macro(input),
            other => Err(addon_error(
                AddonErrorKind::UnsupportedHook,
                format!("unknown text macro subscription {other}"),
            )),
        }
    }
}

fn first_macro(input: TextMacroInput) -> Result<TextMacroOutput, AddonError> {
    if input.text.contains("trap") {
        panic!("text macro fixture trap");
    }
    if input.text.contains("reject") {
        let end = input.text.len() as u64;
        return Ok(TextMacroOutput {
            decision: HookDecision::Reject(Rejection {
                reason: "fixture requested text rollback".to_owned(),
                diagnostics: vec![Diagnostic {
                    code: "fixture.unclosed-delimiter".to_owned(),
                    message: "expected a closing delimiter".to_owned(),
                    severity: DiagnosticSeverity::Error,
                    span: untrusted_span(end, end),
                    related: vec![RelatedSpan {
                        message: "opening delimiter is here".to_owned(),
                        span: untrusted_span(0, 1),
                    }],
                }],
            }),
            edits: vec![TextEdit {
                range: TextRange { start: 0, end: 0 },
                replacement: "discarded ".to_owned(),
                anchor: None,
            }],
            effects: empty_effects(),
        });
    }
    if input.text.contains("bad-diagnostic") {
        return Ok(TextMacroOutput {
            decision: HookDecision::ContinueProcessing,
            edits: Vec::new(),
            effects: effects_with_diagnostics(vec![Diagnostic {
                code: "fixture.invalid-span".to_owned(),
                message: "this span splits a UTF-8 character".to_owned(),
                severity: DiagnosticSeverity::Error,
                span: untrusted_span(1, 2),
                related: Vec::new(),
            }]),
        });
    }
    if input.text.contains("bad-request-span") {
        return Ok(TextMacroOutput {
            decision: HookDecision::ContinueProcessing,
            edits: Vec::new(),
            effects: HookEffects {
                diagnostics: Vec::new(),
                context_updates: Vec::new(),
                parse_requests: vec![parse_request(untrusted_span(1, 2))],
                parse_results: Vec::new(),
            },
        });
    }
    if input.text.contains("invalid") {
        return Ok(TextMacroOutput {
            decision: HookDecision::ContinueProcessing,
            edits: vec![TextEdit {
                range: TextRange { start: 1, end: 2 },
                replacement: "invalid".to_owned(),
                anchor: None,
            }],
            effects: empty_effects(),
        });
    }
    if let Some(start) = input.text.find("alpha") {
        let effects = if input.text.contains("late-stop") {
            HookEffects {
                diagnostics: Vec::new(),
                context_updates: vec![ContextUpdate {
                    syntax_context: input.context.syntax_context,
                    key: "discarded-context".to_owned(),
                    value: Some(vec![1]),
                }],
                parse_requests: vec![parse_request(untrusted_span(
                    start as u64,
                    (start + "alpha".len()) as u64,
                ))],
                parse_results: Vec::new(),
            }
        } else {
            empty_effects()
        };
        return Ok(TextMacroOutput {
            decision: HookDecision::ContinueProcessing,
            edits: vec![TextEdit {
                range: TextRange {
                    start: start as u64,
                    end: (start + "alpha".len()) as u64,
                },
                replacement: "stage-one".to_owned(),
                anchor: None,
            }],
            effects,
        });
    }
    if input.text.contains("anchor") {
        return Ok(TextMacroOutput {
            decision: HookDecision::ContinueProcessing,
            edits: vec![TextEdit {
                range: TextRange {
                    start: input.text.len() as u64,
                    end: input.text.len() as u64,
                },
                replacement: " generated".to_owned(),
                anchor: Some(0),
            }],
            effects: empty_effects(),
        });
    }
    Ok(unchanged())
}

fn second_macro(input: TextMacroInput) -> Result<TextMacroOutput, AddonError> {
    let Some(start) = input.text.find("stage-one") else {
        return Ok(unchanged());
    };
    if input.text.contains("late-stop") {
        let end = input.text.len() as u64;
        return Ok(TextMacroOutput {
            decision: HookDecision::Reject(Rejection {
                reason: "fixture rejected after an earlier expansion".to_owned(),
                diagnostics: vec![Diagnostic {
                    code: "fixture.late-rejection".to_owned(),
                    message: "rejected after generated text".to_owned(),
                    severity: DiagnosticSeverity::Error,
                    span: untrusted_span(start as u64, (start + "stage-one".len()) as u64),
                    related: vec![RelatedSpan {
                        message: "generated input ends here".to_owned(),
                        span: untrusted_span(end, end),
                    }],
                }],
            }),
            edits: Vec::new(),
            effects: empty_effects(),
        });
    }
    let mut effects = effects_with_diagnostics(vec![Diagnostic {
        code: "fixture.generated-source".to_owned(),
        message: "diagnostic over a prior macro expansion".to_owned(),
        severity: DiagnosticSeverity::Information,
        span: untrusted_span(start as u64, (start + "stage-one".len()) as u64),
        related: vec![RelatedSpan {
            message: "the generated range ends here".to_owned(),
            span: untrusted_span(
                (start + "stage-one".len()) as u64,
                (start + "stage-one".len()) as u64,
            ),
        }],
    }]);
    effects.parse_requests.push(parse_request(untrusted_span(
        start as u64,
        (start + "stage-one".len()) as u64,
    )));
    Ok(TextMacroOutput {
        decision: HookDecision::ContinueProcessing,
        edits: vec![TextEdit {
            range: TextRange {
                start: start as u64,
                end: (start + "stage-one".len()) as u64,
            },
            replacement: "二段目".to_owned(),
            anchor: None,
        }],
        effects,
    })
}

fn record_invocation(input: &TextMacroInput) -> Result<(), AddonError> {
    state_store::put(
        StateScope::Parse,
        StateNamespaceVisibility::Private,
        STATE_NAMESPACE,
        &input.context.subscription_id,
        &StateValue {
            schema_id: STATE_SCHEMA.to_owned(),
            encoding: StateEncoding::Json,
            bytes: input.text.as_bytes().to_vec(),
        },
    )
    .map_err(|error| {
        addon_error(
            AddonErrorKind::Internal,
            format!("failed to record text macro state: {}", error.message),
        )
    })
}

fn macro_subscription(id: &str, priority: i32) -> HookSubscription {
    HookSubscription {
        id: id.to_owned(),
        target: HookTarget::ParseStage,
        phase: HookPhase::Preprocess,
        priority,
        mode: HookMode::Transform,
        capability_id: CAPABILITY_TEXT_MACRO.to_owned(),
        selector: empty_selector(),
    }
}

fn unchanged() -> TextMacroOutput {
    TextMacroOutput {
        decision: HookDecision::ContinueProcessing,
        edits: Vec::new(),
        effects: empty_effects(),
    }
}

impl hooks::Guest for TextMacroAddon {
    fn invoke(_input: HookInvocation) -> Result<HookOutput, AddonError> {
        Err(unsupported("hook"))
    }
}

impl tree_macro::Guest for TextMacroAddon {
    fn expand(_input: TreeMacroInput) -> Result<TreeMacroOutput, AddonError> {
        Err(unsupported("tree macro"))
    }
}

impl ast_macro::Guest for TextMacroAddon {
    fn expand(_input: AstMacroInput) -> Result<AstMacroOutput, AddonError> {
        Err(unsupported("AST macro"))
    }
}

fn effects_with_diagnostics(diagnostics: Vec<Diagnostic>) -> HookEffects {
    HookEffects {
        diagnostics,
        context_updates: Vec::new(),
        parse_requests: Vec::new(),
        parse_results: Vec::new(),
    }
}

fn empty_effects() -> HookEffects {
    effects_with_diagnostics(Vec::new())
}

fn parse_request(span: MappedSpan) -> ParseRequest {
    ParseRequest {
        request_id: 7,
        parser_id: "host.expression".to_owned(),
        input: "generated input".to_owned(),
        expected_types: vec![ExpressionExpectedType {
            class_name: "java.lang.String".to_owned(),
            plural: false,
        }],
        span,
        options: vec![MetadataEntry {
            key: "fixture".to_owned(),
            value: "text-macro".to_owned(),
            owner_component_id: None,
        }],
    }
}

fn untrusted_span(start: u64, end: u64) -> MappedSpan {
    MappedSpan {
        virtual_range: TextRange { start, end },
        origins: vec![SourceOrigin {
            original_range: TextRange {
                start: 10_000,
                end: 20_000,
            },
            kind: OriginKind::Exact,
            expansion: Some(999),
        }],
    }
}

fn unsupported(kind: &str) -> AddonError {
    addon_error(
        AddonErrorKind::UnsupportedCapability,
        format!("text macro fixture does not register a {kind}"),
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
export!(TextMacroAddon);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_declares_priority_order_and_private_state() {
        let manifest = <TextMacroAddon as addon::Guest>::manifest();
        assert_eq!(manifest.component_id, COMPONENT_ID);
        assert_eq!(manifest.subscriptions.len(), 2);
        assert_eq!(manifest.subscriptions[0].id, FIRST_SUBSCRIPTION);
        assert_eq!(manifest.subscriptions[0].priority, -10);
        assert_eq!(manifest.subscriptions[1].id, SECOND_SUBSCRIPTION);
        assert_eq!(manifest.subscriptions[1].priority, 10);
        assert_eq!(manifest.state_namespaces.len(), 1);
        assert!(matches!(
            manifest.state_namespaces[0].visibility,
            StateNamespaceVisibility::Private
        ));
    }
}

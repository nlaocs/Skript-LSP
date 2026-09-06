//! Independent providers and downstream consumers for the public default ABI.

use crate::nlaocs::skript_parser_addon::{state_store, types::*};
use parser_wasm::{
    CAPABILITY_DEFAULT_EXPRESSION, CAPABILITY_EFFECT_PARSER, CAPABILITY_EXPRESSION_PARSER,
    CAPABILITY_HOOKS, CAPABILITY_STATE_STORE,
};

const NAMESPACE: &str = "default-expression-test";
const STATE_SCHEMA: &str = "test.default-expression.state";
const EXPRESSION_ID: &str = "expression:test.default-expression:0";
const CUSTOM_EXPRESSION_ID: &str = "expression:test.default-expression:custom:0";
const EFFECT_ID: &str = "effect:test.default-expression:0";
const CUSTOM_TYPE_ID: &str = "type:test.default-expression:0";
const ROLLBACK_PATTERN: &str = "fixture rollback %number% [default %number%]";
const FALLBACK_PATTERN: &str = "fixture rollback 42";
const CUSTOM_FALLBACK_PATTERN: &str = "fixture custom rollback 42";

pub(super) fn register(manifest: &mut ComponentManifest) {
    for capability in [
        CAPABILITY_DEFAULT_EXPRESSION,
        CAPABILITY_EFFECT_PARSER,
        CAPABILITY_STATE_STORE,
    ] {
        if !manifest
            .capabilities
            .iter()
            .any(|entry| entry.id == capability)
        {
            manifest.capabilities.push(CapabilityRequirement {
                id: capability.to_owned(),
                minimum_version: 1,
                required: true,
            });
        }
    }
    manifest.state_namespaces.push(StateNamespaceDeclaration {
        name: NAMESPACE.to_owned(),
        visibility: StateNamespaceVisibility::Private,
        schema_id: STATE_SCHEMA.to_owned(),
        schema_version: 1,
        readers: Vec::new(),
        writers: Vec::new(),
    });
    manifest.subscriptions.push(HookSubscription {
        id: "default-test.observe.explicit-child".to_owned(),
        target: HookTarget::SyntaxKind(SyntaxKind::Type),
        phase: HookPhase::Expression,
        priority: 50,
        mode: HookMode::Transform,
        capability_id: CAPABILITY_EXPRESSION_PARSER.to_owned(),
        selector: crate::empty_selector(),
    });
    if cfg!(feature = "addon-b") {
        // A deliberately has no Matching subscription, so it also exercises
        // transaction scopes when no component observes matching callbacks.
        manifest.subscriptions.push(HookSubscription {
            id: "default-test.observe.pattern-rollback".to_owned(),
            target: HookTarget::Registration(EXPRESSION_ID.to_owned()),
            phase: HookPhase::Matching,
            priority: 50,
            mode: HookMode::Transform,
            capability_id: CAPABILITY_HOOKS.to_owned(),
            selector: crate::empty_selector(),
        });
        manifest
            .subscriptions
            .push(provider("default-test.b.enrich", 10, "java.lang.Number"));
        manifest
            .registered_syntax_handlers
            .push(RegisteredSyntaxHandler {
                handler_id: "default-test.b.consume".to_owned(),
                kind: SyntaxKind::Expression,
                phase: HookPhase::Expression,
                targets: [EXPRESSION_ID, CUSTOM_EXPRESSION_ID]
                    .into_iter()
                    .map(|id| RegisteredSyntaxHandlerTarget::Registration(id.to_owned()))
                    .collect(),
                pattern_indices: Vec::new(),
                pattern_sources: Vec::new(),
                required_tags: Vec::new(),
                forbidden_tags: Vec::new(),
                marks: Vec::new(),
                capture_parsers: Vec::new(),
                context_requirements: Vec::new(),
            });
        for (id, phase, capability) in [
            (
                EXPRESSION_ID,
                HookPhase::Expression,
                CAPABILITY_EXPRESSION_PARSER,
            ),
            (
                CUSTOM_EXPRESSION_ID,
                HookPhase::Expression,
                CAPABILITY_EXPRESSION_PARSER,
            ),
            (EFFECT_ID, HookPhase::Effect, CAPABILITY_EFFECT_PARSER),
        ] {
            manifest.subscriptions.push(HookSubscription {
                id: format!("default-test.b.consume.{id}"),
                target: HookTarget::Registration(id.to_owned()),
                phase,
                priority: 50,
                mode: HookMode::Transform,
                capability_id: capability.to_owned(),
                selector: crate::empty_selector(),
            });
        }
    } else {
        // Intentionally declare the later-priority hook first. Equal priorities
        // below must still retain declaration order.
        manifest.subscriptions.extend([
            provider("default-test.a.late", 10, "java.lang.Number"),
            provider("default-test.a.seed", -20, "java.lang.Number"),
            provider("default-test.a.same-priority", 10, "java.lang.Number"),
            provider("default-test.a.filtered", -100, "fixture.Unrelated"),
        ]);
        let mut custom = provider("default-test.a.custom", 0, "fixture.DefaultValue");
        custom.target = HookTarget::Registration(CUSTOM_TYPE_ID.to_owned());
        let mut enabled = metadata("enabled", "true");
        enabled.owner_component_id = Some(crate::COMPONENT_ID.to_owned());
        custom.selector.metadata.push(enabled);
        manifest.subscriptions.push(custom);
        manifest.catalog_annotations.push(CatalogAnnotation {
            target: CatalogAnnotationTarget::Registration(CUSTOM_TYPE_ID.to_owned()),
            metadata: vec![metadata("enabled", "true")],
        });
    }
}

fn provider(id: &str, priority: i32, class_name: &str) -> HookSubscription {
    let mut selector = crate::empty_selector();
    selector.return_type = Some(ReturnTypeSelector {
        class_name: class_name.to_owned(),
        relation: SelectorTypeRelation::Exact,
    });
    HookSubscription {
        id: id.to_owned(),
        target: HookTarget::SyntaxKind(SyntaxKind::Type),
        phase: HookPhase::DefaultExpression,
        priority,
        mode: HookMode::Transform,
        capability_id: CAPABILITY_DEFAULT_EXPRESSION.to_owned(),
        selector,
    }
}

pub(super) fn invoke(invocation: HookInvocation) -> Result<HookOutput, AddonError> {
    let id = invocation.context.subscription_id;
    match invocation.payload {
        HookPayload::DefaultExpression(payload) => provide(&id, payload),
        HookPayload::Expression(mut payload) => {
            if (payload.input == FALLBACK_PATTERN || payload.input == CUSTOM_FALLBACK_PATTERN)
                && crate::expression_text(&payload) == Some("42")
                && let Some(candidate) = payload.candidates.first_mut()
            {
                record_state("explicit-child")?;
                candidate
                    .metadata
                    .push(metadata("explicit-child", "recorded"));
                return Ok(replace(HookPayload::Expression(payload)));
            }
            Ok(crate::noop())
        }
        HookPayload::Matching(payload) => observe_pattern_rollback(payload),
        HookPayload::RegisteredExpression(mut payload) => {
            let count = payload
                .children
                .iter()
                .filter(|child| child.default_expression.is_some())
                .count();
            for child in &payload.children {
                if let Some(info) = &child.default_expression {
                    verify_info(info, &child.text)?;
                }
            }
            payload
                .metadata
                .push(metadata("default-child-count", &count.to_string()));
            Ok(replace(HookPayload::RegisteredExpression(payload)))
        }
        HookPayload::Effect(mut payload) => {
            let Some(candidate) = &mut payload.candidate else {
                return Ok(crate::noop());
            };
            let mut count = 0;
            for capture in &candidate.parsed_captures {
                if let Some(info) = capture
                    .summary
                    .as_ref()
                    .and_then(|summary| summary.default_expression.as_ref())
                {
                    verify_info(info, &capture.text)?;
                    count += 1;
                }
            }
            candidate
                .metadata
                .push(metadata("default-child-count", &count.to_string()));
            Ok(replace(HookPayload::Effect(payload)))
        }
        _ => Err(crate::addon_error(
            "default test received an unexpected payload",
        )),
    }
}

fn observe_pattern_rollback(payload: MatchingPayload) -> Result<HookOutput, AddonError> {
    if payload.input != FALLBACK_PATTERN || payload.scope != MatchingScope::Pattern {
        return Ok(crate::noop());
    }
    if payload.pattern.as_deref() == Some(ROLLBACK_PATTERN)
        && payload.timing == MatchingTiming::After
        && payload.status == MatchingStatus::Matched
    {
        if !has_state("explicit-child")? || !has_state("default-test.b.enrich")? {
            return Err(crate::addon_error(
                "Pattern After must observe both explicit child and completed default state",
            ));
        }
        return Ok(HookOutput {
            decision: HookDecision::Reject(Rejection {
                reason: "fixture rejects the pattern after both children were selected".to_owned(),
                diagnostics: Vec::new(),
            }),
            replacement: None,
            effects: crate::empty_effects(),
        });
    }
    if payload.pattern.as_deref() == Some(FALLBACK_PATTERN)
        && payload.timing == MatchingTiming::Before
    {
        if has_state("explicit-child")? || has_state("default-test.b.enrich")? {
            return Err(crate::addon_error(
                "next Pattern Before inherited rejected child state",
            ));
        }
        record_state("next-pattern-clean")?;
    }
    Ok(crate::noop())
}

fn verify_info(info: &DefaultExpressionInfo, source: &str) -> Result<(), AddonError> {
    if !source.is_empty()
        || info.span.virtual_range.start != info.span.virtual_range.end
        || info.span.origins.is_empty()
        || info
            .span
            .origins
            .iter()
            .any(|origin| !matches!(origin.kind, OriginKind::Exact | OriginKind::Anchored))
        || info.provider_id.is_empty()
        || info.component_id.is_empty()
    {
        return Err(crate::addon_error(
            "implicit child lost its provider identity or zero-width source anchor",
        ));
    }
    Ok(())
}

fn provide(id: &str, mut payload: DefaultExpressionPayload) -> Result<HookOutput, AddonError> {
    if ![EXPRESSION_ID, CUSTOM_EXPRESSION_ID, EFFECT_ID].contains(&payload.registration_id.as_str())
    {
        return Ok(crate::noop());
    }
    if id == "default-test.a.filtered" {
        panic!("host called a provider excluded by its exact Type selector");
    }
    let mode = payload
        .context
        .values
        .iter()
        .find(|entry| entry.key == "fixture.default-mode")
        .map_or("resolve", |entry| entry.value.as_str())
        .to_owned();
    record_state(id)?;
    if id == "default-test.a.late" || id == "default-test.a.custom" {
        match mode.as_str() {
            "reject" | "reject-second"
                if mode == "reject"
                    || id == "default-test.a.late" && payload.capture_index == 1 =>
            {
                return Ok(HookOutput {
                    decision: HookDecision::Reject(Rejection {
                        reason: "fixture context does not supply the default value".to_owned(),
                        diagnostics: vec![Diagnostic {
                            code: "fixture.default.context".to_owned(),
                            message: "fixture context does not supply the default value".to_owned(),
                            severity: DiagnosticSeverity::Error,
                            span: payload.span,
                            related: Vec::new(),
                        }],
                    }),
                    replacement: None,
                    effects: crate::empty_effects(),
                });
            }
            "trap" | "trap-second"
                if mode == "trap" || id == "default-test.a.late" && payload.capture_index == 1 =>
            {
                panic!("fixture default provider trap after an earlier provider resolved");
            }
            // The host's per-call fuel budget bounds this guest interruption.
            "exhaust-fuel" => loop {
                std::hint::black_box(0usize);
            },
            "invalid" => payload.capture_index += 1,
            "unresolved" => {
                payload.outcome = DefaultExpressionOutcome::Unresolved(
                    "fixture runtime value is unavailable".to_owned(),
                );
                return Ok(replace(HookPayload::DefaultExpression(payload)));
            }
            _ => {}
        }
    }
    if id == "default-test.a.seed"
        || id == "default-test.a.custom"
        || matches!(payload.outcome, DefaultExpressionOutcome::Unresolved(_))
            && id == "default-test.b.enrich"
    {
        let return_type = match mode.as_str() {
            "wrong-type" => "java.lang.String",
            "unknown-type" => "missing.DefaultExpressionType",
            _ => &payload.expected_type.class_name,
        };
        payload.metadata.push(metadata("order", "seed"));
        payload.outcome = DefaultExpressionOutcome::Resolved(DefaultExpressionResolution {
            provider_id: id.to_owned(),
            component_id: None,
            return_type: return_type.to_owned(),
            multiplicity: match mode.as_str() {
                "multiple" => DynamicMultiplicity::Multiple,
                "both" => DynamicMultiplicity::Both,
                _ => DynamicMultiplicity::Single,
            },
            is_literal: mode == "literal",
            time: if mode == "invalid-time" {
                2
            } else {
                payload.time
            },
            reason: "test provider supplies the omitted value from its parse context".to_owned(),
            catalog_references: vec![DefaultExpressionCatalogReference {
                role: "type".to_owned(),
                definition_id: Some(payload.active_type.definition_id.clone()),
                registration_id: Some(payload.active_type.registration_id.clone()),
                source_digest: None,
                snapshot_id: None,
                document: None,
                index: None,
            }],
            public_data: vec![ExpressionPublicData {
                schema_id: "test.default-expression.evidence".to_owned(),
                schema_version: 1,
                json: r#"{"provided":true}"#.to_owned(),
            }],
        });
        if mode == "forged-reference"
            && let DefaultExpressionOutcome::Resolved(resolution) = &mut payload.outcome
        {
            let reference = &mut resolution.catalog_references[0];
            reference.source_digest = Some("forged-digest".to_owned());
            reference.snapshot_id = Some("forged-snapshot".to_owned());
            reference.document = Some("Types.json".to_owned());
            reference.index = Some(0);
        }
    } else if let DefaultExpressionOutcome::Resolved(resolution) = &mut payload.outcome {
        if id == "default-test.b.enrich" {
            let order = payload
                .metadata
                .iter()
                .find(|entry| entry.key == "order")
                .map_or("missing", |entry| entry.value.as_str())
                .to_owned();
            payload.metadata.push(metadata("observed-order", &order));
            payload.metadata.push(metadata(
                "observed-component",
                resolution.component_id.as_deref().unwrap_or("missing"),
            ));
            resolution.public_data.push(ExpressionPublicData {
                schema_id: "test.default-expression.observation".to_owned(),
                schema_version: 1,
                json: r#"{"observedImplicit":true}"#.to_owned(),
            });
            if mode == "spoof-owner" || mode == "replace-owner" {
                resolution.return_type = "java.lang.Long".to_owned();
                resolution.reason = "addon B supplies a more specific default".to_owned();
                if mode == "replace-owner" {
                    resolution.component_id = None;
                    resolution.provider_id = "default-test.b.replacement".to_owned();
                }
            }
        } else if let Some(order) = payload
            .metadata
            .iter_mut()
            .find(|entry| entry.key == "order")
        {
            order.value.push_str(if id == "default-test.a.late" {
                ">late"
            } else {
                ">same"
            });
        }
    }
    Ok(replace(HookPayload::DefaultExpression(payload)))
}

fn metadata(key: &str, value: &str) -> MetadataEntry {
    MetadataEntry {
        owner_component_id: None,
        key: key.to_owned(),
        value: value.to_owned(),
    }
}

fn record_state(key: &str) -> Result<(), AddonError> {
    state_store::put(
        StateScope::Document,
        StateNamespaceVisibility::Private,
        NAMESPACE,
        key,
        &StateValue {
            schema_id: STATE_SCHEMA.to_owned(),
            encoding: StateEncoding::Raw,
            bytes: b"called".to_vec(),
        },
    )
    .map_err(|error| crate::addon_error(error.message))
}

fn has_state(key: &str) -> Result<bool, AddonError> {
    state_store::get(
        StateScope::Document,
        StateNamespaceVisibility::Private,
        NAMESPACE,
        key,
    )
    .map(|value| value.is_some())
    .map_err(|error| crate::addon_error(error.message))
}

fn replace(payload: HookPayload) -> HookOutput {
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(payload),
        effects: crate::empty_effects(),
    }
}

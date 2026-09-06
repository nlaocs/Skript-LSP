//! Standard Skript Type defaults.
//!
//! The SSG descriptor identifies the registered implementation and its static
//! shape. CoreLibrary supplies the context-sensitive behavior for Skript's
//! `SimpleLiteral`, `EventValueExpression`, and `ExprDamageCause` defaults.

#[cfg(test)]
use crate::catalog::CatalogRecordReference;
use crate::catalog::EventValueOption;
use crate::expressions::event_value_expression::preferred_event_value_matches;
use crate::nlaocs::skript_parser_addon::types::*;

pub(crate) const PROVIDER_ID: &str = "core.default-expression.skript";
const SIMPLE_LITERAL_CLASS: &str = "ch.njol.skript.lang.util.SimpleLiteral";
const EVENT_VALUE_CLASS: &str = "ch.njol.skript.expressions.base.EventValueExpression";
const DAMAGE_CAUSE_CLASS: &str = "ch.njol.skript.expressions.ExprDamageCause";

pub(crate) fn subscription() -> HookSubscription {
    HookSubscription {
        id: PROVIDER_ID.to_owned(),
        target: HookTarget::SyntaxKind(SyntaxKind::Type),
        phase: HookPhase::DefaultExpression,
        // Addon and parser-local providers must be able to override Skript's
        // ClassInfo default before this generic fallback runs.
        priority: i32::MAX,
        mode: HookMode::Transform,
        capability_id: parser_wasm::CAPABILITY_DEFAULT_EXPRESSION.to_owned(),
        selector: crate::empty_hook_selector(),
    }
}

pub(crate) fn invoke(input: HookInvocation) -> HookOutput {
    let HookPayload::DefaultExpression(mut payload) = input.payload else {
        return crate::not_applicable();
    };
    if payload.active_type.default_expression.is_none() {
        return crate::not_applicable();
    }
    // An earlier, more specific provider may already have supplied this default.
    if matches!(payload.outcome, DefaultExpressionOutcome::Resolved(_)) {
        return crate::not_applicable();
    }
    let result = resolve(&payload);
    let mut effects = crate::empty_effects();
    payload.outcome = match result {
        Ok(resolution) => {
            effects.diagnostics = resolution
                .diagnostics
                .into_iter()
                .map(|message| Diagnostic {
                    code: "core.default-expression.event-value-excluded".to_owned(),
                    message,
                    severity: DiagnosticSeverity::Error,
                    span: payload.span.clone(),
                    related: Vec::new(),
                })
                .collect();
            payload.metadata.push(MetadataEntry {
                key: "default-expression-class".to_owned(),
                value: payload
                    .active_type
                    .default_expression
                    .as_ref()
                    .unwrap()
                    .implementation_class
                    .clone(),
                owner_component_id: None,
            });
            DefaultExpressionOutcome::Resolved(resolution.expression)
        }
        Err(DefaultFailure::Rejected(reason)) => return crate::reject(&reason),
        Err(DefaultFailure::Unresolved(reason)) => DefaultExpressionOutcome::Unresolved(reason),
    };
    HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: Some(HookPayload::DefaultExpression(payload)),
        effects,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum DefaultFailure {
    Rejected(String),
    Unresolved(String),
}

struct ResolvedDefault {
    expression: DefaultExpressionResolution,
    diagnostics: Vec<String>,
}

fn resolve(payload: &DefaultExpressionPayload) -> Result<ResolvedDefault, DefaultFailure> {
    resolve_with_catalog(payload, crate::catalog::event_values_for)
}

fn resolve_with_catalog(
    payload: &DefaultExpressionPayload,
    mut event_values_for: impl FnMut(&str) -> Result<Vec<EventValueOption>, String>,
) -> Result<ResolvedDefault, DefaultFailure> {
    let descriptor = payload
        .active_type
        .default_expression
        .as_ref()
        .ok_or_else(|| {
            DefaultFailure::Unresolved("Type has no DefaultExpression descriptor".to_owned())
        })?;
    match descriptor.implementation_class.as_str() {
        SIMPLE_LITERAL_CLASS => resolve_literal(payload, descriptor),
        EVENT_VALUE_CLASS => resolve_event_value(
            payload,
            descriptor,
            payload.time,
            payload.time,
            true,
            &mut event_values_for,
        ),
        DAMAGE_CAUSE_CLASS => {
            if payload.time > 0 {
                return Err(DefaultFailure::Rejected(
                    "damage cause DefaultExpression cannot refer to a future event value"
                        .to_owned(),
                ));
            }
            // ExprDamageCause accepts a past request without changing its
            // internal time, so init still resolves the present event value.
            resolve_event_value(
                payload,
                descriptor,
                0,
                payload.time,
                false,
                &mut event_values_for,
            )
        }
        implementation => Err(DefaultFailure::Unresolved(format!(
            "CoreLibrary does not implement Skript DefaultExpression {implementation}"
        ))),
    }
}

fn resolve_literal(
    payload: &DefaultExpressionPayload,
    descriptor: &TypeDefaultExpression,
) -> Result<ResolvedDefault, DefaultFailure> {
    let (return_type, multiplicity) = static_shape(payload, descriptor, true)?;
    if !payload.allow_literals {
        return Err(DefaultFailure::Rejected(format!(
            "{} defaults to a literal, but this capture only allows Expressions",
            payload.active_type.code_name
        )));
    }
    if payload.time != 0 {
        return Err(DefaultFailure::Rejected(format!(
            "{} has a literal DefaultExpression without time states",
            payload.active_type.code_name
        )));
    }
    Ok(ResolvedDefault {
        diagnostics: Vec::new(),
        expression: DefaultExpressionResolution {
            provider_id: PROVIDER_ID.to_owned(),
            component_id: None,
            return_type,
            multiplicity,
            is_literal: true,
            time: 0,
            reason: format!(
                "{} uses Skript's registered SimpleLiteral default",
                payload.active_type.code_name
            ),
            catalog_references: vec![type_reference(payload)],
            public_data: Vec::new(),
        },
    })
}

fn resolve_event_value(
    payload: &DefaultExpressionPayload,
    descriptor: &TypeDefaultExpression,
    effective_time: i32,
    accepted_time: i32,
    require_distinct_time_state: bool,
    event_values_for: &mut impl FnMut(&str) -> Result<Vec<EventValueOption>, String>,
) -> Result<ResolvedDefault, DefaultFailure> {
    let (target_class, multiplicity) = static_shape(payload, descriptor, false)?;
    if !payload.allow_expressions {
        return Err(DefaultFailure::Rejected(format!(
            "{} has a nonliteral DefaultExpression, but this capture only allows literals",
            payload.active_type.code_name
        )));
    }
    if payload.context.event_classes.is_empty() {
        return Err(DefaultFailure::Rejected(format!(
            "omitted {} requires an Event providing {}; no Event context is active",
            payload.active_type.code_name, target_class
        )));
    }
    let mut selected = Vec::new();
    let mut unknown = false;
    let mut has_time_state = effective_time == 0 || !require_distinct_time_state;
    let mut unknown_time_state = false;
    let mut exclusions = Vec::new();
    for event in &payload.context.event_classes {
        let values = event_values_for(event).map_err(DefaultFailure::Unresolved)?;
        if effective_time != 0 && !has_time_state {
            // DefaultExpressionUtils invokes setTime before init. Unlike init,
            // that check cannot fall back to a present-only EventValue.
            let distinct_values = values
                .iter()
                .filter(|value| value.time != 0)
                .cloned()
                .collect::<Vec<_>>();
            for time in [-1, 1] {
                let matches = preferred_event_value_matches(
                    &distinct_values,
                    event,
                    &target_class,
                    time,
                    true,
                    &[],
                );
                if let Some(reason) = matches.abort {
                    exclusions.push(reason);
                    continue;
                }
                unknown_time_state |= matches.unknown;
                has_time_state |= !matches.values.is_empty();
                if has_time_state {
                    // setTime uses past || future, so a successful past lookup
                    // must not be invalidated by an excluded future value.
                    break;
                }
            }
        }
        let direct = preferred_event_value_matches(
            &values,
            event,
            &target_class,
            effective_time,
            false,
            &[],
        );
        if let Some(reason) = direct.abort {
            exclusions.push(reason);
            continue;
        }
        unknown |= direct.unknown;
        if direct.values.len() > 1 {
            return Err(DefaultFailure::Rejected(format!(
                "multiple {target_class} event values in {event}"
            )));
        }
        let matches = if direct.values.is_empty() {
            preferred_event_value_matches(&values, event, &target_class, effective_time, true, &[])
        } else {
            direct
        };
        if let Some(reason) = matches.abort {
            exclusions.push(reason);
            continue;
        }
        unknown |= matches.unknown;
        if matches.values.len() > 1 {
            return Err(DefaultFailure::Rejected(format!(
                "multiple {target_class} event values in {event}"
            )));
        }
        selected.extend(matches.values);
    }
    if !has_time_state {
        return Err(if unknown_time_state {
            DefaultFailure::Unresolved(format!(
                "{target_class} EventValue time-state support is unresolved"
            ))
        } else {
            DefaultFailure::Rejected(format!(
                "{} DefaultExpression does not have distinct time states in the current Event",
                payload.active_type.code_name
            ))
        });
    }
    if unknown
        || selected.iter().any(|value| {
            value.context_dependent != Some(false)
                || value.has_custom_event_validator != Some(false)
        })
    {
        return Err(DefaultFailure::Unresolved(format!(
            "{target_class} EventValue requires unavailable context or validator semantics"
        )));
    }
    if selected.is_empty() {
        if !exclusions.is_empty() {
            return Err(DefaultFailure::Rejected(
                exclusions
                    .into_iter()
                    .map(|excluded| excluded.reason)
                    .collect::<Vec<_>>()
                    .join("; "),
            ));
        }
        return Err(DefaultFailure::Rejected(format!(
            "omitted {} requires {}; the current Event provides none ({})",
            payload.active_type.code_name,
            target_class,
            payload.context.event_classes.join(", ")
        )));
    }
    let mut references = vec![type_reference(payload)];
    references.extend(selected.iter().map(|value| {
        let record = value.source_record.as_ref();
        DefaultExpressionCatalogReference {
            role: "event-value".to_owned(),
            definition_id: None,
            registration_id: Some(value.registration_id.clone()),
            source_digest: record.map(|record| record.source_digest.clone()),
            snapshot_id: record.map(|record| record.snapshot_id.clone()),
            document: record.map(|record| record.document.clone()),
            index: record.map(|record| record.index),
        }
    }));
    Ok(ResolvedDefault {
        diagnostics: exclusions
            .into_iter()
            .filter_map(|excluded| excluded.diagnostic)
            .collect(),
        expression: DefaultExpressionResolution {
            provider_id: PROVIDER_ID.to_owned(),
            component_id: None,
            return_type: target_class.clone(),
            multiplicity,
            is_literal: false,
            time: accepted_time,
            reason: format!(
                "{} defaults to EventValueExpression<{target_class}> from {}",
                payload.active_type.code_name,
                selected
                    .iter()
                    .map(|value| format!("{} -> {}", value.event_class, value.value_class))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            catalog_references: references,
            public_data: Vec::new(),
        },
    })
}

fn static_shape(
    payload: &DefaultExpressionPayload,
    descriptor: &TypeDefaultExpression,
    expected_literal: bool,
) -> Result<(String, DynamicMultiplicity), DefaultFailure> {
    if descriptor
        .literal
        .is_some_and(|literal| literal != expected_literal)
    {
        return Err(DefaultFailure::Unresolved(format!(
            "{} has inconsistent DefaultExpression literal metadata",
            payload.active_type.code_name
        )));
    }
    let return_type = descriptor.return_type.clone().ok_or_else(|| {
        DefaultFailure::Unresolved(format!(
            "{} DefaultExpression has no statically verified return type",
            payload.active_type.code_name
        ))
    })?;
    let single = descriptor.single.ok_or_else(|| {
        DefaultFailure::Unresolved(format!(
            "{} DefaultExpression has no statically verified multiplicity",
            payload.active_type.code_name
        ))
    })?;
    if !payload.expected_type.plural && !single {
        return Err(DefaultFailure::Rejected(format!(
            "{} requires one value, but its DefaultExpression returns multiple values",
            payload.active_type.code_name
        )));
    }
    Ok((
        return_type,
        if single {
            DynamicMultiplicity::Single
        } else {
            DynamicMultiplicity::Multiple
        },
    ))
}

fn type_reference(payload: &DefaultExpressionPayload) -> DefaultExpressionCatalogReference {
    let record = payload.active_type.source_record.as_ref();
    DefaultExpressionCatalogReference {
        role: "type".to_owned(),
        definition_id: Some(payload.active_type.definition_id.clone()),
        registration_id: Some(payload.active_type.registration_id.clone()),
        source_digest: record.map(|record| record.source_digest.clone()),
        snapshot_id: record.map(|record| record.snapshot_id.clone()),
        document: record.map(|record| record.document.clone()),
        index: record.map(|record| record.index),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    const AUDIENCE: &str = "net.kyori.adventure.audience.Audience";
    const SENDER: &str = "org.bukkit.command.CommandSender";

    fn payload() -> DefaultExpressionPayload {
        let anchor = TextRange { start: 10, end: 10 };
        DefaultExpressionPayload {
            metadata: Vec::new(),
            context: ParseContext {
                syntax_context: 3,
                event_classes: vec!["test.Event".to_owned()],
                section_stack: Vec::new(),
                values: Vec::new(),
            },
            skript_version: Some("2.16.0".to_owned()),
            syntax_kind: SyntaxKind::Effect,
            definition_id: "effect:test:message".to_owned(),
            registration_id: "effect:test:message:0".to_owned(),
            pattern_index: Some(0),
            pattern: Some("send %objects% [to %audiences%]".to_owned()),
            tags: Vec::new(),
            mark: Some(0),
            capture_index: 1,
            pattern_span: TextRange { start: 19, end: 30 },
            expression: "audiences".to_owned(),
            expected_type: ExpressionExpectedType {
                class_name: AUDIENCE.to_owned(),
                plural: true,
            },
            active_type: ExpressionTypeOption {
                source_record: Some(CatalogRecordRef {
                    source_digest: "snapshot-digest".to_owned(),
                    snapshot_id: "snapshot-id".to_owned(),
                    document: "Types.json".to_owned(),
                    index: 133,
                    byte_length: 100,
                }),
                definition_id: "type:skript:audience".to_owned(),
                registration_id: "type:skript:audience:0".to_owned(),
                addon_name: "Skript".to_owned(),
                addon_version: "2.16.0".to_owned(),
                code_name: "audience".to_owned(),
                class_name: AUDIENCE.to_owned(),
                parser_class: Some(
                    "org.skriptlang.skript.bukkit.text.types.AudienceClassInfo$AudienceParser"
                        .to_owned(),
                ),
                type_parse_order: 133,
                before: Vec::new(),
                after: Vec::new(),
                singular: "audience".to_owned(),
                plural: "audiences".to_owned(),
                user_input_patterns: vec!["audiences?".to_owned()],
                has_parser: true,
                parse_contexts: Vec::new(),
                has_supplier: false,
                default_expression: Some(TypeDefaultExpression {
                    implementation_class: EVENT_VALUE_CLASS.to_owned(),
                    literal: Some(false),
                    return_type: Some(SENDER.to_owned()),
                    single: Some(true),
                }),
            },
            allow_literals: true,
            allow_expressions: true,
            time: 0,
            span: MappedSpan {
                virtual_range: anchor,
                origins: vec![SourceOrigin {
                    original_range: anchor,
                    kind: OriginKind::Exact,
                    expansion: None,
                }],
            },
            outcome: DefaultExpressionOutcome::Unresolved(
                "no provider has resolved the default".to_owned(),
            ),
        }
    }

    fn sender(id: &str, time: i32) -> EventValueOption {
        EventValueOption {
            source_record: Some(CatalogRecordReference {
                source_digest: "event-values-digest".to_owned(),
                snapshot_id: "snapshot-id".to_owned(),
                document: "EventValues.json".to_owned(),
                index: 7,
                byte_length: 80,
            }),
            event_class: "test.Event".to_owned(),
            value_class: SENDER.to_owned(),
            time,
            registration_id: id.to_owned(),
            patterns: Vec::new(),
            excludes: Vec::new(),
            exclude_error_message: None,
            resolution_order: 0,
            registration_order: Some(0),
            accepted_changers: BTreeMap::new(),
            context_dependent: Some(false),
            has_custom_input_validator: Some(false),
            has_custom_event_validator: Some(false),
        }
    }

    fn resolve_values(
        payload: &DefaultExpressionPayload,
        values: Vec<EventValueOption>,
    ) -> Result<DefaultExpressionResolution, DefaultFailure> {
        resolve_with_catalog(payload, |_| Ok(values.clone())).map(|resolved| resolved.expression)
    }

    #[test]
    fn subscription_covers_all_type_defaults() {
        let subscription = subscription();
        assert!(matches!(
            subscription.target,
            HookTarget::SyntaxKind(SyntaxKind::Type)
        ));
        assert_eq!(subscription.phase, HookPhase::DefaultExpression);
        assert_eq!(subscription.priority, i32::MAX);
        assert!(subscription.selector.return_type.is_none());
    }

    #[test]
    fn no_context_is_rejected_without_inventing_a_sender() {
        let mut payload = payload();
        payload.context.event_classes.clear();
        let result = resolve_with_catalog(&payload, |_| panic!("no event means no Catalog lookup"));
        assert!(
            matches!(result, Err(DefaultFailure::Rejected(reason)) if reason.contains("CommandSender") && reason.contains("no Event context"))
        );
    }

    #[test]
    fn available_sender_has_single_multiplicity_and_catalog_provenance() {
        let payload = payload();
        let resolved =
            resolve_values(&payload, vec![sender("event-value:test:sender", 0)]).unwrap();
        assert_eq!(resolved.return_type, SENDER);
        assert_eq!(resolved.multiplicity, DynamicMultiplicity::Single);
        assert!(!resolved.is_literal);
        assert_eq!(resolved.provider_id, PROVIDER_ID);
        assert!(
            resolved.component_id.is_none(),
            "the host owns component identity"
        );
        assert_eq!(resolved.catalog_references.len(), 2);
        assert_eq!(
            resolved.catalog_references[0].registration_id.as_deref(),
            Some("type:skript:audience:0")
        );
        assert_eq!(
            resolved.catalog_references[1].registration_id.as_deref(),
            Some("event-value:test:sender")
        );
        assert_eq!(
            resolved.catalog_references[1].source_digest.as_deref(),
            Some("event-values-digest")
        );
        assert_eq!(
            resolved.catalog_references[1].document.as_deref(),
            Some("EventValues.json")
        );
        assert_eq!(resolved.catalog_references[1].index, Some(7));
        assert_eq!(
            payload.span.virtual_range.start,
            payload.span.virtual_range.end
        );
        assert_eq!(payload.span.origins[0].kind, OriginKind::Exact);
    }

    #[test]
    fn event_without_sender_is_rejected_and_multi_event_context_needs_one_success() {
        let mut payload = payload();
        payload.context.event_classes = vec!["test.NoSenderEvent".to_owned()];
        assert!(
            matches!(resolve_values(&payload, Vec::new()), Err(DefaultFailure::Rejected(reason)) if reason.contains("CommandSender"))
        );
        payload.context.event_classes.push("test.Event".to_owned());
        let result = resolve_with_catalog(&payload, |event| {
            Ok(if event == "test.Event" {
                vec![sender("sender", 0)]
            } else {
                Vec::new()
            })
        });
        assert!(result.is_ok());
    }

    #[test]
    fn missing_and_addon_specific_descriptors_remain_unresolved() {
        let mut payload = payload();
        payload.active_type.default_expression = None;
        assert!(
            matches!(resolve(&payload), Err(DefaultFailure::Unresolved(reason)) if reason.contains("no DefaultExpression descriptor"))
        );
        payload.active_type.default_expression = Some(TypeDefaultExpression {
            implementation_class: "addon.CustomDefault".to_owned(),
            literal: None,
            return_type: None,
            single: None,
        });
        assert!(matches!(
            resolve_with_catalog(&payload, |_| panic!("custom defaults cannot query event values")),
            Err(DefaultFailure::Unresolved(reason)) if reason.contains("addon.CustomDefault")
        ));

        payload.active_type.default_expression = Some(TypeDefaultExpression {
            implementation_class: EVENT_VALUE_CLASS.to_owned(),
            literal: Some(false),
            return_type: None,
            single: Some(true),
        });
        assert!(matches!(
            resolve_with_catalog(&payload, |_| panic!("incomplete descriptors cannot query event values")),
            Err(DefaultFailure::Unresolved(reason)) if reason.contains("no statically verified return type")
        ));
    }

    #[test]
    fn addon_owned_types_can_use_skript_standard_default_implementations() {
        let mut payload = payload();
        payload.active_type.addon_name = "SkBee".to_owned();
        payload.active_type.default_expression = Some(TypeDefaultExpression {
            implementation_class: SIMPLE_LITERAL_CLASS.to_owned(),
            literal: Some(true),
            return_type: Some(AUDIENCE.to_owned()),
            single: Some(true),
        });
        let output = invoke(HookInvocation {
            context: InvocationContext {
                invocation_id: 1,
                subscription_id: PROVIDER_ID.to_owned(),
                document_id: "file:///default-expression.sk".to_owned(),
                document_revision: 1,
                expansion: None,
                syntax_context: 3,
            },
            target: HookTarget::SyntaxKind(SyntaxKind::Type),
            phase: HookPhase::DefaultExpression,
            parse_results: Vec::new(),
            payload: HookPayload::DefaultExpression(payload),
        });
        assert!(matches!(
            output.replacement,
            Some(HookPayload::DefaultExpression(DefaultExpressionPayload {
                outcome: DefaultExpressionOutcome::Resolved(_),
                ..
            }))
        ));
    }

    #[test]
    fn typed_descriptors_are_not_limited_to_hard_coded_versions() {
        for version in [None, Some("2.6.4"), Some("2.15.4"), Some("2.17.0")] {
            let mut payload = payload();
            payload.skript_version = version.map(str::to_owned);
            assert!(resolve_values(&payload, vec![sender("sender", 0)]).is_ok());
        }
    }

    #[test]
    fn simple_literal_defaults_use_the_descriptor_shape_without_event_context() {
        let mut payload = payload();
        payload.context.event_classes.clear();
        payload.expected_type = ExpressionExpectedType {
            class_name: "java.lang.Number".to_owned(),
            plural: false,
        };
        payload.active_type.code_name = "number".to_owned();
        payload.active_type.class_name = "java.lang.Number".to_owned();
        payload.active_type.default_expression = Some(TypeDefaultExpression {
            implementation_class: SIMPLE_LITERAL_CLASS.to_owned(),
            literal: Some(true),
            return_type: Some("java.lang.Integer".to_owned()),
            single: Some(true),
        });

        let resolved = resolve_with_catalog(&payload, |_| {
            panic!("SimpleLiteral must not query EventValues")
        })
        .unwrap()
        .expression;
        assert_eq!(resolved.return_type, "java.lang.Integer");
        assert_eq!(resolved.multiplicity, DynamicMultiplicity::Single);
        assert!(resolved.is_literal);

        payload.allow_literals = false;
        assert!(matches!(
            resolve(&payload),
            Err(DefaultFailure::Rejected(reason)) if reason.contains("only allows Expressions")
        ));
        payload.allow_literals = true;
        payload.time = -1;
        assert!(matches!(
            resolve(&payload),
            Err(DefaultFailure::Rejected(reason)) if reason.contains("without time states")
        ));
    }

    #[test]
    fn singular_captures_reject_plural_default_shapes() {
        let mut payload = payload();
        payload.expected_type.plural = false;
        payload
            .active_type
            .default_expression
            .as_mut()
            .unwrap()
            .single = Some(false);
        assert!(matches!(
            resolve_values(&payload, vec![sender("sender", 0)]),
            Err(DefaultFailure::Rejected(reason)) if reason.contains("returns multiple values")
        ));
        payload.expected_type.plural = true;
        let resolved = resolve_values(&payload, vec![sender("sender", 0)]).unwrap();
        assert_eq!(resolved.multiplicity, DynamicMultiplicity::Multiple);
    }

    #[test]
    fn damage_cause_accepts_past_syntax_but_resolves_the_present_value() {
        const DAMAGE_CAUSE: &str = "org.bukkit.event.entity.EntityDamageEvent$DamageCause";
        let mut payload = payload();
        payload.expected_type = ExpressionExpectedType {
            class_name: DAMAGE_CAUSE.to_owned(),
            plural: false,
        };
        payload.active_type.code_name = "damagecause".to_owned();
        payload.active_type.class_name = DAMAGE_CAUSE.to_owned();
        payload.active_type.default_expression = Some(TypeDefaultExpression {
            implementation_class: DAMAGE_CAUSE_CLASS.to_owned(),
            literal: Some(false),
            return_type: Some(DAMAGE_CAUSE.to_owned()),
            single: Some(true),
        });
        let mut present = sender("damage-cause", 0);
        present.value_class = DAMAGE_CAUSE.to_owned();
        payload.time = -1;
        let resolved = resolve_values(&payload, vec![present.clone()]).unwrap();
        assert_eq!(resolved.time, -1);

        payload.time = 1;
        assert!(matches!(
            resolve_values(&payload, vec![present]),
            Err(DefaultFailure::Rejected(reason)) if reason.contains("future event value")
        ));
    }

    #[test]
    fn literal_only_capture_rejects_the_nonliteral_default() {
        let mut payload = payload();
        payload.allow_expressions = false;
        assert!(
            matches!(resolve_values(&payload, vec![sender("sender", 0)]), Err(DefaultFailure::Rejected(reason)) if reason.contains("only allows literals"))
        );
    }

    #[test]
    fn ambiguous_and_excluded_senders_are_rejected() {
        assert!(
            matches!(resolve_values(&payload(), vec![sender("first", 0), sender("second", 0)]), Err(DefaultFailure::Rejected(reason)) if reason.contains("multiple") && reason.contains(SENDER))
        );
        let mut excluded = sender("excluded", 0);
        excluded.excludes.push("test.Event".to_owned());
        excluded.exclude_error_message = Some("sender is excluded here".to_owned());
        assert_eq!(
            resolve_values(&payload(), vec![excluded]).unwrap_err(),
            DefaultFailure::Rejected("sender is excluded here".to_owned())
        );
    }

    #[test]
    fn unknown_validator_or_context_never_becomes_verified_success() {
        let mut custom = sender("custom", 0);
        custom.has_custom_event_validator = Some(true);
        for values in [vec![custom.clone()], vec![sender("known", 0), custom]] {
            assert!(matches!(
                resolve_values(&payload(), values),
                Err(DefaultFailure::Unresolved(_))
            ));
        }
        let mut dependent = sender("context-dependent", 0);
        dependent.context_dependent = Some(true);
        assert!(matches!(
            resolve_values(&payload(), vec![dependent]),
            Err(DefaultFailure::Unresolved(_))
        ));
        let mut missing_validator = sender("missing-validator-descriptor", 0);
        missing_validator.has_custom_event_validator = None;
        let mut missing_context = sender("missing-context-descriptor", 0);
        missing_context.context_dependent = None;
        for value in [missing_validator, missing_context] {
            assert!(matches!(
                resolve_values(&payload(), vec![value]),
                Err(DefaultFailure::Unresolved(_))
            ));
        }
        let mut input_only = sender("input-validator", 0);
        input_only.has_custom_input_validator = Some(true);
        assert!(
            resolve_values(&payload(), vec![input_only]).is_ok(),
            "typed EventValue lookup does not invoke identifier validators"
        );
    }

    #[test]
    fn time_states_require_distinct_support_before_default_time_fallback() {
        let mut payload = payload();
        payload.time = -1;
        assert!(
            matches!(resolve_values(&payload, vec![sender("present", 0)]), Err(DefaultFailure::Rejected(reason)) if reason.contains("distinct time states"))
        );
        let values = vec![sender("present", 0), sender("past", -1)];
        let past = resolve_values(&payload, values.clone()).unwrap();
        assert_eq!(past.time, -1);
        assert_eq!(
            past.catalog_references[1].registration_id.as_deref(),
            Some("past")
        );
        let mut excluded_future = sender("future", 1);
        excluded_future.excludes = vec!["test.Event".to_owned()];
        assert!(resolve_values(&payload, vec![sender("past", -1), excluded_future]).is_ok());
        payload.time = 1;
        let future = resolve_values(&payload, values).unwrap();
        assert_eq!(future.time, 1);
        assert_eq!(
            future.catalog_references[1].registration_id.as_deref(),
            Some("present")
        );
        payload.time = -1;
        assert!(
            matches!(resolve_values(&payload, vec![sender("past-a", -1), sender("past-b", -1)]), Err(DefaultFailure::Rejected(reason)) if reason.contains("multiple") && reason.contains(SENDER))
        );
    }

    #[test]
    fn excluded_past_can_use_future_but_cannot_initialize_the_past() {
        let mut payload = payload();
        payload.time = 1;
        let mut past = sender("past", -1);
        past.excludes.push("test.Event".to_owned());
        past.exclude_error_message = Some("past sender is excluded".to_owned());
        let values = vec![past, sender("future", 1)];
        let resolved = resolve_with_catalog(&payload, |_| Ok(values.clone())).unwrap();
        assert_eq!(resolved.expression.time, 1);
        assert_eq!(
            resolved.expression.catalog_references[1]
                .registration_id
                .as_deref(),
            Some("future")
        );
        assert_eq!(resolved.diagnostics, vec!["past sender is excluded"]);
        payload.time = -1;
        assert!(
            matches!(resolve_values(&payload, values), Err(DefaultFailure::Rejected(reason)) if reason.contains("past sender is excluded"))
        );
    }

    #[test]
    fn excluded_event_does_not_block_another_event_and_preserves_registered_diagnostics() {
        for reversed in [false, true] {
            for diagnostic in [None, Some("sender is excluded here".to_owned())] {
                let mut payload = payload();
                payload.context.event_classes =
                    vec!["test.ExcludedEvent".to_owned(), "test.Event".to_owned()];
                if reversed {
                    payload.context.event_classes.reverse();
                }
                let mut excluded = sender("excluded", 0);
                excluded.event_class = "test.ExcludedEvent".to_owned();
                excluded.excludes.push("test.ExcludedEvent".to_owned());
                excluded.exclude_error_message = diagnostic.clone();
                let resolve = |ambiguous| {
                    resolve_with_catalog(&payload, |event| {
                        Ok(if event == "test.ExcludedEvent" {
                            vec![excluded.clone()]
                        } else if ambiguous {
                            vec![sender("first", 0), sender("second", 0)]
                        } else {
                            vec![sender("accepted", 0)]
                        })
                    })
                };
                let resolved = resolve(false).unwrap();
                assert_eq!(
                    resolved.expression.catalog_references[1]
                        .registration_id
                        .as_deref(),
                    Some("accepted")
                );
                assert_eq!(
                    resolved.diagnostics,
                    diagnostic.into_iter().collect::<Vec<_>>()
                );
                assert!(
                    matches!(resolve(true), Err(DefaultFailure::Rejected(reason)) if reason.contains("multiple") && reason.contains(SENDER))
                );
            }
        }
    }
}

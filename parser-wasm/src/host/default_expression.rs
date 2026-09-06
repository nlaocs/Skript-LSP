//! Default providers use the same dispatch, ownership and deferred effects as syntax hooks.

use super::*;
use skript_parser::{DefaultExpressionDecision as Decision, DefaultExpressionRequest};

pub(super) fn provide(
    environment: &mut WasmExpressionEnvironment<'_>,
    request: DefaultExpressionRequest<'_>,
) -> Result<Decision, String> {
    let state = environment
        .hooks
        .transaction
        .savepoint()
        .map_err(|error| error.to_string())?;
    let checkpoint = HookEffectsCheckpoint::capture(
        &environment.hooks.effects,
        &environment.hooks.calls,
        &environment.hooks.failures,
    );
    let result = match provide_inner(environment, request) {
        Ok(Decision::Resolved(mut resolution)) => {
            match environment.defer_effects(&state, &checkpoint) {
                Ok(effects) => {
                    resolution.effects = Some(effects);
                    return Ok(Decision::Resolved(resolution));
                }
                Err(error) => Err(error),
            }
        }
        result => result,
    };
    let rollback = environment.hooks.transaction.rollback_to(&state);
    checkpoint.restore(
        &mut environment.hooks.effects,
        &mut environment.hooks.calls,
        &mut environment.hooks.failures,
    );
    match result {
        Err(error) => Err(error),
        Ok(decision) => {
            rollback.map_err(|error| error.to_string())?;
            Ok(decision)
        }
    }
}

fn provide_inner(
    environment: &mut WasmExpressionEnvironment<'_>,
    request: DefaultExpressionRequest<'_>,
) -> Result<Decision, String> {
    let syntax_kind = match request.syntax.kind {
        CatalogSyntaxKind::Event => SyntaxKind::Event,
        CatalogSyntaxKind::Condition => SyntaxKind::Condition,
        CatalogSyntaxKind::Effect => SyntaxKind::Effect,
        CatalogSyntaxKind::Expression => SyntaxKind::Expression,
        CatalogSyntaxKind::Type => SyntaxKind::Type,
        CatalogSyntaxKind::Function => SyntaxKind::Function,
        CatalogSyntaxKind::Section => SyntaxKind::Section,
        CatalogSyntaxKind::Structure => SyntaxKind::Structure,
    };
    let payload = DefaultExpressionPayload {
        metadata: Vec::new(),
        context: parse_context_to_wit(request.context),
        skript_version: environment
            .hooks
            .host
            .config
            .runtime_profile
            .skript_version
            .clone(),
        syntax_kind,
        definition_id: request.syntax.definition_id.to_owned(),
        registration_id: request.syntax.registration_id.to_owned(),
        pattern_index: request.syntax.pattern_index.map(|index| index as u64),
        pattern: request.syntax.pattern_source.map(str::to_owned),
        tags: request
            .syntax
            .tags
            .unwrap_or_default()
            .iter()
            .map(|tag| tag.value.clone())
            .collect(),
        mark: request.syntax.mark,
        capture_index: request.capture_index as u64,
        pattern_span: WitTextRange {
            start: request.pattern_span.start as u64,
            end: request.pattern_span.end as u64,
        },
        expression: request
            .syntax
            .pattern_source
            .and_then(|source| source.get(request.pattern_span.start..request.pattern_span.end))
            .unwrap_or_default()
            .to_owned(),
        expected_type: WitExpressionExpectedType {
            class_name: request.expected_type.class_name.as_str().to_owned(),
            plural: request.expected_type.plural,
        },
        active_type: expression_type_option(
            environment.hooks.host.config.syntax_catalog.as_deref(),
            request.value_type,
        ),
        allow_literals: request.expression.allow_literals,
        allow_expressions: request.expression.allow_expressions,
        time: request.expression.time,
        span: mapped_span_to_wit(request.span.mapped.clone()),
        outcome: DefaultExpressionOutcome::Unresolved(format!(
            "no DefaultExpression provider for {}",
            request.value_type.code_name.as_str()
        )),
    };
    let result = environment
        .hooks
        .host
        .dispatch_in_parse(
            environment.hooks.transaction,
            DispatchRequest {
                context: environment.hooks.context.clone(),
                target: DispatchTarget::Registration {
                    syntax_kind: SyntaxKind::Type,
                    definition_id: request.value_type.definition_id.as_str().to_owned(),
                    registration_id: request.value_type.registration_id.as_str().to_owned(),
                },
                phase: HookPhase::DefaultExpression,
                payload: HookPayload::DefaultExpression(payload),
            },
        )
        .map_err(|error| error.to_string())?;
    if !result.failures.is_empty() {
        return Ok(Decision::Unresolved {
            reason: format!("DefaultExpression provider failed: {:?}", result.failures),
        });
    }
    let diagnostics = semantic_rejection_diagnostics(&result.decision, &result.effects)?;
    if let HookDecision::Reject(rejection) = result.decision {
        return Ok(Decision::Rejected {
            reason: rejection.reason,
            diagnostics,
        });
    }
    let HookPayload::DefaultExpression(payload) = result.payload else {
        return Err("DefaultExpression provider returned a different payload kind".to_owned());
    };
    let resolution = match payload.outcome {
        DefaultExpressionOutcome::Unresolved(reason) => return Ok(Decision::Unresolved { reason }),
        DefaultExpressionOutcome::Resolved(resolution) => resolution,
    };
    if let Err(reason) = validate_resolution_catalog(
        environment.hooks.host.config.syntax_catalog.as_deref(),
        &resolution,
    ) {
        return Ok(Decision::Unresolved { reason });
    }
    let resolution = skript_parser::DefaultExpressionResolution {
        provider_id: resolution.provider_id,
        component_id: resolution
            .component_id
            .ok_or("DefaultExpression provider has no component identity")?,
        return_type: ClassName(resolution.return_type),
        multiplicity: multiplicity_from_wit(resolution.multiplicity),
        is_literal: resolution.is_literal,
        time: resolution.time,
        reason: resolution.reason,
        catalog_references: resolution
            .catalog_references
            .into_iter()
            .map(
                |reference| skript_parser::DefaultExpressionCatalogReference {
                    role: reference.role,
                    definition_id: reference.definition_id,
                    registration_id: reference.registration_id,
                    source_digest: reference.source_digest,
                    snapshot_id: reference.snapshot_id,
                    document: reference.document,
                    index: reference.index,
                },
            )
            .collect(),
        public_data: public_data::from_wit(resolution.public_data)?,
        metadata: metadata_entries(payload.metadata)?,
        effects: None,
    };
    merge_effects(&mut environment.hooks.effects, result.effects);
    environment.hooks.calls.extend(result.calls);
    Ok(Decision::Resolved(Box::new(resolution)))
}

fn validate_resolution_catalog(
    catalog: Option<&syntaxes::Catalog>,
    resolution: &DefaultExpressionResolution,
) -> Result<(), String> {
    let catalog = catalog.ok_or("DefaultExpression resolution requires an SSG catalog")?;
    if catalog.class(&resolution.return_type).is_none() {
        return Err(format!(
            "DefaultExpression returned unknown class {}",
            resolution.return_type
        ));
    }
    let source = catalog
        .source()
        .ok_or("DefaultExpression provenance requires retained SSG source data")?;
    for reference in &resolution.catalog_references {
        let source_fields = [
            reference.source_digest.is_some(),
            reference.snapshot_id.is_some(),
            reference.document.is_some(),
            reference.index.is_some(),
        ];
        if source_fields.iter().any(|present| *present)
            && !source_fields.iter().all(|present| *present)
        {
            return Err(
                "DefaultExpression catalog reference has partial source identity".to_owned(),
            );
        }
        if let (Some(digest), Some(snapshot_id), Some(document), Some(index)) = (
            reference.source_digest.as_deref(),
            reference.snapshot_id.as_deref(),
            reference.document.as_deref(),
            reference.index,
        ) {
            if digest != source.source_digest || snapshot_id != source.snapshot_id {
                return Err(
                    "DefaultExpression catalog reference belongs to another snapshot".to_owned(),
                );
            }
            let index = usize::try_from(index)
                .map_err(|_| "DefaultExpression catalog reference index is too large")?;
            if source.record(document, index).is_none() {
                return Err("DefaultExpression catalog reference does not exist".to_owned());
            }
            if reference
                .registration_id
                .as_deref()
                .is_some_and(|registration_id| {
                    !source
                        .records_by_registration_id(registration_id)
                        .iter()
                        .any(|record| record.document == document && record.index == index)
                })
            {
                return Err(
                    "DefaultExpression catalog reference has a mismatched registration ID"
                        .to_owned(),
                );
            }
            if reference
                .definition_id
                .as_deref()
                .is_some_and(|definition_id| {
                    !source
                        .records_by_definition_id(definition_id)
                        .iter()
                        .any(|record| record.document == document && record.index == index)
                })
            {
                return Err(
                    "DefaultExpression catalog reference has a mismatched definition ID".to_owned(),
                );
            }
        }
    }
    Ok(())
}

pub(super) fn normalize(
    original: &DefaultExpressionPayload,
    replacement: &mut DefaultExpressionPayload,
    component_id: &str,
) -> Result<(), String> {
    if !same_parse_context(&original.context, &replacement.context)
        || original.skript_version != replacement.skript_version
        || original.syntax_kind != replacement.syntax_kind
        || original.definition_id != replacement.definition_id
        || original.registration_id != replacement.registration_id
        || original.pattern_index != replacement.pattern_index
        || original.pattern != replacement.pattern
        || original.tags != replacement.tags
        || original.mark != replacement.mark
        || original.capture_index != replacement.capture_index
        || original.pattern_span.start != replacement.pattern_span.start
        || original.pattern_span.end != replacement.pattern_span.end
        || original.expression != replacement.expression
        || original.expected_type.class_name != replacement.expected_type.class_name
        || original.expected_type.plural != replacement.expected_type.plural
        || !same_expression_type_option(&original.active_type, &replacement.active_type)
        || original.allow_literals != replacement.allow_literals
        || original.allow_expressions != replacement.allow_expressions
        || original.time != replacement.time
        || !same_mapped_span(&original.span, &replacement.span)
    {
        return Err("DefaultExpression hook changed immutable request fields".to_owned());
    }
    merge_owned_metadata(&original.metadata, &mut replacement.metadata, component_id)?;
    let DefaultExpressionOutcome::Resolved(resolution) = &mut replacement.outcome else {
        return Ok(());
    };
    if resolution.provider_id.trim().is_empty()
        || resolution.return_type.trim().is_empty()
        || resolution.reason.trim().is_empty()
    {
        return Err("DefaultExpression requires a provider, return type and reason".to_owned());
    }
    if matches!(resolution.multiplicity, WitDynamicMultiplicity::Both) {
        return Err(
            "DefaultExpression must resolve to a concrete single or multiple multiplicity"
                .to_owned(),
        );
    }
    if !(-1..=1).contains(&resolution.time) {
        return Err("DefaultExpression time must be -1, 0 or 1".to_owned());
    }
    let previous = match &original.outcome {
        DefaultExpressionOutcome::Resolved(value) => Some(value),
        _ => None,
    };
    if let Some(owner) = resolution.component_id.as_deref() {
        if !previous.is_some_and(|value| {
            value.component_id.as_deref() == Some(owner)
                && value.provider_id == resolution.provider_id
                && (owner == component_id || same_resolution_value(value, resolution))
        }) {
            return Err("DefaultExpression component identity is host-owned; a new resolution must request a new owner stamp".to_owned());
        }
    } else {
        resolution.component_id = Some(component_id.to_owned());
    }
    public_data::validate(&resolution.public_data)?;
    Ok(())
}

fn same_resolution_value(
    left: &DefaultExpressionResolution,
    right: &DefaultExpressionResolution,
) -> bool {
    left.return_type == right.return_type
        && left.multiplicity == right.multiplicity
        && left.is_literal == right.is_literal
        && left.time == right.time
        && left.reason == right.reason
        && same_references(&left.catalog_references, &right.catalog_references)
}

fn same_references(
    left: &[DefaultExpressionCatalogReference],
    right: &[DefaultExpressionCatalogReference],
) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.role == right.role
                && left.definition_id == right.definition_id
                && left.registration_id == right.registration_id
                && left.source_digest == right.source_digest
                && left.snapshot_id == right.snapshot_id
                && left.document == right.document
                && left.index == right.index
        })
}

pub(super) fn node_info(node: &ExpressionNode) -> Option<DefaultExpressionInfo> {
    let ExpressionNodeKind::Default { info } = &node.kind else {
        return None;
    };
    Some(info_to_wit(info))
}

pub(super) fn info_to_request(
    info: &skript_parser::DefaultExpressionInfo,
    request: &ParseRequest,
) -> DefaultExpressionInfo {
    let mut result = info_to_wit(info);
    result.span = nested_span_to_request(
        &MatchSpan {
            local_range: info.anchor.virtual_range,
            mapped: info.anchor.clone(),
        },
        request,
    );
    result
}

pub(super) fn info_to_wit(info: &skript_parser::DefaultExpressionInfo) -> DefaultExpressionInfo {
    DefaultExpressionInfo {
        capture_index: info.capture_index as u64,
        pattern_span: WitTextRange {
            start: info.pattern_span.start as u64,
            end: info.pattern_span.end as u64,
        },
        expression: info.expression_source.clone(),
        requested_type: WitExpressionExpectedType {
            class_name: info.requested_type.class_name.as_str().to_owned(),
            plural: info.requested_type.plural,
        },
        type_definition_id: info.type_definition_id.clone(),
        type_registration_id: info.type_registration_id.clone(),
        provider_id: info.provider_id.clone(),
        component_id: info.component_id.clone(),
        reason: info.reason.clone(),
        event_classes: info
            .event_classes
            .iter()
            .map(|class| class.as_str().to_owned())
            .collect(),
        section_scope_ids: info.section_scope_ids.clone(),
        catalog_references: info
            .catalog_references
            .iter()
            .map(|reference| DefaultExpressionCatalogReference {
                role: reference.role.clone(),
                definition_id: reference.definition_id.clone(),
                registration_id: reference.registration_id.clone(),
                source_digest: reference.source_digest.clone(),
                snapshot_id: reference.snapshot_id.clone(),
                document: reference.document.clone(),
                index: reference.index,
            })
            .collect(),
        is_literal: info.is_literal,
        time: info.time,
        span: mapped_span_to_wit(info.anchor.clone()),
    }
}

fn reference_size(reference: &DefaultExpressionCatalogReference) -> usize {
    reference.role.len()
        + reference.definition_id.as_ref().map_or(0, String::len)
        + reference.registration_id.as_ref().map_or(0, String::len)
        + reference.source_digest.as_ref().map_or(0, String::len)
        + reference.snapshot_id.as_ref().map_or(0, String::len)
        + reference.document.as_ref().map_or(0, String::len)
        + 8
}

pub(super) fn same_info(
    left: Option<&DefaultExpressionInfo>,
    right: Option<&DefaultExpressionInfo>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.capture_index == right.capture_index
                && left.pattern_span.start == right.pattern_span.start
                && left.pattern_span.end == right.pattern_span.end
                && left.expression == right.expression
                && left.requested_type.class_name == right.requested_type.class_name
                && left.requested_type.plural == right.requested_type.plural
                && left.type_definition_id == right.type_definition_id
                && left.type_registration_id == right.type_registration_id
                && left.provider_id == right.provider_id
                && left.component_id == right.component_id
                && left.reason == right.reason
                && left.event_classes == right.event_classes
                && left.section_scope_ids == right.section_scope_ids
                && left.is_literal == right.is_literal
                && left.time == right.time
                && same_mapped_span(&left.span, &right.span)
                && same_references(&left.catalog_references, &right.catalog_references)
        }
        _ => false,
    }
}

fn mapped_span_size(span: &MappedSpan) -> usize {
    16 + span.origins.len() * 40
}

pub(super) fn capture_state(state: skript_parser::TypeCaptureState) -> TypeCaptureState {
    match state {
        skript_parser::TypeCaptureState::Explicit => TypeCaptureState::Explicit,
        skript_parser::TypeCaptureState::Omitted => TypeCaptureState::Omitted,
        skript_parser::TypeCaptureState::Null => TypeCaptureState::Null,
        skript_parser::TypeCaptureState::Default => TypeCaptureState::Default,
    }
}

pub(super) fn info_size(info: &DefaultExpressionInfo) -> usize {
    64 + info.expression.len()
        + info.requested_type.class_name.len()
        + info.type_definition_id.len()
        + info.type_registration_id.len()
        + info.provider_id.len()
        + info.component_id.len()
        + info.reason.len()
        + info.event_classes.iter().map(String::len).sum::<usize>()
        + info.section_scope_ids.len() * 8
        + info
            .catalog_references
            .iter()
            .map(reference_size)
            .sum::<usize>()
        + mapped_span_size(&info.span)
}

pub(super) fn payload_size(payload: &DefaultExpressionPayload) -> usize {
    let outcome = match &payload.outcome {
        DefaultExpressionOutcome::Unresolved(reason) => reason.len(),
        DefaultExpressionOutcome::Resolved(value) => {
            value.provider_id.len()
                + value.component_id.as_ref().map_or(0, String::len)
                + value.return_type.len()
                + value.reason.len()
                + value
                    .catalog_references
                    .iter()
                    .map(reference_size)
                    .sum::<usize>()
                + public_data::size(&value.public_data)
        }
    };
    128 + parse_context_size(&payload.context)
        + metadata_entries_size(&payload.metadata)
        + payload.skript_version.as_ref().map_or(0, String::len)
        + payload.definition_id.len()
        + payload.registration_id.len()
        + payload.pattern.as_ref().map_or(0, String::len)
        + payload.tags.iter().map(String::len).sum::<usize>()
        + payload.expression.len()
        + payload.expected_type.class_name.len()
        + expression_type_option_size(&payload.active_type)
        + mapped_span_size(&payload.span)
        + outcome
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn cancelling_inside_default_provider_discards_resolution_and_state() {
        const COMPONENT: &str = "test.expression-data-a";
        const NAMESPACE: &str = "default-expression-test";
        const DOCUMENT: &str = "file:///workspace/default-cancel.sk";
        let catalog = Arc::new(
            ssg::load(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/data/type-parser-versions/skript-2.16.0"),
            )
            .unwrap()
            .into_catalog(),
        );
        let mut host = ParserHost::new(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../artifacts/core-library.wasm"
            )),
            HostConfig {
                syntax_catalog: Some(Arc::clone(&catalog)),
                runtime_profile: RuntimeProfile {
                    skript_version: Some("2.16.0".to_owned()),
                    ..RuntimeProfile::default()
                },
                ..HostConfig::default()
            },
        )
        .unwrap();
        host.load_addon(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../artifacts/expression-data-addon-a.wasm"
        )))
        .unwrap();
        let transaction = host.begin_parse("file:///workspace", DOCUMENT, 1).unwrap();
        let addon = host
            .components
            .iter()
            .position(|entry| entry.manifest.component_id == COMPONENT)
            .unwrap();
        host.components[addon]
            .store
            .data_mut()
            .cancel_after_state_put = Some(("default-test.a.late".to_owned(), transaction.clone()));
        let pattern =
            syntax_pattern_parser::syntax::parse("%number%", catalog.plural_rules()).unwrap();
        let syntax_pattern_parser::syntax::PatternElement::TypeExpr(expression) =
            &pattern.elements[0].value
        else {
            panic!("typed capture expected");
        };
        let context = ExpressionParseContext::default();
        let span = MatchSpan {
            local_range: ParserTextRange::empty(0),
            mapped: skript_parser::MappedSpan {
                virtual_range: ParserTextRange::empty(0),
                origins: vec![skript_parser::SourceOrigin {
                    original_range: ParserTextRange::empty(0),
                    kind: ParserOriginKind::Anchored,
                    expansion: None,
                }],
            },
        };
        let mut environment = WasmExpressionEnvironment {
            hooks: WasmPatternHooks {
                host: &mut host,
                transaction: &transaction,
                dynamic_snapshot: None,
                matching_hooks_registered: false,
                context: InvocationContext {
                    invocation_id: 1,
                    subscription_id: String::new(),
                    document_id: DOCUMENT.to_owned(),
                    document_revision: 1,
                    expansion: None,
                    syntax_context: 0,
                },
                input: String::new(),
                frames: Vec::new(),
                scope_frames: Vec::new(),
                branch_states: Vec::new(),
                last_candidate: None,
                effects: empty_effects(),
                calls: Vec::new(),
                failures: Vec::new(),
            },
            pending_leaf: None,
            pending_registered: None,
            expression_candidates: Vec::new(),
            semantic_candidates: Vec::new(),
            function_registry: None,
        };
        let result = environment.provide_default_expression(DefaultExpressionRequest {
            syntax: RegisteredSyntaxIdentity {
                kind: CatalogSyntaxKind::Expression,
                definition_id: "expression:test.default-expression",
                registration_id: "expression:test.default-expression:0",
                pattern_index: Some(0),
                pattern_source: Some("%number%"),
                tags: None,
                mark: None,
                dynamic_handler: None,
            },
            capture_index: 0,
            pattern_span: pattern.elements[0].span,
            expression,
            expected_type: &skript_parser::ExpressionExpectedType {
                class_name: ClassName("java.lang.Number".to_owned()),
                plural: false,
            },
            value_type: catalog.type_by_code_name("number").unwrap(),
            span: &span,
            context: &context,
        });
        assert!(
            result.is_err(),
            "cancel must not return a default or its metadata: {result:?}"
        );
        let (effects, calls, failures) = environment.into_parts();
        assert!(effects.diagnostics.is_empty());
        assert!(effects.context_updates.is_empty());
        assert!(effects.parse_requests.is_empty());
        assert!(effects.parse_results.is_empty());
        assert!(calls.is_empty());
        assert!(failures.is_empty());
        let store = host.components[addon].store.data();
        assert!(
            store.cancel_after_state_put.is_none(),
            "cancel ran inside the late provider's put import"
        );
        assert!(store.invocation.is_none());
        // Accepted-access history survives cancel, although the values do not.
        // This proves seed returned successfully before cancellation inside late.
        let accesses = transaction.read_write_set().unwrap();
        assert!(
            accesses
                .writes
                .iter()
                .any(|entry| entry.key == "default-test.a.seed")
        );
        assert!(
            !accesses
                .writes
                .iter()
                .any(|entry| entry.key == "default-test.a.late")
        );
        assert!(matches!(
            transaction.commit(),
            Err(StateError::TransactionClosed)
        ));
        let next = host.begin_parse("file:///workspace", DOCUMENT, 2).unwrap();
        let mut invocation = next.begin_invocation(COMPONENT).unwrap();
        for key in ["default-test.a.seed", "default-test.a.late"] {
            assert!(
                invocation
                    .get(
                        StateScope::Document,
                        NamespaceVisibility::Private,
                        NAMESPACE,
                        key
                    )
                    .unwrap()
                    .is_none()
            );
        }
        invocation.rollback();
        next.cancel().unwrap();
    }

    fn default_node() -> ExpressionNode {
        // Obtain the native node through its public parser API; its internal
        // routed-capture storage deliberately has no public constructor.
        let catalog = ssg::load(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/data/type-parser-versions/skript-2.16.0"),
        )
        .expect("the existing SSG fixture loads")
        .into_catalog();
        let input = "dummy direct registry expression";
        let source = MappedSource::identity(input);
        let mut node = skript_parser::parse_expression(
            &catalog,
            ExpressionParseRequest {
                source: &source,
                range: ParserTextRange::new(0, input.len()),
                expected_types: Vec::new(),
                context: ExpressionParseContext::default(),
            },
            &mut skript_parser::NoopExpressionEnvironment,
            ExpressionParserConfig::default(),
        )
        .expect("the direct fixture Expression parses")
        .selected
        .expect("the direct fixture Expression is selected")
        .node;
        let pattern = syntax_pattern_parser::syntax::parse("%string%", catalog.plural_rules())
            .expect("the fixture placeholder parses");
        let syntax_pattern_parser::syntax::PatternElement::TypeExpr(expression) =
            &pattern.elements[0].value
        else {
            panic!("typed placeholder expected");
        };
        // A nested matcher's local offset differs from its position within the
        // AdditionalParse input. Both ranges are deliberately nonzero.
        node.span = MatchSpan {
            local_range: ParserTextRange::empty(2),
            mapped: skript_parser::MappedSpan {
                virtual_range: ParserTextRange::empty(4),
                origins: vec![
                    skript_parser::SourceOrigin::anchored(40, ExpansionId::new(7)),
                    skript_parser::SourceOrigin::anchored(80, ExpansionId::new(8)),
                ],
            },
        };
        node.kind = ExpressionNodeKind::Default {
            info: Box::new(skript_parser::DefaultExpressionInfo {
                capture_index: 1,
                pattern_span: pattern.elements[0].span,
                expression: expression.clone(),
                expression_source: "%string%".to_owned(),
                requested_type: skript_parser::ExpressionExpectedType {
                    class_name: ClassName("java.lang.String".to_owned()),
                    plural: false,
                },
                type_definition_id: "type:fixture:string".to_owned(),
                type_registration_id: "type:fixture:string:0".to_owned(),
                provider_id: "fixture.default".to_owned(),
                component_id: "fixture.addon".to_owned(),
                reason: "fixture context supplies this value".to_owned(),
                event_classes: vec![ClassName("fixture.Event".to_owned())],
                section_scope_ids: vec![19],
                anchor: node.span.mapped.clone(),
                catalog_references: Vec::new(),
                is_literal: false,
                time: 0,
            }),
        };
        node
    }

    fn additional_request() -> ParseRequest {
        ParseRequest {
            request_id: 3,
            parser_id: skript_parser::HOST_EXPRESSION_PARSER_ID.to_owned(),
            input: "0123456789".to_owned(),
            expected_types: Vec::new(),
            span: MappedSpan {
                virtual_range: WitTextRange {
                    start: 100,
                    end: 110,
                },
                origins: vec![
                    WitSourceOrigin {
                        original_range: WitTextRange { start: 50, end: 60 },
                        kind: WitOriginKind::Replaced,
                        expansion: Some(11),
                    },
                    WitSourceOrigin {
                        original_range: WitTextRange { start: 80, end: 83 },
                        kind: WitOriginKind::Replaced,
                        expansion: Some(12),
                    },
                    WitSourceOrigin {
                        original_range: WitTextRange {
                            start: 150,
                            end: 150,
                        },
                        kind: WitOriginKind::Anchored,
                        expansion: Some(13),
                    },
                ],
            },
            options: Vec::new(),
        }
    }

    fn assert_request_anchor(span: &MappedSpan) {
        assert_eq!(
            (span.virtual_range.start, span.virtual_range.end),
            (104, 104)
        );
        assert_eq!(span.origins.len(), 3);
        for (origin, (anchor, expansion)) in
            span.origins.iter().zip([(54, 11), (83, 12), (150, 13)])
        {
            assert_eq!(origin.kind, WitOriginKind::Anchored);
            assert_eq!(origin.original_range.start, anchor);
            assert_eq!(origin.original_range.end, anchor);
            assert_eq!(origin.expansion, Some(expansion));
        }
    }

    #[test]
    fn native_span_rebasing_updates_nested_default_info_and_preserves_all_origins() {
        let child = default_node();
        let original_info = match &child.kind {
            ExpressionNodeKind::Default { info } => (**info).clone(),
            _ => unreachable!(),
        };
        let mut parent = child.clone();
        parent.kind = ExpressionNodeKind::Grouped;
        parent.span.local_range = ParserTextRange::new(0, 10);
        parent.span.mapped.virtual_range = ParserTextRange::new(0, 10);
        parent.children = vec![child];
        parent
            .try_map_spans(&mut |span| {
                let mut mapped = span.clone();
                mapped.local_range =
                    ParserTextRange::new(span.local_range.start + 6, span.local_range.end + 6);
                mapped.mapped.virtual_range = ParserTextRange::new(
                    span.mapped.virtual_range.start + 10,
                    span.mapped.virtual_range.end + 10,
                );
                for origin in &mut mapped.mapped.origins {
                    origin.original_range = ParserTextRange::new(
                        origin.original_range.start + 20,
                        origin.original_range.end + 20,
                    );
                }
                Ok::<_, std::convert::Infallible>(mapped)
            })
            .unwrap();

        let child = &parent.children[0];
        let ExpressionNodeKind::Default { info } = &child.kind else {
            panic!("default child expected");
        };
        assert_eq!(child.span.local_range, ParserTextRange::empty(8));
        assert_eq!(child.span.mapped.virtual_range, ParserTextRange::empty(14));
        assert_eq!(info.anchor, child.span.mapped);
        assert_eq!(info.anchor.origins.len(), 2);
        for (origin, (anchor, expansion)) in info.anchor.origins.iter().zip([(60, 7), (100, 8)]) {
            assert_eq!(origin.kind, ParserOriginKind::Anchored);
            assert_eq!(origin.original_range, ParserTextRange::empty(anchor));
            assert_eq!(origin.expansion, Some(ExpansionId::new(expansion)));
        }
        let mut expected_info = original_info;
        expected_info.anchor = child.span.mapped.clone();
        assert_eq!(
            **info, expected_info,
            "rebasing only changes source provenance"
        );
    }

    #[test]
    fn additional_parse_expression_and_opaque_summaries_share_the_rebased_default_anchor() {
        let node = default_node();
        let ExpressionNodeKind::Default { info } = &node.kind else {
            panic!("default node expected");
        };
        let request = additional_request();
        let mut arena = ParseResultArena::new(&request, None);
        let expression_id = arena.push_expression(&node);
        let capture = ParserParsedCapture {
            capture_index: info.capture_index,
            binding: RegisteredCaptureBinding {
                capture_index: info.capture_index,
                parser_id: "fixture.opaque".to_owned(),
                required: true,
                options: BTreeMap::new(),
            },
            result: skript_parser::ParsedCaptureResult {
                parser_id: "fixture.opaque".to_owned(),
                status: ParserParsedCaptureStatus::Success,
                span: node.span.clone(),
                summary: Some(skript_parser::ParsedCaptureSemanticSummary {
                    default_expression: Some((**info).clone()),
                    kind: "default-expression".to_owned(),
                    definition_id: None,
                    registration_id: None,
                    element_class: None,
                    pattern_index: None,
                    return_type: node.return_type.clone(),
                    possible_return_types: node.possible_return_types.clone(),
                    possible_return_types_state: node.possible_return_types_state,
                    multiplicity: node.multiplicity,
                    public_data: Vec::new(),
                    metadata: BTreeMap::new(),
                }),
                value: None,
                diagnostics: Vec::new(),
                attachments: Vec::new(),
            },
        };
        let opaque_id = arena.push_opaque_capture(&capture, "opaque", &node.span);
        for id in [expression_id, opaque_id] {
            let rendered = &arena.nodes[id as usize];
            assert!(rendered.text.is_empty(), "defaults have no source text");
            assert_request_anchor(&rendered.span);
            let provenance = rendered
                .summary
                .as_ref()
                .and_then(|summary| summary.default_expression.as_ref())
                .expect("the arena retains implicit provenance");
            assert_request_anchor(&provenance.span);
            assert!(same_mapped_span(&rendered.span, &provenance.span));
            assert_eq!(provenance.capture_index, 1);
            assert_eq!(provenance.provider_id, "fixture.default");
            assert_eq!(provenance.section_scope_ids, vec![19]);
        }
    }
}

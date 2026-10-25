//! Retain the pre-annotation request without duplicating its immutable body.
use super::*;

pub(super) struct DispatchOriginal {
    kind: mem::Discriminant<HookPayload>,
    metadata: Option<Vec<WitMetadataEntry>>,
    retained: Option<HookPayload>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::nlaocs::skript_parser_addon::types::{
        EffectCandidate, EffectFailure, EffectNearMatch, EffectPayload, EffectTiming,
    };

    fn registered() -> HookPayload {
        let mut payload = crate::host::tests::registered_expression();
        let HookPayload::RegisteredExpression(value) = &mut payload else { unreachable!() };
        value.input = "public immutable input\n".repeat(8192);
        value.metadata = vec![entry(None, "same", "first"), entry(Some("owner"), "same", "owned"),
            entry(None, "same", "duplicate"), entry(None, "unknown-schema", "{\"future\":true}")];
        payload
    }
    fn entry(owner: Option<&str>, key: &str, value: &str) -> WitMetadataEntry {
        WitMetadataEntry { owner_component_id: owner.map(str::to_owned), key: key.into(), value: value.into() }
    }
    fn annotate(payload: &mut HookPayload) {
        let entries = payload_metadata_mut(payload).unwrap();
        entries[0] = entry(None, "same", "annotation overwrote first");
        entries.push(entry(Some("annotation-owner"), "extra", "appended"));
    }
    fn input_pointer(payload: &HookPayload) -> *const u8 {
        match payload { HookPayload::RegisteredExpression(value) => value.input.as_ptr(), _ => unreachable!() }
    }

    #[test]
    fn fault_before_adoption_restores_exact_metadata_and_keeps_the_input_allocation() {
        let mut payload = registered();
        let expected = format!("{payload:?}"); let pointer = input_pointer(&payload);
        let original = DispatchOriginal::capture(&payload);
        annotate(&mut payload);
        let restored = original.restore(payload).unwrap();
        assert_eq!(format!("{restored:?}"), expected);
        assert_eq!(input_pointer(&restored), pointer);
    }

    #[test]
    fn multiple_adoptions_retain_only_the_first_preannotation_request_by_move() {
        for replacements in [1, 3] {
            let mut payload = registered(); let expected = format!("{payload:?}");
            let pointer = input_pointer(&payload); let mut original = DispatchOriginal::capture(&payload);
            annotate(&mut payload);
            for index in 0..replacements {
                let mut next = registered();
                let HookPayload::RegisteredExpression(value) = &mut next else { unreachable!() };
                value.input = format!("replacement{index}");
                value.metadata = vec![entry(Some("guest"), "result", &index.to_string())];
                original.retain(mem::replace(&mut payload, next)).unwrap();
            }
            let restored = original.restore(payload).unwrap();
            assert_eq!(format!("{restored:?}"), expected);
            assert_eq!(input_pointer(&restored), pointer);
        }
    }

    #[test]
    fn legacy_and_moved_original_agree_for_faults_before_and_after_adoption() {
        for adoption in [false, true] {
            let mut payload = registered(); let expected = format!("{payload:?}");
            let mut legacy = DispatchOriginal::legacy(&payload);
            let mut current = DispatchOriginal::capture(&payload);
            annotate(&mut payload);
            if adoption {
                legacy.retain(payload.clone()).unwrap(); current.retain(payload).unwrap();
                payload = registered();
            }
            let reference = legacy.restore(payload.clone()).unwrap();
            let actual = current.restore(payload).unwrap();
            assert_eq!(format!("{reference:?}"), expected);
            assert_eq!(format!("{actual:?}"), expected);
        }
    }

    fn effect(candidate: bool, near: bool) -> HookPayload {
        let HookPayload::RegisteredExpression(base) = registered() else { unreachable!() };
        let failure = EffectFailure { offset: 0, span: base.span.clone(), reasons: vec!["no match".into()] };
        HookPayload::Effect(EffectPayload {
            input: base.input, context: base.context, raw_node_id: 1, span: base.span.clone(),
            timing: EffectTiming::After,
            candidate: candidate.then(|| EffectCandidate {
                raw_node_id: 1, definition_id: "effect".into(), registration_id: "effect:r".into(),
                element_class: None, priority: 0, registration_order: 0, pattern_index: 0,
                pattern: "effect".into(), span: base.span.clone(), captures: vec![], tags: vec![],
                mark: 0, marks: vec![], handler: None, metadata: vec![], parsed_captures: vec![],
            }), alternatives: vec![], failure: None,
            near_match: near.then(|| EffectNearMatch {
                definition_id: "near".into(), registration_id: "near:r".into(), element_class: None,
                priority: 0, registration_order: 0, resolved_order: None, handler: None,
                metadata: vec![entry(Some("near"), "kept", "untouched")], failure,
            }),
        })
    }

    #[test]
    fn effect_metadata_projection_preserves_candidate_near_match_and_absent_vs_empty() {
        for (candidate, near) in [(true,true),(false,true),(true,false),(false,false)] {
            let mut payload = effect(candidate, near); let expected = format!("{payload:?}");
            let original = DispatchOriginal::capture(&payload);
            if let Some(metadata) = payload_metadata_mut(&mut payload) { metadata.push(entry(None,"added","annotation")); }
            let restored = original.restore(payload).unwrap();
            assert_eq!(format!("{restored:?}"), expected);
        }
    }

    #[test]
    fn wrong_payload_kind_before_adoption_is_an_internal_error() {
        let original = DispatchOriginal::capture(&registered());
        assert!(matches!(original.restore(effect(false,false)), Err(HostError::StateStore(StateError::Internal { .. }))));
    }

    #[test]
    fn dispatch_fault_before_adoption_undoes_real_catalog_annotations_in_both_strategies() {
        use crate::bindings::nlaocs::skript_parser_addon::types::{CatalogAnnotation, CatalogAnnotationTarget};
        for legacy in [false, true] {
            let mut host = ParserHost::new(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),
                "/../artifacts/core-library.wasm")), HostConfig {
                    runtime_profile: RuntimeProfile { skript_version: Some("2.15.4".into()), ..Default::default() },
                    ..Default::default()
                }).unwrap();
            host.legacy_dispatch_original = legacy;
            host.semantic_receipts = Some(semantic_proof::ReceiptRegistry::default());
            host.load_addon(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),
                "/../artifacts/expression-data-addon-a.wasm"))).unwrap();
            let fault_index = host.components.len() - 1;
            host.components[fault_index].disabled = true;
            // An active annotation provider and an unavailable normal subscriber
            // are independent. No guest execution is needed for this fault path.
            let subscription = host.components[0].manifest.subscriptions.iter()
                .find(|s|s.id == "core.registered-expression-semantics").unwrap().clone();
            host.registry = SubscriptionRegistry::default();
            host.registry.register(fault_index, 1, &[subscription]);
            host.components[0].manifest.catalog_annotations.push(CatalogAnnotation {
                target: CatalogAnnotationTarget::Registration("expression:test:registration".into()),
                metadata: vec![entry(None,"same","overwritten"), entry(Some("catalog"),"added","new")],
            });
            let payload = registered(); let expected = format!("{payload:?}");
            let result = host.dispatch("file:///rollback-test", DispatchRequest {
                context: InvocationContext { invocation_id: 1, subscription_id: String::new(),
                    document_id: "file:///rollback-test/public.sk".into(), document_revision: 1,
                    expansion: None, syntax_context: 0 },
                target: DispatchTarget::Registration { definition_id: "expression:test:definition".into(),
                    registration_id: "expression:test:registration".into(), syntax_kind: SyntaxKind::Expression },
                phase: HookPhase::Expression, payload,
            }).unwrap();
            assert!(result.fault.is_some());
            assert_eq!(format!("{:?}", result.payload), expected);
            assert!(result.calls.iter().all(|call|call.semantic_receipt.is_none()));
            assert!(result.effects.context_updates.is_empty() && result.effects.parse_results.is_empty());
        }
    }

    #[test]
    fn dispatch_late_fault_restores_preannotation_request_and_clears_adopted_receipts() {
        use crate::bindings::nlaocs::skript_parser_addon::types::{CatalogAnnotation, CatalogAnnotationTarget};
        let catalog = Arc::new(ssg::load(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4")).unwrap().into_catalog());
        let expression = catalog.expressions().find(|e|e.common.element_class.as_str().ends_with(".LitPi")).unwrap();
        for legacy in [false, true] {
            let mut host = ParserHost::new(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),
                "/../artifacts/core-library.wasm")), HostConfig {
                    syntax_catalog: Some(Arc::clone(&catalog)),
                    runtime_profile: RuntimeProfile { skript_version: Some("2.15.4".into()), ..Default::default() },
                    ..Default::default()
                }).unwrap();
            host.legacy_dispatch_original = legacy;
            host.semantic_receipts = Some(semantic_proof::ReceiptRegistry::default());
            host.load_addon(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"),
                "/../artifacts/expression-data-addon-a.wasm"))).unwrap();
            let fault_index = host.components.len() - 1;
            host.components[fault_index].disabled = true;
            let subscription = host.components[0].manifest.subscriptions.iter()
                .find(|s|s.id == "core.registered-expression-semantics").unwrap().clone();
            host.registry = SubscriptionRegistry::default();
            host.registry.register(0, 0, &[subscription.clone()]);
            host.components[0].manifest.catalog_annotations.push(CatalogAnnotation {
                target: CatalogAnnotationTarget::Registration(expression.common.registration_id.as_str().into()),
                metadata: vec![entry(None,"same","overwritten"), entry(Some("catalog"),"added","new")],
            });
            let mut payload = registered();
            let HookPayload::RegisteredExpression(value) = &mut payload else { unreachable!() };
            value.input = "pi".into(); value.span.virtual_range.end = 2;
            value.definition_id = expression.common.definition_id.as_str().into();
            value.registration_id = expression.common.registration_id.as_str().into();
            value.element_class = expression.common.element_class.as_str().into();
            value.pattern_index = 0;
            value.pattern = expression.common.patterns[0].source.clone();
            value.declared_return_type = expression.return_type.as_ref().map(|t|t.as_str().into());
            value.return_type_state = match expression.return_type_state {
                syntaxes::ReturnTypeState::Static => ExpressionReturnTypeState::Static,
                syntaxes::ReturnTypeState::Dynamic => ExpressionReturnTypeState::Dynamic,
                syntaxes::ReturnTypeState::Unresolved => ExpressionReturnTypeState::Unresolved,
            };
            value.possible_return_types = expression.possible_return_types.iter().map(|t|t.as_str().into()).collect();
            value.possible_return_types_state = match expression.possible_return_types_state {
                syntaxes::PossibleReturnTypesState::Complete => ExpressionPossibleReturnTypesState::Complete,
                syntaxes::PossibleReturnTypesState::Partial => ExpressionPossibleReturnTypesState::Partial,
                syntaxes::PossibleReturnTypesState::Unresolved => ExpressionPossibleReturnTypesState::Unresolved,
            };
            value.declared_multiplicity = expression.return_type_multiplicity.map(|kind|match kind {
                syntaxes::Multiplicity::Single => WitDynamicMultiplicity::Single,
                syntaxes::Multiplicity::Multiple => WitDynamicMultiplicity::Multiple,
                syntaxes::Multiplicity::Both => WitDynamicMultiplicity::Both,
            });
            value.effective_return_type = value.declared_return_type.clone();
            value.effective_possible_return_types = value.possible_return_types.clone();
            value.effective_possible_return_types_state = value.possible_return_types_state;
            value.effective_multiplicity = value.declared_multiplicity;
            value.tags.clear(); value.mark = 0;
            // Unlike helper-only restoration cases, this positive dispatch must
            // satisfy the normal unique owner/key metadata contract.
            value.metadata = vec![entry(None,"same","first"), entry(None,"unknown-schema","{\"future\":true}")];
            let request = || DispatchRequest {
                context: InvocationContext { invocation_id: 1, subscription_id: String::new(),
                    document_id: "file:///rollback-test/late.sk".into(), document_revision: 1,
                    expansion: None, syntax_context: 0 },
                target: DispatchTarget::Registration { definition_id: value.definition_id.clone(),
                    registration_id: value.registration_id.clone(), syntax_kind: SyntaxKind::Expression },
                phase: HookPhase::Expression, payload: HookPayload::RegisteredExpression(value.clone()),
            };
            let expected = format!("{:?}", request().payload);
            let healthy = host.dispatch("file:///rollback-test", request()).unwrap();
            assert!(healthy.fault.is_none(), "{:?}", healthy.fault);
            assert_eq!(healthy.calls.len(), 1);
            assert!(healthy.calls[0].semantic_receipt.is_some());
            assert_ne!(format!("{:?}", healthy.payload), expected, "real Core adoption must occur");
            host.registry.register(fault_index, 1, &[subscription]);
            let fault = host.dispatch("file:///rollback-test", request()).unwrap();
            assert!(fault.fault.is_some());
            assert_eq!(fault.calls.len(), 1, "Core executes before the unavailable subscriber");
            assert_eq!(format!("{:?}", fault.payload), expected);
            assert!(fault.calls.iter().all(|call|call.semantic_receipt.is_none()));
            assert!(fault.effects.diagnostics.is_empty() && fault.effects.context_updates.is_empty()
                && fault.effects.parse_requests.is_empty() && fault.effects.parse_results.is_empty());
            assert!(fault.available_parse_results.is_empty());
        }
    }
}

impl DispatchOriginal {
    pub(super) fn capture(payload: &HookPayload) -> Self {
        Self {
            kind: mem::discriminant(payload),
            metadata: payload_metadata(payload).map(<[WitMetadataEntry]>::to_vec),
            retained: None,
        }
    }

    #[cfg(test)]
    pub(super) fn legacy(payload: &HookPayload) -> Self {
        Self { kind: mem::discriminant(payload), metadata: None, retained: Some(payload.clone()) }
    }

    fn undo_annotations(&mut self, mut payload: HookPayload) -> Result<HookPayload, HostError> {
        if mem::discriminant(&payload) != self.kind
            || payload_metadata(&payload).is_some() != self.metadata.is_some()
        {
            return Err(StateError::Internal {
                message: "dispatch input identity changed before its first adoption".to_owned(),
            }.into());
        }
        if let Some(metadata) = self.metadata.take() {
            *payload_metadata_mut(&mut payload).expect("metadata projection was checked") = metadata;
        }
        Ok(payload)
    }

    /// Until this point the input has only top-level catalog annotations applied:
    /// guest calls borrow it, and incremental key normalization restores it.
    /// Any future persistent pre-adoption mutation must extend this checkpoint.
    pub(super) fn retain(&mut self, previous: HookPayload) -> Result<(), HostError> {
        if self.retained.is_none() {
            self.retained = Some(self.undo_annotations(previous)?);
        }
        Ok(())
    }

    pub(super) fn restore(mut self, current: HookPayload) -> Result<HookPayload, HostError> {
        match self.retained.take() {
            Some(original) => Ok(original),
            None => self.undo_annotations(current),
        }
    }
}

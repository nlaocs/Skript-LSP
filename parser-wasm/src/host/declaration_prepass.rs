//! Bounded declaration discovery over the final native RawTree, before freeze.
//! Providers publish valid header prototypes; recoverable implementation-body
//! diagnostics remain visible and confer no executable, purity or value proof.
use super::*;

pub(super) const PARSER_ID: &str = "host.document-declarations";
pub(super) const TREE_OPTION: &str = "native.tree-view.v1";

/// The carrier contains no source text or node arrays. Its mapped whole-source
/// span is for diagnostics; this is never an executable additional parse request.
fn request_payload(source: &MappedSource) -> Result<HookPayload, HostError> {
    Ok(HookPayload::Parser(ParseRequest {
        request_id: 0,
        parser_id: PARSER_ID.into(),
        input: String::new(),
        expected_types: Vec::new(),
        options: Vec::new(),
        span: mapped_span_to_wit(
            source
                .map_range(ParserTextRange::new(0, source.virtual_source().len()))
                .map_err(|error| HostError::SourceMapping {
                    message: error.to_string(),
                })?,
        ),
    }))
}

/// Install only the current Store's invocation lease. Decimal strings avoid
/// precision loss in consumers; possession of these strings alone grants no access.
pub(super) fn bind_tree_reference(
    payload: &mut HookPayload,
    reference: tree_view::Reference,
) -> Result<(), HostError> {
    let HookPayload::Parser(request) = payload else {
        return Err(StateError::InvalidInput {
            message: "invalid declaration carrier".into(),
        }
        .into());
    };
    if request.parser_id != PARSER_ID
        || request.request_id != 0
        || !request.input.is_empty()
        || !request.expected_types.is_empty()
    {
        return Err(StateError::InvalidInput {
            message: "invalid declaration carrier identity".into(),
        }
        .into());
    }
    request.options = vec![WitMetadataEntry { key: TREE_OPTION.into(), owner_component_id: None,
        value: serde_json::json!({"version":1,
            "epochHigh":reference.epoch_high.to_string(), "epochLow":reference.epoch_low.to_string(),
            "token":reference.token.to_string(), "generation":reference.generation.to_string()}).to_string() }];
    Ok(())
}

pub(super) fn validate_subscription(
    manifest: &ComponentManifest,
    subscription: &HookSubscription,
) -> Result<(), HostError> {
    if !matches!(subscription.target, HookTarget::ParseStage)
        || subscription.phase != HookPhase::Document
        || subscription.mode != HookMode::Transform
        || !selector_is_empty(&subscription.selector)
        || !manifest.capabilities.iter().any(|claim| {
            claim.id == crate::CAPABILITY_DOCUMENT_DECLARATIONS
                && claim.required
                && claim.minimum_version == 1
        })
        || !manifest.capabilities.iter().any(|claim| {
            claim.id == crate::CAPABILITY_PROJECT_STABLE_SEMANTICS
                && claim.required
                && claim.minimum_version == 2
        })
        || !project_stability_promise(&manifest.capabilities, true)
    {
        return Err(HostError::InvalidManifest { message:
            "document declaration subscriptions require parse-stage Document/Transform, an empty selector, document-declarations v1 and project-stable-semantics v2".into() });
    }
    Ok(())
}

/// The native tree is immutable. Additional parsing here would freeze the
/// registry before other declaration providers could finish.
pub(super) fn validate_output(output: &HookOutput) -> Result<(), String> {
    if output.replacement.is_some()
        || !output.effects.context_updates.is_empty()
        || !output.effects.parse_requests.is_empty()
        || !output.effects.parse_results.is_empty()
        || matches!(output.decision, HookDecision::Handled)
    {
        return Err("document declarations allow diagnostics, private Parse state and dynamic registrations, but no replacement, context updates, extra parsing or terminal Handled decision".into());
    }
    Ok(())
}

impl ParserHost {
    pub(super) fn prepare_document_declarations(
        &mut self,
        transaction: &ParseTransaction,
        context: &InvocationContext,
        source: &MappedSource,
        tree: Arc<ParserRawTree>,
    ) -> Result<Option<DispatchResult>, HostError> {
        self.check_native_cancellation()?;
        if transaction.document_id()? != context.document_id
            || transaction.document_revision()? != context.document_revision
        {
            return Err(StateError::InvalidInput {
                message: "document declaration context differs from its parse transaction".into(),
            }
            .into());
        }
        if !self.registry.has_active_matching_capability(
            &self.components,
            &DispatchTarget::ParseStage,
            HookPhase::Document,
            crate::CAPABILITY_DOCUMENT_DECLARATIONS,
        ) {
            return Ok(None);
        }
        if source.virtual_source().len() > self.config.max_virtual_source_bytes {
            return Err(HostError::VirtualSourceQuotaExceeded {
                limit: self.config.max_virtual_source_bytes,
            });
        }
        if tree.nodes.len() > self.config.max_tree_macro_nodes {
            return Err(HostError::TreeMacroNodeQuotaExceeded {
                limit: self.config.max_tree_macro_nodes,
            });
        }
        if raw_tree_depth(&tree) > self.config.max_raw_tree_depth {
            return Err(HostError::RawTreeDepthQuotaExceeded {
                limit: self.config.max_raw_tree_depth,
            });
        }
        let state = transaction.savepoint()?;
        let dynamic = self
            .dynamic_syntax_registry
            .as_ref()
            .ok_or(HostError::DynamicSyntaxUnavailable)?
            .savepoint(&context.document_id, context.document_revision)?;
        let prepared = self.dispatch_with_transaction_for_capability(transaction, DispatchRequest {
            context: context.clone(), target: DispatchTarget::ParseStage,
            phase: HookPhase::Document, payload: request_payload(source)?,
        }, crate::CAPABILITY_DOCUMENT_DECLARATIONS, Some(tree), None).and_then(|mut result| {
            normalize_text_macro_effects(source, &mut result.effects, "document-declarations.effects")
                .map_err(|message| HostError::SourceMapping { message })?;
            if let HookDecision::Reject(rejection) = &result.decision {
                return Err(HostError::AddonFailure {
                    kind: crate::bindings::nlaocs::skript_parser_addon::types::AddonErrorKind::InvalidPayload,
                    component_id: result.calls.last().map_or("document declarations", |call| call.component_id.as_str()).to_owned(),
                    message: format!("document declaration preparation rejected: {}", rejection.reason),
                });
            }
            Ok(result)
        });
        if prepared
            .as_ref()
            .map_or(true, |result| result.fault.is_some())
        {
            transaction.rollback_to(&state)?;
            self.dynamic_syntax_registry
                .as_ref()
                .ok_or(HostError::DynamicSyntaxUnavailable)?
                .rollback_to(&dynamic)?;
        }
        prepared.map(Some)
    }
}

#[cfg(test)]
mod tests;

//! Only the final accepted native Structure can bind its entry context effects.
use super::{ReceiptRegistry, SemanticEffects};
use crate::host::{
    HookEffects, HookPayload, HookPhase, WitStructureBodyMode, WitStructureNodeType,
    WitStructureTiming,
};

impl SemanticEffects {
    pub(in crate::host) fn include_hook_effects(
        self,
        phase: HookPhase,
        syntax_context: u64,
        payload: &HookPayload,
        effects: &HookEffects,
    ) -> Self {
        if self == Self::Unbound
            || !effects.parse_requests.is_empty()
            || !effects.parse_results.is_empty()
        {
            return Self::Unbound;
        }
        if effects.context_updates.is_empty() {
            return self;
        }
        if phase == HookPhase::Structure
            && let HookPayload::Structure(value) = payload
            && value.timing == WitStructureTiming::EnterBody
            && value.context.syntax_context == syntax_context
            && value.candidate.declarations.is_empty()
            && effects
                .context_updates
                .iter()
                .all(|update| update.syntax_context == syntax_context)
        {
            Self::StructureContext
        } else {
            Self::Unbound
        }
    }
}

impl ReceiptRegistry {
    /// The caller supplies only provisional tokens issued directly by this
    /// successful dispatch, after every provider has accepted the final body.
    pub(in crate::host) fn bind_structure_context(&mut self, payload: &HookPayload, tokens: &[u64]) {
        let HookPayload::Structure(value) = payload else {
            return;
        };
        if value.timing != WitStructureTiming::EnterBody
            || value.candidate.actual_node_type != WitStructureNodeType::Section
            || value.candidate.body_mode == WitStructureBodyMode::None
            || !value.candidate.declarations.is_empty()
        {
            return;
        }
        for token in tokens {
            if let Some(receipt) = self.receipts.get_mut(token)
                && receipt.valid
                && receipt.provisional_structure_context
            {
                // Do not traverse parents or admitted_nested. Their reach and
                // authorship remain independent, even inside this body scope.
                receipt.unbound = false;
                receipt.provisional_structure_context = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::nlaocs::skript_parser_addon::types as wit;
    use crate::host::semantic_proof::tests::{node, payload, write};
    use crate::host::semantic_proof::{SemanticProofRegistry, RECEIPT};
    use crate::host::{AstNode, AstTree, HookCall, HookCallOrigin, HookMode};

    fn structure() -> HookPayload {
        HookPayload::Structure(wit::StructurePayload {
            input: "fixture".into(),
            body_tree: wit::RawTree {
                roots: vec![], nodes: vec![], diagnostics: vec![], indentation: None,
            },
            context: wit::ParseContext {
                syntax_context: 7, event_classes: vec![], section_stack: vec![], values: vec![],
            },
            timing: WitStructureTiming::EnterBody,
            type_options: vec![],
            candidate: wit::StructureCandidate {
                raw_node_id: 0, definition_id: "fixture".into(), registration_id: "fixture.0".into(),
                element_class: None, priority: 0, registration_order: 0, pattern_index: 0,
                pattern: "fixture".into(), span: node(0).span,
                declared_node_type: WitStructureNodeType::Section,
                actual_node_type: WitStructureNodeType::Section,
                regex_captures: vec![], tags: vec![], mark: 0, marks: vec![], parsed_captures: vec![],
                body_mode: WitStructureBodyMode::Trigger, child_node_ids: vec![], entries: vec![],
                handler: None, metadata: vec![], declarations: vec![],
            },
        })
    }

    fn declare(value: &mut HookPayload) {
        let HookPayload::Structure(value) = value else { unreachable!() };
        value.candidate.declarations.push(wit::ParserDeclaration::DocumentFunction(wit::FunctionDeclaration {
            source: value.input.clone(), span: wit::TextRange { start: 0, end: 7 },
            scope: wit::FunctionDeclarationScope::Global, name: "fixture".into(), parameters: vec![],
            returns: wit::FunctionReturnDeclaration { class_name: None, single: true }, metadata: vec![],
        }));
    }

    fn updates(context: u64) -> HookEffects {
        let mut effects = crate::host::empty_effects();
        effects.context_updates.push(wit::ContextUpdate {
            syntax_context: context, key: "fixture.scope".into(), value: Some(b"body".to_vec()),
        });
        effects
    }

    fn accept(
        registry: &mut ReceiptRegistry,
        target: &mut AstNode,
        issuer: &str,
        mode: HookMode,
        reach: SemanticEffects,
    ) -> HookCall {
        let mut input = payload();
        if let HookPayload::Matching(value) = &mut input { value.metadata = target.metadata.clone(); }
        let mut writers = registry.begin(&input);
        let receipt = registry.accept(&mut writers, &mut input, issuer, mode, reach);
        if let HookPayload::Matching(value) = input { target.metadata = value.metadata; }
        HookCall { origin: HookCallOrigin::Executed, component_id: issuer.into(),
            subscription_id: "fixture.context".into(), semantic_receipt: receipt }
    }

    #[test]
    fn structure_context_classification_requires_only_current_entry_updates() {
        for case in 0..10 {
            let mut input = structure();
            let mut effects = updates(7);
            let mut phase = HookPhase::Structure;
            let mut context = 7;
            match case {
                0 => {},
                1 => effects.context_updates.clear(),
                2 => effects.context_updates.push(wit::ContextUpdate {
                    syntax_context: 99, key: "foreign".into(), value: None,
                }),
                3 => { let HookPayload::Structure(value) = &mut input else { unreachable!() };
                    value.timing = WitStructureTiming::ExitBody; },
                4 => phase = HookPhase::Effect,
                5 => input = payload(),
                6 => declare(&mut input),
                7 => context = 99,
                8 => effects.parse_requests.push(wit::ParseRequest {
                    request_id: 1, parser_id: "fixture".into(), input: "fixture".into(),
                    expected_types: vec![], span: node(0).span, options: vec![],
                }),
                _ => effects.parse_results.push(wit::ParseResult {
                    host_token: 1, request_id: 1, parser_id: "fixture".into(),
                    status: wit::ParseResultStatus::Success, roots: vec![], nodes: vec![], diagnostics: vec![],
                }),
            }
            let expected = match case {
                0 => SemanticEffects::StructureContext,
                1 => SemanticEffects::None,
                _ => SemanticEffects::Unbound,
            };
            assert_eq!(SemanticEffects::None.include_hook_effects(phase, context, &input, &effects), expected, "case {case}");
        }
    }

    #[test]
    fn structure_context_never_downgrades_unbound_continuations() {
        let input = structure();
        for effects in [updates(7), crate::host::empty_effects()] {
            // Additional parsing, shared/durable State and staged dynamic
            // changes already mark this candidate Unbound in native dispatch.
            assert_eq!(SemanticEffects::Unbound.include_hook_effects(HookPhase::Structure, 7, &input, &effects), SemanticEffects::Unbound);
        }
        assert_eq!(SemanticEffects::Local.include_hook_effects(HookPhase::Structure, 7, &input, &updates(7)), SemanticEffects::StructureContext);
        assert_eq!(SemanticEffects::StructureContext.include_hook_effects(HookPhase::Structure, 7, &input, &crate::host::empty_effects()), SemanticEffects::StructureContext);
    }

    #[test]
    fn structure_context_binding_waits_for_the_final_provider_body_shape() {
        for case in 0..6 {
            let mut registry = ReceiptRegistry::default();
            let call = accept(&mut registry, &mut node(0), "context", HookMode::Transform, SemanticEffects::StructureContext);
            let token = call.semantic_receipt.unwrap();
            assert!(registry.receipts[&token].unbound, "entry acceptance is provisional");
            let mut final_payload = structure();
            match case {
                0 => {},
                1 => { let HookPayload::Structure(value) = &mut final_payload else { unreachable!() };
                    value.candidate.body_mode = WitStructureBodyMode::None; },
                2 => declare(&mut final_payload),
                3 => { let HookPayload::Structure(value) = &mut final_payload else { unreachable!() };
                    value.timing = WitStructureTiming::ExitBody; },
                4 => { let HookPayload::Structure(value) = &mut final_payload else { unreachable!() };
                    value.candidate.actual_node_type = WitStructureNodeType::Simple; },
                _ => final_payload = payload(),
            }
            registry.bind_structure_context(&final_payload, &[token]);
            assert_eq!(registry.receipts[&token].unbound, case != 0, "final provider {case}");
        }
    }

    #[test]
    fn structure_context_keeps_transform_and_observe_authorship_inside_its_body() {
        for mode in [HookMode::Transform, HookMode::Observe] {
            let mut registry = ReceiptRegistry::default();
            let mut header = node(0); header.children = vec![1];
            let mut body = node(1); let mut other = node(2);
            let context = accept(&mut registry, &mut header, "context", mode, SemanticEffects::StructureContext);
            let token = context.semantic_receipt.unwrap();
            let body_call = write(&mut registry, &mut body, "core");
            let other_call = write(&mut registry, &mut other, "core");
            let tree = AstTree { roots: vec![0, 2], nodes: vec![header, body, other] };
            let calls = [context, body_call, other_call];
            assert!(!SemanticProofRegistry::capture(&tree, Some(&registry), &calls).view(&tree).provider_valid(&tree.nodes[2], "core"));
            registry.bind_structure_context(&structure(), &[token]);
            let proof = SemanticProofRegistry::capture(&tree, Some(&registry), &calls);
            let view = proof.view(&tree);
            assert!(view.provider_valid(&tree.nodes[2], "core"));
            assert!(view.unique_issuer(&tree.nodes[0]).is_none(), "whole-body authorship still includes the Core child");
            assert!(!view.provider_valid(&tree.nodes[0], "core"), "the scoped header remains a context author");
            assert!(view.unique_issuer(&tree.nodes[1]).is_none(), "context still contributes to its body");
            assert!(registry.receipts[&token].writers.contains("context"));
            assert_eq!(registry.receipts[&token].payload_writer, mode != HookMode::Observe);
        }
    }

    #[test]
    fn structure_context_binding_does_not_change_nested_or_unbound_receipts() {
        let mut registry = ReceiptRegistry::default();
        let nested = accept(&mut registry, &mut node(9), "nested", HookMode::Transform, SemanticEffects::StructureContext);
        let unbound = accept(&mut registry, &mut node(8), "shared", HookMode::Observe, SemanticEffects::Unbound);
        let nested_token = nested.semantic_receipt.unwrap();
        let unbound_token = unbound.semantic_receipt.unwrap();
        let mut input = payload();
        let mut writers = registry.begin(&input);
        registry.admit_nested(&mut writers, [&nested, &unbound]);
        let token = registry.accept(&mut writers, &mut input, "outer", HookMode::Transform, SemanticEffects::StructureContext).unwrap();
        let before = registry.receipts[&token].admitted_nested.clone();
        registry.bind_structure_context(&structure(), &[token, unbound_token]);
        assert!(!registry.receipts[&token].unbound);
        assert!(registry.receipts[&nested_token].unbound);
        assert!(registry.receipts[&unbound_token].unbound, "ordinary Unbound is never a candidate");
        assert_eq!(registry.receipts[&token].admitted_nested, before);
        assert!(registry.receipts[&token].writers.contains("nested"));
        assert!(registry.receipts[&token].writers.contains("shared"));
        let mut header = node(0);
        if let HookPayload::Matching(value) = input { header.metadata = value.metadata; }
        let mut other = node(1); let core = write(&mut registry, &mut other, "core");
        let outer = HookCall { origin: HookCallOrigin::Executed, component_id: "outer".into(),
            subscription_id: "fixture.context".into(), semantic_receipt: Some(token) };
        let tree = AstTree { roots: vec![0, 1], nodes: vec![header, other] };
        assert!(!SemanticProofRegistry::capture(&tree, Some(&registry), &[outer, core]).view(&tree).provider_valid(&tree.nodes[1], "core"));
    }

    #[test]
    fn unattached_structure_context_receipt_still_invalidates_other_roots() {
        let mut registry = ReceiptRegistry::default();
        let context = accept(&mut registry, &mut node(0), "context", HookMode::Transform, SemanticEffects::StructureContext);
        registry.bind_structure_context(&structure(), &[context.semantic_receipt.unwrap()]);
        let mut other = node(1); let core = write(&mut registry, &mut other, "core");
        let tree = AstTree { roots: vec![1], nodes: vec![other] };
        assert!(!SemanticProofRegistry::capture(&tree, Some(&registry), &[context, core]).view(&tree).provider_valid(&tree.nodes[0], "core"));
    }

    #[test]
    fn structure_context_keeps_prior_writer_dependencies_and_current_receipt_requirement() {
        let mut registry = ReceiptRegistry::default();
        let mut root = node(0);
        let prior = write(&mut registry, &mut root, "prior");
        let context = accept(&mut registry, &mut root, "context", HookMode::Transform, SemanticEffects::StructureContext);
        let token = context.semantic_receipt.unwrap();
        registry.bind_structure_context(&structure(), &[token]);
        assert!(registry.receipts[&token].parents.contains(&prior.semantic_receipt.unwrap()));
        assert!(registry.receipts[&token].writers.contains("prior"));
        assert!(root.metadata.iter().any(|entry| entry.key == RECEIPT && entry.value == token.to_string()));
        let tree = AstTree { roots: vec![0], nodes: vec![root] };
        let calls = [prior.clone(), context];
        assert!(SemanticProofRegistry::capture(&tree, Some(&registry), &calls).view(&tree).native_valid(&tree.nodes[0]));
        assert!(!SemanticProofRegistry::capture(&tree, Some(&ReceiptRegistry::default()), &calls).view(&tree).native_valid(&tree.nodes[0]));
        assert!(!SemanticProofRegistry::capture(&tree, Some(&registry), &calls[..1]).view(&tree).native_valid(&tree.nodes[0]), "a rolled-back current call cannot borrow its earlier parent");
    }
}

use std::collections::BTreeMap;

use super::*;
use crate::host::HostConfig;
use skript_parser::{
    ExpressionLeafParse, ExpressionLeafRequest, ExpressionParseEnvironment, ExtensionFault,
    PatternHookControl, PatternHookEvent, PatternMatchEnvironment, RawNodeId, StructureBody,
    StructureBodyMode, StructureCandidate, StructureDocumentNode, StructureExitDecision,
    StructureHookDecision, StructureHookRequest, StructurePreparationEvent, TypeExpressionOutcome,
    TypeExpressionRequest, parse_structures,
};
use syntaxes::{
    Catalog, CatalogParts, ClassName, DefinitionId, NodeType, Pattern, RegistrationId, Syntax,
};

fn catalog() -> Arc<Catalog> {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/type-parser-versions/skript-2.16.2-java25");
    Arc::new(ssg::load(fixture).unwrap().into_catalog())
}

fn host() -> ParserHost {
    ParserHost::new(
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../artifacts/core-library.wasm"
        )),
        HostConfig {
            syntax_catalog: Some(catalog()),
            ..HostConfig::default()
        },
    )
    .unwrap()
}

fn request(source: &str) -> DocumentParseRequest {
    DocumentParseRequest::new(
        "file:///workspace",
        "file:///workspace/preparation.sk",
        1,
        source.to_owned(),
    )
}

#[derive(Debug)]
struct Headers {
    source: MappedSource,
    tree: RawTree,
    candidates: Vec<(StructureCandidate, StructureBodyMode, bool)>,
}

fn capture_headers(event: StructurePreparationEvent<'_>, output: &mut Vec<Headers>) {
    match event {
        StructurePreparationEvent::Headers {
            source,
            tree,
            headers,
        } => {
            assert!(
                headers
                    .iter()
                    .all(|header| matches!(header.candidate.body, StructureBody::None))
            );
            output.push(Headers {
                source: source.clone(),
                tree: tree.clone(),
                candidates: headers
                    .into_iter()
                    .map(|header| (header.candidate.clone(), header.body_mode, header.ambiguous))
                    .collect(),
            });
        }
        StructurePreparationEvent::StructureHeader { .. }
        | StructurePreparationEvent::SectionHeader { .. } => {}
        StructurePreparationEvent::Invalidated { .. } => {
            panic!("valid document must not invalidate an accepted header")
        }
    }
}

fn assert_same_document(control: &DocumentParseResult, observed: &DocumentParseResult) {
    assert_eq!(control.execution_status, observed.execution_status);
    assert_eq!(control.source.original(), observed.source.original());
    assert_eq!(
        control.source.virtual_source(),
        observed.source.virtual_source()
    );
    assert_eq!(control.raw_tree, observed.raw_tree);
    // ExpressionEffects::eq intentionally compares Arc identity. Fresh hosts own
    // different speculative checkpoints; their Debug marker normalizes only
    // this opaque payload while retaining checkpoint presence and the complete
    // candidate/body/capture/source-mapping structure. Final effects, committed
    // state, Functions, projections, and hook calls are compared separately.
    assert_eq!(
        format!("{:?}", control.syntax),
        format!("{:?}", observed.syntax)
    );
    assert_eq!(control.functions, observed.functions);
    assert_eq!(control.state, observed.state);
    assert_eq!(control.text_macro_calls, observed.text_macro_calls);
    assert_eq!(control.tree_macro_calls, observed.tree_macro_calls);
    assert_eq!(control.ast_macro_calls, observed.ast_macro_calls);
    assert_eq!(control.syntax_calls, observed.syntax_calls);
    // WIT records implement Debug rather than PartialEq; their fields are ordered vectors.
    assert_eq!(format!("{:?}", control.ast), format!("{:?}", observed.ast));
    assert_eq!(
        format!("{:?}", control.effects),
        format!("{:?}", observed.effects)
    );
    assert_eq!(
        format!("{:?}", control.text_macro_decision),
        format!("{:?}", observed.text_macro_decision)
    );
    assert_eq!(
        format!("{:?}", control.tree_macro_decision),
        format!("{:?}", observed.tree_macro_decision)
    );
    assert_eq!(
        format!("{:?}", control.ast_macro_decision),
        format!("{:?}", observed.ast_macro_decision)
    );
    assert_eq!(
        format!("{:?}", control.fault_regions),
        format!("{:?}", observed.fault_regions)
    );
    assert_eq!(
        control.component_failures.len(),
        observed.component_failures.len()
    );
    for (before, after) in control
        .component_failures
        .iter()
        .zip(&observed.component_failures)
    {
        assert_eq!(before.component_id, after.component_id);
        assert_eq!(before.subscription_id, after.subscription_id);
        assert_eq!(
            format!("{:?}", before.affected_span),
            format!("{:?}", after.affected_span)
        );
        assert_eq!(before.error.to_string(), after.error.to_string());
    }
}

#[test]
fn observer_keeps_the_same_active_document_parse_and_hook_calls() {
    let source = "function observed(n: number) :: number:\n    return {_n}\non load:\n    send observed(1) to console\ncommand /observed:\n    trigger:\n        send 1 to console\n";
    let control = host()
        .parse_document(request(source), DocumentParserConfig::default())
        .unwrap();
    let mut preparations = Vec::new();
    let observed = host()
        .parse_document_with_preparation(
            request(source),
            DocumentParserConfig::default(),
            &mut |event| capture_headers(event, &mut preparations),
        )
        .unwrap();
    assert_same_document(&control, &observed);
    assert!(observed.component_failures.is_empty());
    assert!(
        observed.syntax.diagnostics.is_empty(),
        "{:#?}",
        observed.syntax.diagnostics
    );
    assert_eq!(observed.functions.registrations().len(), 1);
    assert!(
        !observed.syntax_calls.is_empty(),
        "actual semantic hooks must have run"
    );
    assert_eq!(
        preparations.len(),
        1,
        "the active parse prepares exactly once"
    );
    let prepared = &preparations[0];
    assert_eq!(prepared.source.original(), source);
    assert_eq!(prepared.tree, observed.raw_tree);
    assert_eq!(
        prepared
            .candidates
            .iter()
            .map(|(_, mode, _)| *mode)
            .collect::<Vec<_>>(),
        [
            StructureBodyMode::Trigger,
            StructureBodyMode::Trigger,
            StructureBodyMode::Entries
        ]
    );
    let selected = observed
        .syntax
        .roots
        .iter()
        .filter_map(|root| match root {
            StructureDocumentNode::Structure(matches) => matches.selected.as_ref(),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 3);
    for ((header, _, _), final_candidate) in prepared.candidates.iter().zip(selected) {
        assert_eq!(header.raw_node_id, final_candidate.raw_node_id);
        assert_eq!(
            header.matched.registration_id,
            final_candidate.matched.registration_id
        );
        assert_eq!(
            header.metadata, final_candidate.metadata,
            "preparation must follow actual Enter metadata"
        );
        assert!(
            !matches!(final_candidate.body, StructureBody::None),
            "body parsing must continue after observation"
        );
    }
}

#[test]
fn observer_receives_tree_macro_headers_and_original_call_site_mappings() {
    let source = "options:\n    name: prepared\n    message: send 1 to console\ncommand /{@name}:\n    trigger:\n        {@message}\n";
    let control = host()
        .parse_document(request(source), DocumentParserConfig::default())
        .unwrap();
    let mut preparations = Vec::new();
    let observed = host()
        .parse_document_with_preparation(
            request(source),
            DocumentParserConfig::default(),
            &mut |event| capture_headers(event, &mut preparations),
        )
        .unwrap();
    assert_same_document(&control, &observed);
    assert!(observed.component_failures.is_empty());
    assert!(
        observed
            .tree_macro_calls
            .iter()
            .any(|call| call.accepted && call.expansion.is_some())
    );
    assert_eq!(preparations.len(), 1);
    let prepared = &preparations[0];
    assert_eq!(prepared.source.original(), source);
    assert_eq!(
        prepared.source.virtual_source(),
        observed.source.virtual_source()
    );
    assert_eq!(prepared.tree, observed.raw_tree);
    let (header, mode, _) = prepared
        .candidates
        .iter()
        .find(|(candidate, _, _)| {
            prepared.tree.get(candidate.raw_node_id).unwrap().text == "command /prepared"
        })
        .expect("the observer must see the macro-expanded header");
    assert_eq!(*mode, StructureBodyMode::Entries);
    let mapped_header = &header.matched.matched.span.mapped;
    assert_eq!(
        mapped_header
            .virtual_range
            .slice(prepared.source.virtual_source()),
        Some("command /prepared")
    );
    let header_origin = mapped_header.primary_origin().unwrap();
    assert!(
        header_origin
            .original_range
            .slice(source)
            .unwrap()
            .contains("{@name}")
    );
    assert!(header_origin.expansion.is_some());
    let generated_effect = prepared
        .tree
        .nodes
        .iter()
        .find(|node| node.text == "send 1 to console")
        .expect("body preprocessing precedes Headers too");
    let origin = generated_effect.span.primary_origin().unwrap();
    assert_eq!(
        origin.original_range.slice(source),
        Some("        {@message}\n")
    );
    let backtrace = prepared
        .source
        .expansion_backtrace(origin.expansion.unwrap())
        .unwrap();
    assert_eq!(backtrace[0].component.as_str(), "nlaocs.core-library");
}

#[test]
fn observer_does_not_prepare_standard_headers_after_a_global_macro_fault() {
    let source = "on load:\n    # trap\n    send 1 to console\n";
    let make_host = || {
        let mut host = host();
        host.load_addon(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../artifacts/text-macro-addon.wasm"
        )))
        .unwrap();
        host
    };
    let control = make_host()
        .parse_document(request(source), DocumentParserConfig::default())
        .unwrap();
    let mut preparations = Vec::new();
    let observed = make_host()
        .parse_document_with_preparation(
            request(source),
            DocumentParserConfig::default(),
            &mut |event| capture_headers(event, &mut preparations),
        )
        .unwrap();
    assert_same_document(&control, &observed);
    assert_eq!(observed.execution_status, DocumentExecutionStatus::Partial);
    assert!(
        observed
            .fault_regions
            .iter()
            .any(|region| region.fault.code == "parser.extension-trap")
    );
    assert_eq!(preparations.len(), 1);
    assert!(preparations[0].candidates.is_empty());
    assert!(
        preparations[0]
            .tree
            .nodes
            .iter()
            .all(|node| node.kind == skript_parser::RawNodeKind::Invalid)
    );
    assert_eq!(preparations[0].source.original(), source);
    assert!(observed.syntax.roots.iter().all(|root| !matches!(root, StructureDocumentNode::Structure(matches) if matches.selected.is_some())));
}

fn alternative_catalog() -> Catalog {
    let source = catalog();
    let mut first = source.structures().next().unwrap().clone();
    first.common.definition_id = DefinitionId("test:observer:first".into());
    first.common.registration_id = RegistrationId("test:observer:first:0".into());
    first.common.registration_order = 0;
    first.common.element_class = ClassName("test.ObserverStructure".into());
    first.common.patterns = vec![Pattern {
        source: "observer root".into(),
        parsed: syntax_pattern_parser::syntax::parse("observer root", source.plural_rules())
            .unwrap(),
    }];
    first.node_type = Some(NodeType::Section);
    first.entry_validator = None;
    let mut fallback = first.clone();
    fallback.common.definition_id = DefinitionId("test:observer:fallback".into());
    fallback.common.registration_id = RegistrationId("test:observer:fallback:0".into());
    fallback.common.registration_order = 1;
    Catalog::new(CatalogParts {
        syntaxes: vec![Syntax::Structure(first), Syntax::Structure(fallback)],
        converters: Vec::new(),
        comparators: Vec::new(),
        event_values: Vec::new(),
        properties: Vec::new(),
        operators: Vec::new(),
        operations: BTreeMap::new(),
        differences: Vec::new(),
        classes: Vec::new(),
        aliases: source.aliases().clone(),
        plural_rules: source.plural_rules().clone(),
        language: BTreeMap::new(),
    })
}

#[derive(Clone, Copy)]
enum ExitOutcome {
    RejectFirst,
    FaultFirst,
}

struct LifecycleEnvironment {
    observe: bool,
    outcome: ExitOutcome,
    pending_fault: Option<ExtensionFault>,
    hooks: Vec<String>,
    events: Vec<String>,
    invalidated: Vec<(RawNodeId, String, usize)>,
    prepared: Vec<(RawNodeId, String, usize, StructureBodyMode, bool)>,
    faults: Vec<ExtensionFault>,
}

impl LifecycleEnvironment {
    fn new(observe: bool, outcome: ExitOutcome) -> Self {
        Self {
            observe,
            outcome,
            pending_fault: None,
            hooks: Vec::new(),
            events: Vec::new(),
            invalidated: Vec::new(),
            prepared: Vec::new(),
            faults: Vec::new(),
        }
    }

    fn hook(&mut self, phase: &str, candidate: &StructureCandidate) {
        let event = format!("{phase}:{}", candidate.matched.registration_id);
        self.hooks.push(event.clone());
        self.events.push(event);
    }
}

impl PatternMatchEnvironment for LifecycleEnvironment {
    fn resolve_type(
        &mut self,
        _: TypeExpressionRequest<'_>,
    ) -> Result<TypeExpressionOutcome, String> {
        Ok(TypeExpressionOutcome::default())
    }
    fn dispatch_hook(&mut self, _: PatternHookEvent<'_>) -> Result<PatternHookControl, String> {
        Ok(PatternHookControl::Continue)
    }
    fn take_extension_fault(&mut self) -> Option<ExtensionFault> {
        self.pending_fault.take()
    }
}

impl ExpressionParseEnvironment for LifecycleEnvironment {
    fn state_revision(&self) -> Result<u64, String> {
        Ok(0)
    }
    fn parse_expression_leaf(
        &mut self,
        _: ExpressionLeafRequest<'_>,
    ) -> Result<ExpressionLeafParse, String> {
        Ok(ExpressionLeafParse::default())
    }
    fn enter_structure(
        &mut self,
        request: StructureHookRequest<'_>,
    ) -> Result<StructureHookDecision, String> {
        self.hook("enter", request.candidate);
        assert!(matches!(request.candidate.body, StructureBody::None));
        Ok(StructureHookDecision::Accept {
            context: request.context.clone(),
            body_mode: StructureBodyMode::Raw,
            entry_body_modes: BTreeMap::new(),
            metadata: request.candidate.metadata.clone(),
        })
    }
    fn exit_structure(
        &mut self,
        request: StructureHookRequest<'_>,
    ) -> Result<StructureExitDecision, String> {
        self.hook("exit", request.candidate);
        assert!(
            matches!(&request.candidate.body, StructureBody::Raw(children) if children.len() == 1)
        );
        if request.candidate.matched.registration_id != "test:observer:first:0" {
            return Ok(StructureExitDecision::Accept);
        }
        match self.outcome {
            ExitOutcome::RejectFirst => Ok(StructureExitDecision::Reject {
                reason: "rejected after body evaluation".into(),
                diagnostics: Vec::new(),
            }),
            ExitOutcome::FaultFirst => {
                self.pending_fault = Some(ExtensionFault {
                    component_id: "test.observer".into(),
                    subscription_id: "exit".into(),
                    phase: "Structure".into(),
                    target: "test:observer:first:0".into(),
                    code: "parser.extension-trap".into(),
                    message: "injected Exit fault".into(),
                });
                Err("injected Exit fault".into())
            }
        }
    }
    fn discard_structure_candidate(
        &mut self,
        candidate: &StructureCandidate,
    ) -> Result<(), String> {
        self.hook("discard", candidate);
        Ok(())
    }
    fn observe_extension_fault(&mut self, fault: &ExtensionFault, _: skript_parser::MappedSpan) {
        self.faults.push(fault.clone());
        self.events.push("fault".into());
    }
    fn structure_preparation(&mut self, event: StructurePreparationEvent<'_>) {
        if !self.observe {
            return;
        }
        match event {
            StructurePreparationEvent::Headers {
                source,
                tree,
                headers,
            } => {
                self.events.push("headers".into());
                assert_eq!(source.original(), "observer root:\n    child\n");
                assert_eq!(tree.roots.len(), 1);
                for header in headers {
                    assert!(matches!(header.candidate.body, StructureBody::None));
                    self.prepared.push((
                        header.candidate.raw_node_id,
                        header.candidate.matched.registration_id.clone(),
                        header.candidate.matched.pattern_index,
                        header.body_mode,
                        header.ambiguous,
                    ));
                }
            }
            StructurePreparationEvent::Invalidated {
                raw_node_id,
                registration_id,
                pattern_index,
                ..
            } => {
                self.events.push(format!("invalidated:{registration_id}"));
                self.invalidated
                    .push((raw_node_id, registration_id.to_owned(), pattern_index));
            }
            StructurePreparationEvent::StructureHeader { .. }
            | StructurePreparationEvent::SectionHeader { .. } => {}
        }
    }
}

fn native_parse(observe: bool, outcome: ExitOutcome) -> (StructureDocument, LifecycleEnvironment) {
    let source = MappedSource::identity("observer root:\n    child\n");
    let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 16));
    let mut environment = LifecycleEnvironment::new(observe, outcome);
    let result = parse_structures(
        &alternative_catalog(),
        StructureParseRequest {
            source: &source,
            tree: &tree,
            context: ExpressionParseContext::default(),
        },
        &mut environment,
        StructureParserConfig::default(),
    )
    .unwrap();
    (result, environment)
}

#[test]
fn exit_reject_invalidates_preparation_before_retrying_the_fallback() {
    let (control, without) = native_parse(false, ExitOutcome::RejectFirst);
    let (observed, with) = native_parse(true, ExitOutcome::RejectFirst);
    assert_eq!(control, observed);
    assert_eq!(
        without.hooks, with.hooks,
        "observation must not rerun semantic hooks"
    );
    assert_eq!(
        with.events,
        [
            "enter:test:observer:first:0",
            "enter:test:observer:fallback:0",
            "headers",
            "exit:test:observer:first:0",
            "invalidated:test:observer:first:0",
            "discard:test:observer:first:0",
            "enter:test:observer:fallback:0",
            "exit:test:observer:fallback:0"
        ]
    );
    assert_eq!(with.prepared.len(), 1);
    let (node, registration, pattern, mode, ambiguous) = &with.prepared[0];
    assert_eq!(*mode, StructureBodyMode::Raw);
    assert!(*ambiguous);
    assert_eq!(with.invalidated, [(*node, registration.clone(), *pattern)]);
    let StructureDocumentNode::Structure(matches) = &observed.roots[0] else {
        panic!("Structure root")
    };
    assert_eq!(
        matches.selected.as_ref().unwrap().matched.registration_id,
        "test:observer:fallback:0"
    );
    assert!(matches.unknown.is_none());
}

#[test]
fn exit_fault_invalidates_preparation_without_adopting_a_fallback() {
    let (control, without) = native_parse(false, ExitOutcome::FaultFirst);
    let (observed, with) = native_parse(true, ExitOutcome::FaultFirst);
    assert_eq!(control, observed);
    assert_eq!(without.hooks, with.hooks);
    assert_eq!(without.faults, with.faults);
    assert_eq!(
        with.events,
        [
            "enter:test:observer:first:0",
            "enter:test:observer:fallback:0",
            "headers",
            "exit:test:observer:first:0",
            "invalidated:test:observer:first:0",
            "discard:test:observer:first:0",
            "fault"
        ]
    );
    assert_eq!(with.prepared.len(), 1);
    let (node, registration, pattern, _, _) = &with.prepared[0];
    assert_eq!(with.invalidated, [(*node, registration.clone(), *pattern)]);
    let StructureDocumentNode::Structure(matches) = &observed.roots[0] else {
        panic!("Structure root")
    };
    assert!(matches.selected.is_none());
    assert!(matches.alternatives.is_empty());
    assert!(
        matches.unknown.is_none(),
        "a fault is not ordinary syntax rejection"
    );
    assert_eq!(with.faults.len(), 1);
    assert_eq!(with.faults[0].code, "parser.extension-trap");
    assert!(observed.diagnostics.iter().any(
        |diagnostic| diagnostic.kind == skript_parser::StructureDiagnosticKind::ExtensionFault
    ));
}

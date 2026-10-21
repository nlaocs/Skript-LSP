//! Structure Enter reuse is a separate explicit provider promise. No old tree,
//! state delta, accepted receipt, declaration registration or context ID survives.
use super::*;

pub(super) fn eligible(entry: &ComponentEntry, invocation: &HookInvocation) -> bool {
    let HookPayload::Structure(value) = &invocation.payload else {
        return false;
    };
    if invocation.phase != HookPhase::Structure
        || value.timing != WitStructureTiming::EnterBody
        || !value.context.section_stack.is_empty()
        || !value.candidate.parsed_captures.is_empty()
        || !value.candidate.child_node_ids.is_empty()
        || !value.candidate.entries.is_empty()
        || !value.candidate.declarations.is_empty()
        || !component_promises(
            &entry.manifest.capabilities,
            crate::CAPABILITY_INCREMENTAL_STRUCTURE_HEADER,
        )
    {
        return false;
    }
    entry
        .manifest
        .registered_syntax_handlers
        .iter()
        .any(|handler| {
            entry
                .manifest
                .incremental_header_handlers
                .contains(&handler.handler_id)
                && handler.kind == SyntaxKind::Structure
                && handler.phase == HookPhase::Structure
                && (handler.pattern_indices.is_empty()
                    || handler
                        .pattern_indices
                        .contains(&value.candidate.pattern_index))
                && (handler.pattern_sources.is_empty()
                    || handler.pattern_sources.contains(&value.candidate.pattern))
                && (handler.marks.is_empty() || handler.marks.contains(&value.candidate.mark))
                && handler.required_tags.iter().all(|required| {
                    value
                        .candidate
                        .tags
                        .iter()
                        .any(|tag| &tag.value == required)
                })
                && !handler.forbidden_tags.iter().any(|forbidden| {
                    value
                        .candidate
                        .tags
                        .iter()
                        .any(|tag| &tag.value == forbidden)
                })
                && registered_handler_target_matches(
                    entry,
                    handler,
                    &value.candidate.definition_id,
                    &value.candidate.registration_id,
                    value.candidate.handler.as_deref(),
                )
        })
}

fn relative_span(span: &mut MappedSpan, base: u64, end: u64) -> Option<()> {
    let [origin] = span.origins.as_mut_slice() else {
        return None;
    };
    if origin.kind != WitOriginKind::Exact
        || origin.expansion.is_some()
        || origin.original_range.start != span.virtual_range.start
        || origin.original_range.end != span.virtual_range.end
        || span.virtual_range.start < base
        || span.virtual_range.end > end
        || span.virtual_range.start > span.virtual_range.end
    {
        return None;
    }
    span.virtual_range.start -= base;
    span.virtual_range.end -= base;
    origin.original_range = span.virtual_range.clone();
    Some(())
}

/// The immutable Catalog/runtime are host-lifetime facts. The world key includes
/// ordered components/bindings, dynamic definitions and current availability.
/// Native matching, captures and source mapping still execute for this revision.
pub(super) fn key_payload(value: &WitStructurePayload) -> Option<String> {
    let mut candidate = value.candidate.clone();
    let base = candidate.span.virtual_range.start;
    let end = candidate.span.virtual_range.end;
    relative_span(&mut candidate.span, base, end)?;
    for tag in &mut candidate.tags {
        relative_span(&mut tag.input_span, base, end)?;
    }
    for mark in &mut candidate.marks {
        relative_span(&mut mark.input_span, base, end)?;
    }
    candidate.raw_node_id = 0;
    let mut context = value.context.clone();
    context.syntax_context = 0;
    Some(format!("{context:?}|{candidate:?}"))
}

/// Check immutable echoes before stripping them. A malformed response must never
/// be repaired into an apparently valid cached result.
pub(super) fn normalize_output(original: &HookPayload, output: &mut HookOutput) -> bool {
    let HookPayload::Structure(input) = original else {
        return true;
    };
    if output
        .effects
        .context_updates
        .iter()
        .any(|update| update.syntax_context != input.context.syntax_context)
    {
        return false;
    }
    let Some(HookPayload::Structure(value)) = output.replacement.as_mut() else {
        return output.replacement.is_none() && output.effects.context_updates.is_empty();
    };
    if !same_structure_payload_identity(value, input) {
        return false;
    }
    let base = input.candidate.span.virtual_range.start;
    let end = input.candidate.span.virtual_range.end;
    for declaration in &mut value.candidate.declarations {
        let WitParserDeclaration::DocumentFunction(declaration) = declaration;
        if declaration.source != input.input
            || declaration.span.start < base
            || declaration.span.end > end
            || declaration.span.start > declaration.span.end
            || usize::try_from(declaration.span.start)
                .ok()
                .zip(usize::try_from(declaration.span.end).ok())
                .and_then(|(start, end)| input.input.get(start..end))
                .is_none()
        {
            return false;
        }
        declaration.source.clear();
        declaration.span.start -= base;
        declaration.span.end -= base;
    }
    value.input.clear();
    value.body_tree.roots.clear();
    value.body_tree.nodes.clear();
    value.body_tree.diagnostics.clear();
    value.body_tree.indentation = None;
    value.type_options.clear();
    // Only the mutable semantic delta is retained. All immutable fields are
    // rebuilt from the current invocation before ordinary output validation.
    value.candidate.raw_node_id = 0;
    value.context.syntax_context = 0;
    for update in &mut output.effects.context_updates {
        update.syntax_context = 0;
    }
    true
}

pub(super) fn restore_output(original: &HookPayload, output: &mut HookOutput) -> bool {
    let HookPayload::Structure(input) = original else {
        return true;
    };
    if output
        .effects
        .context_updates
        .iter()
        .any(|update| update.syntax_context != 0)
    {
        return false;
    }
    if let Some(HookPayload::Structure(cached)) = output.replacement.as_mut() {
        let mut current = input.clone();
        current.candidate.body_mode = cached.candidate.body_mode;
        current
            .candidate
            .metadata
            .clone_from(&cached.candidate.metadata);
        current
            .candidate
            .declarations
            .clone_from(&cached.candidate.declarations);
        let base = input.candidate.span.virtual_range.start;
        let end = input.candidate.span.virtual_range.end;
        for declaration in &mut current.candidate.declarations {
            let WitParserDeclaration::DocumentFunction(declaration) = declaration;
            let Some(start) = base.checked_add(declaration.span.start) else {
                return false;
            };
            let Some(finish) = base.checked_add(declaration.span.end) else {
                return false;
            };
            if start > finish
                || finish > end
                || usize::try_from(start)
                    .ok()
                    .zip(usize::try_from(finish).ok())
                    .and_then(|(start, end)| input.input.get(start..end))
                    .is_none()
            {
                return false;
            }
            declaration.source.clone_from(&input.input);
            declaration.span = WitTextRange { start, end: finish };
        }
        *cached = current;
    } else if output.replacement.is_some() {
        return false;
    }
    for update in &mut output.effects.context_updates {
        update.syntax_context = input.context.syntax_context;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::nlaocs::skript_parser_addon::types::{
        ContextUpdate, FunctionReturnDeclaration, RawTree, RegisteredSyntaxHandler, SourceOrigin,
    };
    use crate::state::{NamespaceVisibility, StateEncoding, StateScope, StateValue};
    const DOCUMENT: &str = "file:///workspace/header.sk";
    const HANDLER: &str = "fixture.header-local";
    fn host() -> ParserHost {
        let mut host = ParserHost::new(
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../artifacts/core-library.wasm"
            )),
            HostConfig {
                runtime_profile: RuntimeProfile {
                    skript_version: Some("2.15.4".into()),
                    ..RuntimeProfile::default()
                },
                ..HostConfig::default()
            },
        )
        .unwrap();
        let manifest = &mut host.components[0].manifest;
        manifest.incremental_header_handlers = vec![HANDLER.into()];
        manifest
            .registered_syntax_handlers
            .push(RegisteredSyntaxHandler {
                handler_id: HANDLER.into(),
                kind: SyntaxKind::Structure,
                phase: HookPhase::Structure,
                targets: vec![RegisteredSyntaxHandlerTarget::DynamicHandler(
                    HANDLER.into(),
                )],
                pattern_indices: Vec::new(),
                pattern_sources: Vec::new(),
                required_tags: Vec::new(),
                forbidden_tags: Vec::new(),
                marks: Vec::new(),
                capture_parsers: Vec::new(),
                context_requirements: Vec::new(),
            });
        host
    }
    fn invocation(source: &str, revision: u64, root: u64, context: u64) -> HookInvocation {
        let start = source.find("function").unwrap();
        let end = start + source[start..].find('\n').unwrap() - 1;
        let span = MappedSpan {
            virtual_range: WitTextRange {
                start: start as u64,
                end: end as u64,
            },
            origins: vec![SourceOrigin {
                original_range: WitTextRange {
                    start: start as u64,
                    end: end as u64,
                },
                kind: WitOriginKind::Exact,
                expansion: None,
            }],
        };
        HookInvocation {
            context: InvocationContext {
                invocation_id: revision * 7,
                subscription_id: "fixture.structure".into(),
                document_id: DOCUMENT.into(),
                document_revision: revision,
                expansion: None,
                syntax_context: context,
            },
            phase: HookPhase::Structure,
            target: HookTarget::SyntaxKind(SyntaxKind::Structure),
            parse_results: Vec::new(),
            payload: HookPayload::Structure(WitStructurePayload {
                input: source.into(),
                body_tree: RawTree {
                    roots: vec![root],
                    nodes: Vec::new(),
                    diagnostics: Vec::new(),
                    indentation: None,
                },
                context: WitParseContext {
                    syntax_context: context,
                    event_classes: Vec::new(),
                    section_stack: Vec::new(),
                    values: Vec::new(),
                },
                timing: WitStructureTiming::EnterBody,
                type_options: Vec::new(),
                candidate:
                    crate::bindings::nlaocs::skript_parser_addon::types::StructureCandidate {
                        raw_node_id: root,
                        definition_id: "fixture.fn".into(),
                        registration_id: "fixture.fn:1".into(),
                        element_class: None,
                        priority: 0,
                        registration_order: 0,
                        pattern_index: 0,
                        pattern: "function .+".into(),
                        span,
                        declared_node_type: WitStructureNodeType::Section,
                        actual_node_type: WitStructureNodeType::Section,
                        regex_captures: vec!["sample() :: number".into()],
                        tags: Vec::new(),
                        mark: 0,
                        marks: Vec::new(),
                        parsed_captures: Vec::new(),
                        body_mode: WitStructureBodyMode::Raw,
                        child_node_ids: Vec::new(),
                        entries: Vec::new(),
                        handler: Some(HANDLER.into()),
                        metadata: Vec::new(),
                        declarations: Vec::new(),
                    },
            }),
        }
    }
    fn output(call: &HookInvocation) -> HookOutput {
        let HookPayload::Structure(mut value) = call.payload.clone() else {
            unreachable!()
        };
        value.candidate.body_mode = WitStructureBodyMode::Trigger;
        value
            .candidate
            .declarations
            .push(WitParserDeclaration::DocumentFunction(
                WitFunctionDeclaration {
                    source: value.input.clone(),
                    span: value.candidate.span.virtual_range.clone(),
                    scope: WitFunctionDeclarationScope::Global,
                    name: "sample".into(),
                    parameters: Vec::new(),
                    returns: FunctionReturnDeclaration {
                        class_name: Some("java.lang.Number".into()),
                        single: true,
                    },
                    metadata: vec![WitMetadataEntry {
                        key: "fixture.name.offset".into(),
                        value: "9".into(),
                        owner_component_id: None,
                    }],
                },
            ));
        HookOutput {
            decision: HookDecision::ContinueProcessing,
            replacement: Some(HookPayload::Structure(value)),
            effects: HookEffects {
                diagnostics: Vec::new(),
                context_updates: vec![ContextUpdate {
                    syntax_context: call.context.syntax_context,
                    key: "fixture.scope".into(),
                    value: Some(b"ready".to_vec()),
                }],
                parse_requests: Vec::new(),
                parse_results: Vec::new(),
            },
        }
    }
    fn begin(host: &mut ParserHost, source: &str, revision: u64) -> ParseTransaction {
        let parse = host
            .begin_parse("file:///workspace", DOCUMENT, revision)
            .unwrap();
        host.begin_incremental_document(&parse, Arc::from(source))
            .unwrap();
        parse
    }
    fn record(
        host: &mut ParserHost,
        parse: &ParseTransaction,
        call: &mut HookInvocation,
    ) -> String {
        let key = host.incremental_key(0, call).unwrap().unwrap();
        let before = parse.observation_checkpoint().unwrap();
        let mut state = parse
            .begin_invocation(host.components[0].manifest.component_id.clone())
            .unwrap();
        state.observe_reads();
        host.incremental_record(
            key.clone(),
            &before,
            &state,
            &call.payload,
            &output(call),
            false,
        )
        .unwrap();
        state.rollback();
        key
    }
    #[test]
    fn header_reuse_rebinds_utf8_shifted_source_ids_context_and_declarations() {
        let mut host = host();
        let first = "function sample() :: number:\n    return 1\n";
        let second =
            "# \u{65e5}\u{672c}\u{8a9e}\u{1f98a}\n\nfunction sample() :: number:\n    return 2\n";
        let parse = begin(&mut host, first, 1);
        let mut old = invocation(first, 1, 3, 7);
        let key = record(&mut host, &parse, &mut old);
        parse.commit().unwrap();
        host.finish_incremental_document(DOCUMENT, 1, true);
        let parse = begin(&mut host, second, 2);
        let mut current = invocation(second, 2, 19, 80);
        let current_key = host.incremental_key(0, &mut current).unwrap().unwrap();
        assert_eq!(key, current_key);
        let mut state = parse
            .begin_invocation(host.components[0].manifest.component_id.clone())
            .unwrap();
        state.observe_reads();
        let hit = host
            .incremental_lookup(&current_key, &mut state, &current.payload)
            .unwrap()
            .unwrap();
        let Some(HookPayload::Structure(value)) = hit.replacement else {
            panic!("header replacement")
        };
        let HookPayload::Structure(expected) = current.payload else {
            unreachable!()
        };
        assert!(same_structure_payload_identity(&value, &expected));
        assert_eq!(value.candidate.body_mode, WitStructureBodyMode::Trigger);
        assert_eq!(value.context.syntax_context, 80);
        assert_eq!(value.candidate.raw_node_id, 19);
        assert_eq!(hit.effects.context_updates[0].syntax_context, 80);
        let WitParserDeclaration::DocumentFunction(declaration) = &value.candidate.declarations[0];
        assert_eq!(declaration.source, second);
        assert!(same_wit_range(
            &declaration.span,
            &expected.candidate.span.virtual_range
        ));
        assert_eq!(declaration.metadata[0].value, "9");
        assert_eq!(host.incremental_metrics().header_hits, 1);
        state.rollback();
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 2, false);
    }
    #[test]
    fn header_key_preserves_header_context_and_opt_in_dependencies() {
        let mut host = host();
        let source = "function sample() :: number:\n    return 1\n";
        let parse = begin(&mut host, source, 1);
        let mut call = invocation(source, 1, 3, 7);
        let key = host.incremental_key(0, &mut call).unwrap().unwrap();
        let mut changed = call.clone();
        if let HookPayload::Structure(value) = &mut changed.payload {
            value.context.event_classes.push("other.Event".into());
        }
        assert_ne!(key, host.incremental_key(0, &mut changed).unwrap().unwrap());
        let mut local = call.clone();
        if let HookPayload::Structure(value) = &mut local.payload {
            value.input = "function sample() :: number".into();
        }
        assert!(host.incremental_key(0, &mut local).unwrap().is_none());
        let mut changed = call.clone();
        if let HookPayload::Structure(value) = &mut changed.payload {
            value.candidate.span.origins[0].kind = WitOriginKind::Replaced;
        }
        assert!(host.incremental_key(0, &mut changed).unwrap().is_none());
        host.components[0]
            .manifest
            .incremental_header_handlers
            .clear();
        assert!(host.incremental_key(0, &mut call).unwrap().is_none());
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 1, false);
    }
    #[test]
    fn header_state_mutation_and_invalid_immutable_echo_never_publish() {
        let mut host = host();
        let source = "function sample() :: number:\n    return 1\n";
        let parse = begin(&mut host, source, 1);
        let mut call = invocation(source, 1, 3, 7);
        let key = host.incremental_key(0, &mut call).unwrap().unwrap();
        let before = parse.observation_checkpoint().unwrap();
        let mut state = parse
            .begin_invocation(host.components[0].manifest.component_id.clone())
            .unwrap();
        state.observe_reads();
        let mut wrong = output(&call);
        if let Some(HookPayload::Structure(value)) = &mut wrong.replacement {
            value.input = "wrong".into();
        }
        host.incremental_record(key.clone(), &before, &state, &call.payload, &wrong, false)
            .unwrap();
        assert!(host.incremental.active.as_ref().unwrap().pending.is_empty());
        state
            .put(
                StateScope::Parse,
                NamespaceVisibility::Private,
                "commands",
                "fixture",
                StateValue {
                    schema_id: "nlaocs.core-library.commands".into(),
                    encoding: StateEncoding::Raw,
                    bytes: vec![1],
                },
            )
            .unwrap();
        host.incremental_record(key, &before, &state, &call.payload, &output(&call), false)
            .unwrap();
        assert!(host.incremental.active.as_ref().unwrap().pending.is_empty());
        state.rollback();
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 1, false);
    }
    #[test]
    fn cancelled_header_parse_never_replaces_the_last_committed_cache() {
        let mut host = host();
        let source = "function sample() :: number:\n    return 1\n";
        let parse = begin(&mut host, source, 1);
        let mut call = invocation(source, 1, 3, 7);
        record(&mut host, &parse, &mut call);
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 1, false);
        assert!(!host.incremental.documents.contains_key(DOCUMENT));
        let parse = begin(&mut host, source, 2);
        let mut call = invocation(source, 2, 9, 12);
        record(&mut host, &parse, &mut call);
        parse.commit().unwrap();
        host.finish_incremental_document(DOCUMENT, 2, true);
        let parse = begin(&mut host, source, 3);
        let mut call = invocation(source, 3, 16, 31);
        record(&mut host, &parse, &mut call);
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 3, false);
        assert_eq!(host.incremental.documents[DOCUMENT].revision, 2);
        let parse = begin(&mut host, source, 4);
        let mut call = invocation(source, 4, 20, 60);
        let key = host.incremental_key(0, &mut call).unwrap().unwrap();
        let mut state = parse
            .begin_invocation(host.components[0].manifest.component_id.clone())
            .unwrap();
        state.observe_reads();
        assert!(
            host.incremental_lookup(&key, &mut state, &call.payload)
                .unwrap()
                .is_some()
        );
        state.rollback();
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 4, false);
    }
    #[test]
    fn header_observed_state_reads_invalidate_without_adopting_stale_reads() {
        let mut host = host();
        let source = "function sample() :: number:\n    return 1\n";
        let parse = begin(&mut host, source, 1);
        let mut call = invocation(source, 1, 3, 7);
        let key = host.incremental_key(0, &mut call).unwrap().unwrap();
        let before = parse.observation_checkpoint().unwrap();
        let owner = host.components[0].manifest.component_id.clone();
        let mut state = parse.begin_invocation(owner.clone()).unwrap();
        state.observe_reads();
        assert!(
            state
                .get(
                    StateScope::Parse,
                    NamespaceVisibility::Private,
                    "commands",
                    "absent"
                )
                .unwrap()
                .is_none()
        );
        host.incremental_record(
            key.clone(),
            &before,
            &state,
            &call.payload,
            &output(&call),
            false,
        )
        .unwrap();
        state.rollback();
        parse.commit().unwrap();
        host.finish_incremental_document(DOCUMENT, 1, true);
        let parse = begin(&mut host, source, 2);
        let mut call = invocation(source, 2, 12, 21);
        let key2 = host.incremental_key(0, &mut call).unwrap().unwrap();
        assert_eq!(key, key2);
        let mut writer = parse.begin_invocation(owner.clone()).unwrap();
        writer
            .put(
                StateScope::Parse,
                NamespaceVisibility::Private,
                "commands",
                "absent",
                StateValue::new("nlaocs.core-library.commands", StateEncoding::Raw, vec![1]),
            )
            .unwrap();
        writer.commit().unwrap();
        let mut state = parse.begin_invocation(owner).unwrap();
        state.observe_reads();
        assert!(
            host.incremental_lookup(&key2, &mut state, &call.payload)
                .unwrap()
                .is_none()
        );
        assert_eq!(host.incremental_metrics().dependency_misses, 1);
        assert!(state.read_write_set().reads.is_empty());
        state.rollback();
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 2, false);
    }
    #[test]
    fn header_component_and_dynamic_world_changes_invalidate() {
        for dynamic in [false, true] {
            let mut host = host();
            if dynamic {
                // Install a real registry before revision 1: both snapshots have
                // the same static catalog, and only revision 2 gains a definition.
                let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/data/type-parser-versions/skript-2.16.2-java25");
                let catalog = Arc::new(ssg::load(fixture).unwrap().into_catalog());
                host.dynamic_syntax_registry = Some(DynamicSyntaxRegistry::new(catalog));
            }
            let source = "function sample() :: number:\n    return 1\n";
            let parse = begin(&mut host, source, 1);
            let mut call = invocation(source, 1, 3, 7);
            let key = record(&mut host, &parse, &mut call);
            parse.commit().unwrap();
            host.finish_incremental_document(DOCUMENT, 1, true);
            if !dynamic {
                host.components[0]
                    .manifest
                    .component_version
                    .push_str(".changed");
            }
            let parse = begin(&mut host, source, 2);
            if dynamic {
                let registry = host.dynamic_syntax_registry.as_ref().unwrap();
                let mut update = registry
                    .begin_document_update(
                        host.components[0].manifest.component_id.clone(),
                        0,
                        DOCUMENT,
                        2,
                    )
                    .unwrap();
                update
                    .register(DynamicSyntaxInput {
                        local_id: "fixture-extra".into(),
                        kind: CatalogSyntaxKind::Effect,
                        patterns: vec!["fixture extra".into()],
                        priority: 0,
                        before: Vec::new(),
                        after: Vec::new(),
                        return_type: None,
                        return_multiplicity: None,
                        structure_node_type: None,
                        structure_body_mode: None,
                        entry_validator: None,
                        handler: "fixture.extra".into(),
                        metadata: BTreeMap::new(),
                    })
                    .unwrap();
                update.commit().unwrap();
            }
            let mut call = invocation(source, 2, 12, 21);
            let key2 = host.incremental_key(0, &mut call).unwrap().unwrap();
            assert_eq!(key, key2);
            let mut state = parse
                .begin_invocation(host.components[0].manifest.component_id.clone())
                .unwrap();
            state.observe_reads();
            assert!(
                host.incremental_lookup(&key2, &mut state, &call.payload)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(host.incremental_metrics().header_hits, 0);
            state.rollback();
            parse.cancel().unwrap();
            host.finish_incremental_document(DOCUMENT, 2, false);
        }
    }
    #[test]
    fn header_manifest_requires_explicit_capability_and_valid_unique_owner() {
        let mut host = host();
        validate_manifest(&host.components[0].manifest, &host_capabilities()).unwrap();
        let mut manifest = host.components[0].manifest.clone();
        manifest
            .capabilities
            .retain(|capability| capability.id != crate::CAPABILITY_INCREMENTAL_STRUCTURE_HEADER);
        assert!(validate_manifest(&manifest, &host_capabilities()).is_err());
        manifest = host.components[0].manifest.clone();
        manifest.incremental_header_handlers.push(HANDLER.into());
        assert!(validate_manifest(&manifest, &host_capabilities()).is_err());
        manifest = host.components[0].manifest.clone();
        manifest.incremental_header_handlers = vec!["missing".into()];
        assert!(validate_manifest(&manifest, &host_capabilities()).is_err());
        host.components[0]
            .manifest
            .incremental_local_subscriptions
            .clear();
        host.components[0]
            .manifest
            .capabilities
            .retain(|capability| capability.id != crate::CAPABILITY_INCREMENTAL_LOCAL_SYNTAX);
        let source = "function sample() :: number:\n    return 1\n";
        let parse = begin(&mut host, source, 1);
        let mut call = invocation(source, 1, 3, 7);
        assert!(
            host.incremental_key(0, &mut call).unwrap().is_some(),
            "new contract does not require or widen local-syntax v1"
        );
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 1, false);
    }

    #[test]
    fn header_opt_in_exact_registration_does_not_include_definition_siblings() {
        use crate::bindings::nlaocs::skript_parser_addon::types::RegisteredHandlerBinding;
        let mut host = host();
        let handler = host.components[0]
            .manifest
            .registered_syntax_handlers
            .iter_mut()
            .find(|handler| handler.handler_id == HANDLER)
            .unwrap();
        handler.targets = vec![RegisteredSyntaxHandlerTarget::Registration(
            "fixture.fn:1".into(),
        )];
        host.components[0]
            .registered_handler_bindings
            .push(RegisteredHandlerBinding {
                handler_id: HANDLER.into(),
                definition_ids: vec!["fixture.fn".into()],
                registration_ids: vec!["fixture.fn:1".into()],
            });
        let source = "function sample() :: number:\n    return 1\n";
        let parse = begin(&mut host, source, 1);
        let mut call = invocation(source, 1, 3, 7);
        assert!(host.incremental_key(0, &mut call).unwrap().is_some());
        if let HookPayload::Structure(value) = &mut call.payload {
            value.candidate.registration_id = "fixture.fn:2".into();
        }
        assert!(
            host.incremental_key(0, &mut call).unwrap().is_none(),
            "same definition does not broaden exact registration promise"
        );
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 1, false);
    }
    #[test]
    fn header_mixed_targets_do_not_broaden_an_exact_registration_to_its_definition() {
        use crate::bindings::nlaocs::skript_parser_addon::types::RegisteredHandlerBinding;
        let mut host = host();
        let handler = host.components[0]
            .manifest
            .registered_syntax_handlers
            .iter_mut()
            .find(|handler| handler.handler_id == HANDLER)
            .unwrap();
        handler.targets = vec![
            RegisteredSyntaxHandlerTarget::Definition("fixture.allowed".into()),
            RegisteredSyntaxHandlerTarget::Registration("fixture.fn:1".into()),
        ];
        host.components[0]
            .registered_handler_bindings
            .push(RegisteredHandlerBinding {
                handler_id: HANDLER.into(),
                definition_ids: vec!["fixture.allowed".into(), "fixture.fn".into()],
                registration_ids: vec!["fixture.allowed:1".into(), "fixture.fn:1".into()],
            });
        let source = "function sample() :: number:\n    return 1\n";
        let parse = begin(&mut host, source, 1);
        let mut call = invocation(source, 1, 3, 7);
        assert!(host.incremental_key(0, &mut call).unwrap().is_some());
        if let HookPayload::Structure(value) = &mut call.payload {
            value.candidate.registration_id = "fixture.fn:2".into();
        }
        assert!(
            host.incremental_key(0, &mut call).unwrap().is_none(),
            "another Definition target cannot broaden exact Registration promise"
        );
        if let HookPayload::Structure(value) = &mut call.payload {
            value.candidate.definition_id = "fixture.allowed".into();
            value.candidate.registration_id = "fixture.allowed:1".into();
        }
        assert!(
            host.incremental_key(0, &mut call).unwrap().is_some(),
            "explicit Definition target still applies"
        );
        parse.cancel().unwrap();
        host.finish_incremental_document(DOCUMENT, 1, false);
    }
}

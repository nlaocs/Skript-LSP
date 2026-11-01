use super::*;
use crate::bindings::nlaocs::skript_parser_addon::types::{
    CapabilityRequirement, ContextUpdate, ParseRequest,
};

const FIXTURE: &str = "nlaocs.test.dynamic-syntax";
const PREPASS: &str = "dynamic.prepass";

// The existing guest already registers on its Document callback, independent of
// the payload text. Adapt only its native test manifest/route to exercise the
// new production boundary without requiring a new built guest artifact. This
// does not claim the old guest implements source declaration validation.
fn fixture() -> ParserHost {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ssg/tests/data/legacy-2.6.4-mc-1.12.2");
    let mut host = ParserHost::new(
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../artifacts/core-library.wasm"
        )),
        HostConfig {
            syntax_catalog: Some(Arc::new(ssg::load(path).unwrap().into_catalog())),
            ..HostConfig::default()
        },
    )
    .unwrap();
    host.load_addon(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../artifacts/dynamic-syntax-addon.wasm"
    )))
    .unwrap();
    let manifest = &mut host.components.last_mut().unwrap().manifest;
    manifest
        .capabilities
        .retain(|c| c.id != crate::CAPABILITY_PROJECT_STABLE_SEMANTICS);
    for (id, version) in [
        (crate::CAPABILITY_DOCUMENT_DECLARATIONS, 1),
        (crate::CAPABILITY_PROJECT_STABLE_SEMANTICS, 2),
    ] {
        manifest.capabilities.push(CapabilityRequirement {
            id: id.into(),
            minimum_version: version,
            required: true,
        });
    }
    let route = manifest
        .subscriptions
        .iter_mut()
        .find(|s| s.id == PREPASS)
        .unwrap();
    route.capability_id = crate::CAPABILITY_DOCUMENT_DECLARATIONS.into();
    route.mode = HookMode::Transform;
    validate_manifest(manifest, &configured_host_capabilities(true, true)).unwrap();
    routes(&mut host, true);
    host
}

fn routes(host: &mut ParserHost, declarations: bool) {
    host.registry = SubscriptionRegistry::default();
    for (index, component) in host.components.iter().enumerate() {
        let selected = component
            .manifest
            .subscriptions
            .iter()
            .filter(|s| declarations || s.capability_id != crate::CAPABILITY_DOCUMENT_DECLARATIONS)
            .cloned()
            .collect::<Vec<_>>();
        host.registry
            .register(index, component.load_order, &selected);
    }
}

fn context(revision: u64) -> InvocationContext {
    InvocationContext {
        invocation_id: revision,
        subscription_id: String::new(),
        document_id: "file:///declarations/main.sk".into(),
        document_revision: revision,
        expansion: None,
        syntax_context: 0,
    }
}
fn request(revision: u64) -> DocumentParseRequest {
    DocumentParseRequest::new(
        "file:///declarations",
        &context(revision).document_id,
        revision,
        "",
    )
}
fn registered(snapshot: &syntaxes::DynamicSyntaxSnapshot) -> bool {
    snapshot
        .definitions
        .contains_key(&syntaxes::DynamicSyntaxId::new(FIXTURE, "prepass-effect"))
}

#[test]
fn document_pipeline_runs_only_opted_in_declarations_and_exposes_its_frozen_snapshot() {
    let mut host = fixture();
    let first = host
        .parse_document(request(1), DocumentParserConfig::default())
        .unwrap();
    assert!(
        first.component_failures.is_empty(),
        "{:?}",
        first.component_failures
    );
    assert_eq!(first.declaration_calls.len(), 1);
    assert_eq!(first.declaration_calls[0].component_id, FIXTURE);
    assert_eq!(first.declaration_calls[0].subscription_id, PREPASS);
    assert!(
        first
            .syntax_calls
            .iter()
            .all(|call| call.subscription_id != PREPASS)
    );
    assert_eq!(first.dynamic_syntax.document_id, context(1).document_id);
    assert_eq!(first.dynamic_syntax.document_revision, 1);
    assert!(registered(&first.dynamic_syntax));
    assert!(
        host.components
            .iter()
            .all(|component| component.store.data().tree_view.is_none())
    );
    // A new revision starts from the baseline, not the previous document overlay.
    routes(&mut host, false);
    let next = host
        .parse_document(request(2), DocumentParserConfig::default())
        .unwrap();
    assert!(next.declaration_calls.is_empty());
    assert!(!registered(&next.dynamic_syntax));
    assert!(
        registered(&first.dynamic_syntax),
        "old immutable views keep their original data"
    );
    host.release_document("file:///declarations", &context(2).document_id, 2)
        .unwrap();
    let reopened = host
        .parse_document(request(3), DocumentParserConfig::default())
        .unwrap();
    assert!(!registered(&reopened.dynamic_syntax));
}

#[test]
fn a_later_declaration_fault_rolls_back_earlier_accepted_registrations() {
    let mut host = fixture();
    let mut unavailable = host.components[1]
        .manifest
        .subscriptions
        .iter()
        .find(|s| s.id == PREPASS)
        .unwrap()
        .clone();
    unavailable.id = "fixture.unavailable-declaration-provider".into();
    unavailable.priority = 100;
    host.registry.register(0, 0, &[unavailable]);
    host.components[0].disabled = true;
    let result = host
        .parse_document(request(1), DocumentParserConfig::default())
        .unwrap();
    assert_eq!(result.execution_status, DocumentExecutionStatus::Partial);
    assert!(
        host.components
            .iter()
            .all(|component| component.store.data().tree_view.is_none())
    );
    assert!(!result.fault_regions.is_empty());
    assert_eq!(
        result.declaration_calls.first().unwrap().subscription_id,
        PREPASS
    );
    assert!(
        !registered(&result.dynamic_syntax),
        "partial discovery cannot publish an accepted prefix"
    );
}

#[test]
fn declaration_preparation_checks_cancellation_context_and_existing_tree_limits() {
    let mut host = fixture();
    let transaction = host
        .begin_parse("file:///declarations", &context(1).document_id, 1)
        .unwrap();
    let source = MappedSource::identity("on load:\n    send 1\n");
    let tree = Arc::new(skript_parser::parse_raw_tree(
        &source,
        host.raw_tree_options(),
    ));
    assert!(matches!(
        host.prepare_document_declarations(&transaction, &context(2), &source, Arc::clone(&tree)),
        Err(HostError::StateStore(StateError::InvalidInput { .. }))
    ));
    host.config.max_tree_macro_nodes = 1;
    assert!(matches!(
        host.prepare_document_declarations(&transaction, &context(1), &source, Arc::clone(&tree)),
        Err(HostError::TreeMacroNodeQuotaExceeded { limit: 1 })
    ));
    host.config.max_tree_macro_nodes = 100_000;
    let cancellation = DocumentCancellationToken::new();
    cancellation.cancel();
    host.native_cancellation = Some(cancellation);
    assert!(matches!(
        host.prepare_document_declarations(&transaction, &context(1), &source, Arc::clone(&tree)),
        Err(HostError::Cancelled(_))
    ));
    assert!(!registered(
        &host.dynamic_syntax_snapshot(&transaction).unwrap()
    ));
    transaction.cancel().unwrap();
}

#[test]
fn immutable_declaration_output_rejects_transformations_and_early_native_parsing() {
    let empty = || HookOutput {
        decision: HookDecision::ContinueProcessing,
        replacement: None,
        effects: empty_effects(),
    };
    assert!(validate_output(&empty()).is_ok());
    let mut output = empty();
    output.decision = HookDecision::Handled;
    assert!(validate_output(&output).is_err());
    let source = MappedSource::identity("example");
    let span = mapped_span_to_wit(source.map_range(ParserTextRange::new(0, 7)).unwrap());
    let mut output = empty();
    output.effects.context_updates.push(ContextUpdate {
        syntax_context: 0,
        key: "parser.event-classes".into(),
        value: Some(Vec::new()),
    });
    assert!(validate_output(&output).is_err());
    let mut output = empty();
    output.effects.parse_requests.push(ParseRequest {
        request_id: 1,
        parser_id: "host.expression".into(),
        input: "example".into(),
        expected_types: Vec::new(),
        span,
        options: Vec::new(),
    });
    assert!(validate_output(&output).is_err());
    let mut output = empty();
    output.replacement = Some(HookPayload::Tree(parser_raw_tree_to_wit(
        &skript_parser::parse_raw_tree(
            &source,
            skript_parser::RawTreeOptions::for_skript_version(2, 15),
        ),
    )));
    assert!(validate_output(&output).is_err());
}

#[test]
fn declaration_and_editor_promises_require_supported_explicit_versions() {
    let host = fixture();
    let manifest = &host.components[1].manifest;
    let route = manifest
        .subscriptions
        .iter()
        .find(|s| s.id == PREPASS)
        .unwrap();
    assert!(validate_subscription(manifest, route).is_ok());
    for version in [0, 1, 3] {
        let mut changed = manifest.clone();
        changed
            .capabilities
            .iter_mut()
            .find(|c| c.id == crate::CAPABILITY_PROJECT_STABLE_SEMANTICS)
            .unwrap()
            .minimum_version = version;
        assert!(validate_subscription(&changed, route).is_err());
    }
    let mut wrong = route.clone();
    wrong.mode = HookMode::Override;
    assert!(validate_subscription(manifest, &wrong).is_err());
    let claim = |version| CapabilityRequirement {
        id: crate::CAPABILITY_PROJECT_STABLE_SEMANTICS.into(),
        minimum_version: version,
        required: true,
    };
    for (version, allowed) in [(0, false), (1, true), (2, true), (3, false)] {
        assert_eq!(project_stability_promise(&[claim(version)], true), allowed);
    }
    assert!(!project_stability_promise(&[claim(1), claim(2)], true));
    assert!(
        host_capabilities()
            .iter()
            .any(|c| c.id == crate::CAPABILITY_PROJECT_STABLE_SEMANTICS && c.version == 2)
    );
    assert!(
        !host_capabilities()
            .iter()
            .any(|c| c.id == crate::CAPABILITY_DOCUMENT_DECLARATIONS)
    );
    assert!(
        configured_host_capabilities(true, true)
            .iter()
            .any(|c| c.id == crate::CAPABILITY_DOCUMENT_DECLARATIONS && c.version == 1)
    );
}

#[test]
fn document_declarations_prevent_startup_grammar_and_binding_closure() {
    let mut host = fixture();
    let declaration = host.components[1]
        .manifest
        .subscriptions
        .iter()
        .find(|s| s.id == PREPASS)
        .unwrap()
        .clone();
    // Retain just the actual Core baseline with its advertised closed grammar.
    host.dynamic_syntax_registry
        .as_ref()
        .unwrap()
        .remove_component(FIXTURE)
        .unwrap();
    host.components.truncate(1);
    routes(&mut host, false);
    assert!(host.completion_catalog_patterns_are_closed());
    assert!(host.completion_candidate_bindings_are_closed());
    // Even a provider claiming startup closure cannot certify future document
    // registrations that were absent when that static index was constructed.
    let (kind,definition,registration,expected)=host.config.syntax_catalog.as_ref().unwrap().syntaxes().iter()
        .filter(|syntax|matches!(syntax.kind(),CatalogSyntaxKind::Effect|CatalogSyntaxKind::Condition|CatalogSyntaxKind::Section))
        .find_map(|syntax| {
            let common=syntax.common()?;
            let routes=host.completion_capture_parser_ids(syntax.kind(),common.definition_id.as_str(),common.registration_id.as_str())?;
            (!routes.is_empty()).then(||(syntax.kind(),common.definition_id.0.clone(),common.registration_id.0.clone(),routes))
        }).expect("Core has a registered nested parser domain");
    host.registry.register(0, 0, &[declaration]);
    assert!(host.completion_capture_parser_ids(kind,&definition,&registration).is_none(),"strict proof remains unavailable");
    assert_eq!(host.completion_advisory_capture_parser_ids(kind,&definition,&registration),Some(expected),
        "an active document declaration subscriber does not erase positive grammar guidance");
    assert!(!host.completion_catalog_patterns_are_closed());
    assert!(!host.completion_candidate_bindings_are_closed());
}

#[test]
fn paged_carrier_is_constant_size_and_keeps_exact_whole_source_geometry() {
    let source = MappedSource::identity("# unrelated public comment\n".repeat(10_000));
    let tree = Arc::new(skript_parser::parse_raw_tree(
        &source,
        skript_parser::RawTreeOptions::for_skript_version(2, 15),
    ));
    assert!(tree.nodes.len() >= 10_000);
    let lease = tree_view::Lease::new(Arc::clone(&tree), u64::MAX, None).unwrap();
    let reference = lease.reference();
    let mut payload = request_payload(&source).unwrap();
    bind_tree_reference(&mut payload, reference).unwrap();
    assert!(
        hook_payload_size(&payload) < 1024,
        "the carrier must not lower node arrays or source text"
    );
    let HookPayload::Parser(request) = payload else {
        panic!("paged parser carrier required");
    };
    assert_eq!(request.parser_id, PARSER_ID);
    assert_eq!(request.request_id, 0);
    assert!(request.input.is_empty());
    assert!(request.expected_types.is_empty());
    assert_eq!(request.span.virtual_range.start, 0);
    assert_eq!(
        request.span.virtual_range.end,
        source.virtual_source().len() as u64
    );
    assert_eq!(
        request.span.origins[0].original_range.end,
        source.original().len() as u64
    );
    assert_eq!(request.options.len(), 1);
    let option = &request.options[0];
    assert_eq!(option.key, TREE_OPTION);
    assert!(option.owner_component_id.is_none());
    let value: serde_json::Value = serde_json::from_str(&option.value).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 5);
    assert_eq!(value["version"], 1);
    for (key, expected) in [
        ("epochHigh", reference.epoch_high),
        ("epochLow", reference.epoch_low),
        ("token", reference.token),
        ("generation", reference.generation),
    ] {
        assert_eq!(value[key].as_str(), Some(expected.to_string().as_str()));
    }
}

#[test]
fn declaration_view_uses_existing_bounded_pages_and_cannot_cross_stores_or_calls() {
    use crate::bindings::nlaocs::skript_parser_addon::{
        tree_data::Host as TreeHost, types::TreeError,
    };
    let mut host = fixture();
    let transaction = host
        .begin_parse("file:///declarations", &context(1).document_id, 1)
        .unwrap();
    let source = MappedSource::identity("# public header\n".repeat(300));
    let tree = Arc::new(skript_parser::parse_raw_tree(
        &source,
        host.raw_tree_options(),
    ));
    let lease = tree_view::Lease::new(Arc::clone(&tree), 1, None).unwrap();
    let reference = tree_access::reference(lease.reference());
    host.components[1].store.data_mut().invocation =
        Some(transaction.begin_invocation(FIXTURE).unwrap());
    host.components[1].store.data_mut().tree_view = Some(lease);
    let page = TreeHost::roots(
        host.components[1].store.data_mut(),
        reference.clone(),
        0,
        128,
    )
    .unwrap();
    assert_eq!(page.nodes.len(), 128);
    assert!(page.next.is_some());
    assert!(matches!(
        TreeHost::roots(
            host.components[1].store.data_mut(),
            reference.clone(),
            0,
            129
        ),
        Err(TreeError::InvalidPage)
    ));
    assert!(matches!(
        TreeHost::info(host.components[0].store.data_mut(), reference.clone()),
        Err(TreeError::Unavailable)
    ));
    host.components[1].store.data_mut().tree_view.take();
    assert!(matches!(
        TreeHost::info(host.components[1].store.data_mut(), reference.clone()),
        Err(TreeError::Unavailable)
    ));
    host.components[1].store.data_mut().tree_view =
        Some(tree_view::Lease::new(tree, 1, None).unwrap());
    assert!(matches!(
        TreeHost::info(host.components[1].store.data_mut(), reference),
        Err(TreeError::Stale)
    ));
    host.components[1].store.data_mut().tree_view.take();
    host.components[1]
        .store
        .data_mut()
        .invocation
        .take()
        .unwrap()
        .rollback();
    transaction.cancel().unwrap();
}

#[test]
fn a_declaration_guest_trap_revokes_the_view_and_rolls_back_registration() {
    let mut host = fixture();
    host.config.fuel_per_call = 1;
    let result = host
        .parse_document(request(1), DocumentParserConfig::default())
        .unwrap();
    assert_eq!(result.execution_status, DocumentExecutionStatus::Partial);
    assert!(
        result
            .component_failures
            .iter()
            .any(|failure| matches!(failure.error, HostError::FuelExhausted { .. }))
    );
    assert!(
        host.components
            .iter()
            .all(|component| component.store.data().tree_view.is_none())
    );
    assert!(!registered(&result.dynamic_syntax));
}

use super::*;
use std::path::Path;
use syntaxes::{CatalogParts, ClassName, DefinitionId, Pattern, RegistrationId, Syntax};

const CORE: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../artifacts/core-library.wasm"));
const ADDON: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../artifacts/type-parser-addon.wasm"));
const OWNER: &str = "test.type-parser-addon";
const PROJECT: &str = "file:///transfer-tests";
const DOCUMENT: &str = "file:///transfer-tests/document.sk";
const TRANSFER_REGISTRATION: &str = "structure:test.type-parser-addon:transfer:0";
const SOURCE: &str = "fixture transfer\n";

fn host(mode: &str) -> ParserHost {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/type-parser-versions/skript-2.15.4-storage-domain");
    let snapshot = ssg::load(fixture).expect("stock storage snapshot must load");
    let source = snapshot.catalog();
    let mut syntaxes = source.syntaxes().to_vec();
    for syntax in &mut syntaxes {
        if let Syntax::Type(value) = syntax {
            let parser = match value.code_name.as_str() {
                "number" => "fixture.NumberParser",
                "blockdata" => "fixture.BlockDataParser",
                "loottable" => "fixture.LootTableParser",
                "enchantmenttype" => "fixture.EnchantmentTypeParser",
                _ => continue,
            };
            value.parser_class = Some(ClassName(parser.into()));
        }
    }
    // A root with no body keeps this boundary fixture's complete author set
    // inside the querying addon, without granting authority to Core's parents.
    let mut structure = source.structures().find(|value|
        value.common.element_class.as_str() == "ch.njol.skript.structures.StructUsing")
        .expect("stock snapshot must contain a simple Structure").clone();
    structure.common.definition_id = DefinitionId("structure:test.type-parser-addon:transfer".into());
    structure.common.registration_id = RegistrationId(TRANSFER_REGISTRATION.into());
    structure.common.registration_order = usize::MAX;
    structure.common.element_class = ClassName("fixture.TransferStructure".into());
    structure.common.priority = None;
    structure.common.priority_name = None;
    structure.common.patterns = vec![Pattern {source: "fixture transfer".into(),
        parsed: syntax_pattern_parser::syntax::parse("fixture transfer", source.plural_rules()).unwrap()}];
    structure.entry_validator = None;
    syntaxes.push(Syntax::Structure(structure));
    let catalog = Catalog::new(CatalogParts {
        syntaxes,
        converters: source.converters().to_vec(),
        comparators: source.comparators().to_vec(),
        event_values: source.event_values().to_vec(),
        properties: source.properties().to_vec(),
        operators: source.operators().to_vec(),
        operations: source.operations().clone(),
        differences: source.differences().to_vec(),
        classes: source.classes().to_vec(),
        aliases: source.aliases().clone(),
        plural_rules: source.plural_rules().clone(),
        language: source.language_entries().map(|(key, value)| (key.into(), value.into())).collect(),
    }).with_unchecked_source(source.source().unwrap().clone());
    let mut host = ParserHost::new(CORE, HostConfig {
        syntax_catalog: Some(Arc::new(catalog)),
        runtime_profile: RuntimeProfile {
            language: Some(format!("test.semantic-transfer/{mode}")),
            ..RuntimeProfile::default()
        },
        // Leave the ordinary 20M fuel budget intact and make fuel the first limit.
        call_timeout: Duration::from_secs(2),
        ..HostConfig::default()
    }).expect("Core must initialize with the stock source");
    host.load_addon(ADDON).expect("Type parser fixture must initialize");
    host
}

fn parse(host: &mut ParserHost, revision: u64, source: &str) -> DocumentParseResult {
    let result = host.parse_document(
        DocumentParseRequest::new(PROJECT, DOCUMENT, revision, source.to_owned()),
        DocumentParserConfig::default(),
    ).expect("fixture document must parse through the public host API");
    assert!(result.component_failures.is_empty(), "{:?}", result.component_failures);
    assert!(result.syntax.diagnostics.is_empty(), "{:?}", result.syntax.diagnostics);
    result
}

fn selected(result: &DocumentParseResult) -> u64 {
    let node = result.ast.nodes.iter().find(|node|
        node.summary.as_ref().and_then(|summary| summary.registration_id.as_deref()) == Some(TRANSFER_REGISTRATION)
        && node.metadata.iter().any(|entry| entry.key == METADATA
            && entry.owner_component_id.as_deref() == Some(OWNER)))
        .expect("the selected Structure must carry host-owned transfer metadata");
    let witnesses = result.semantic_witnesses();
    assert!(witnesses.provider_valid(node, OWNER),
        "the body-free selected root must have an actual unique-author semantic receipt");
    assert_eq!(result.ast.roots, [node.id]);
    assert!(node.children.is_empty());
    assert!(node.captures.iter().all(|capture| matches!(&capture.value,
        CaptureValue::Nodes(nodes) if nodes.is_empty())), "{:?}", node.captures);
    assert!(result.syntax_calls.iter().any(|call| call.component_id == OWNER));
    node.id
}

fn bound() -> TypeBound {
    TypeBound { types: vec!["java.lang.Number".into()], complete: true, multiple: false, maybe_unset: false }
}

#[test]
fn selected_structure_transfers_a_complete_bound_through_the_guest_export() {
    let mut host = host("good");
    let result = parse(&mut host, 1, SOURCE);
    let node = selected(&result);
    assert_eq!(result.state.writes, 0);
    let mut session = host.semantic_transfer_session(&result, None);
    let output = session.evaluate(node, bound()).unwrap().expect("selected operation must execute");
    assert_eq!(session.calls, 1);
    assert_eq!(output.value.types, ["java.lang.Number"]);
    assert!(output.value.complete && !output.value.multiple && !output.value.maybe_unset);
    assert!(matches!(output.source, SourceUpdate::Unchanged));
    assert!(matches!(output.effects, BindingEffects::Preserve));
    assert!(session.certificate().unwrap().is_current());
    let empty = TypeBound { types: Vec::new(), maybe_unset: true, ..bound() };
    let output = session.evaluate(node, empty).unwrap().unwrap();
    assert!(output.value.types.is_empty() && output.value.complete && output.value.maybe_unset);
}

#[test]
fn incomplete_plural_duplicate_unknown_and_overlarge_inputs_do_not_invoke_the_guest() {
    let mut host = host("good");
    let result = parse(&mut host, 1, SOURCE);
    let node = selected(&result);
    let mut session = host.semantic_transfer_session(&result, None);
    assert!(session.evaluate(node, bound()).unwrap().is_some());
    for case in ["partial", "plural", "duplicate", "object", "empty", "control", "long", "many"] {
        let mut input = bound();
        match case {
            "partial" => input.complete = false,
            "plural" => input.multiple = true,
            "duplicate" => input.types.push(input.types[0].clone()),
            "object" => input.types = vec!["java.lang.Object".into()],
            "empty" => input.types.clear(),
            "control" => input.types[0].push('\n'),
            "long" => input.types = vec!["x".repeat(1025)],
            "many" => input.types = (0..65).map(|index| format!("fixture.Type{index}")).collect(),
            _ => unreachable!(),
        }
        assert!(session.evaluate(node, input).unwrap().is_none(), "{case}");
        assert_eq!(session.calls, 1, "invalid input reached the guest: {case}");
    }
}

#[test]
fn partial_and_duplicate_guest_bounds_are_errors_without_current_output() {
    for mode in ["partial", "duplicate-types"] {
        let mut host = host(mode);
        let result = parse(&mut host, 1, SOURCE);
        let node = selected(&result);
        let mut session = host.semantic_transfer_session(&result, None);
        let error = session.evaluate(node, bound()).expect_err("invalid guest bound cannot be adopted");
        assert!(matches!(error, HostError::InvalidHookOutput { ref component_id, ref subscription_id, .. }
            if component_id == OWNER && subscription_id == METADATA), "{mode}: {error:?}");
        assert_eq!(session.calls, 1, "the actual guest export must have run: {mode}");
    }
}

#[test]
fn individually_valid_guest_bounds_cannot_exceed_the_combined_result_budget() {
    let mut host = host("oversized");
    let result = parse(&mut host, 1, SOURCE);
    let node = selected(&result);
    let mut session = host.semantic_transfer_session(&result, None);
    assert!(session.evaluate(node, bound()).unwrap().is_none());
    assert_eq!(session.calls, 1, "the oversized output must come from the guest export");
}

#[test]
fn forbidden_state_import_is_rejected_even_when_the_guest_ignores_its_error() {
    let mut host = host("forbidden-state");
    let result = parse(&mut host, 1, SOURCE);
    let node = selected(&result);
    let mut session = host.semantic_transfer_session(&result, None);
    let error = session.evaluate(node, bound()).expect_err("ignored import failure cannot authorize output");
    assert!(matches!(error, HostError::InvalidHookOutput { ref component_id, ref message, .. }
        if component_id == OWNER && message.contains("state/scope/parse import")), "{error:?}");
    assert_eq!(session.calls, 1);
    drop(session);
    let store = host.components.iter().find(|entry| entry.manifest.component_id == OWNER).unwrap().store.data();
    assert!(!store.editor_readonly && !store.editor_input_only && !store.editor_mutation_attempted);
    assert!(store.invocation.is_none());
    let healthy = parse(&mut host, 2, SOURCE);
    assert_eq!(healthy.state.writes, 0);
    selected(&healthy);
}

#[test]
fn transfer_fuel_exhaustion_revokes_certificates_and_disables_the_provider() {
    let mut host = host("fuel");
    let result = parse(&mut host, 1, SOURCE);
    let node = selected(&result);
    let mut session = host.semantic_transfer_session(&result, None);
    let certificate = session.certificate().unwrap();
    let error = session.evaluate(node, bound()).expect_err("unbounded guest must exhaust fuel");
    assert!(matches!(error, HostError::FuelExhausted { ref component_id, operation: "semantic transfer" }
        if component_id == OWNER), "{error:?}");
    assert_eq!(session.calls, 1);
    assert!(!certificate.is_current());
    assert!(session.certificate().is_none());
    assert!(session.evaluate(node, bound()).unwrap().is_none());
    drop(session);
    assert!(host.components().iter().any(|entry| entry.component_id == OWNER && entry.disabled));
}

#[test]
fn cancellation_after_session_creation_cannot_produce_another_output() {
    let mut host = host("good");
    let result = parse(&mut host, 1, SOURCE);
    let node = selected(&result);
    let token = DocumentCancellationToken::new();
    let mut session = host.semantic_transfer_session(&result, Some(token.clone()));
    assert!(session.evaluate(node, bound()).unwrap().is_some());
    token.cancel();
    assert!(matches!(session.evaluate(node, bound()), Err(HostError::Cancelled(_))));
    assert_eq!(session.calls, 1, "cancelled evaluation must not reenter the guest");
}

#[test]
fn source_revision_close_and_reopen_never_rebind_an_old_result() {
    let mut host = host("good");
    let old = parse(&mut host, 1, SOURCE);
    let old_node = selected(&old);
    let old_certificate = {
        let mut session = host.semantic_transfer_session(&old, None);
        assert!(session.evaluate(old_node, bound()).unwrap().is_some());
        session.certificate().unwrap()
    };
    let newer = parse(&mut host, 2, &format!("{SOURCE}# edited source\n"));
    let newer_node = selected(&newer);
    assert!(!old_certificate.is_current());
    assert!(host.semantic_transfer_session(&old, None).evaluate(old_node, bound()).unwrap().is_none());
    let newer_certificate = {
        let mut session = host.semantic_transfer_session(&newer, None);
        assert!(session.evaluate(newer_node, bound()).unwrap().is_some());
        session.certificate().unwrap()
    };
    host.release_document(PROJECT, DOCUMENT, 1).unwrap();
    assert!(newer_certificate.is_current(), "a delayed old close cannot revoke a newer revision");
    host.release_document(PROJECT, DOCUMENT, 2).unwrap();
    assert!(!newer_certificate.is_current());
    let reopened = parse(&mut host, 3, SOURCE);
    let reopened_node = selected(&reopened);
    assert!(host.semantic_transfer_session(&newer, None).evaluate(newer_node, bound()).unwrap().is_none());
    assert!(host.semantic_transfer_session(&reopened, None).evaluate(reopened_node, bound()).unwrap().is_some());
}

#[test]
fn unloading_the_owner_revokes_old_results_and_materialized_certificates() {
    let mut host = host("good");
    let result = parse(&mut host, 1, SOURCE);
    let node = selected(&result);
    let certificate = {
        let mut session = host.semantic_transfer_session(&result, None);
        assert!(session.evaluate(node, bound()).unwrap().is_some());
        session.certificate().unwrap()
    };
    assert!(host.unload_addon(OWNER).unwrap());
    assert!(!certificate.is_current());
    let mut session = host.semantic_transfer_session(&result, None);
    assert!(session.certificate().is_none());
    assert!(session.evaluate(node, bound()).unwrap().is_none());
    assert_eq!(session.calls, 0);
}

#[test]
fn editing_public_ast_metadata_cannot_forge_a_selected_operation() {
    let mut host = host("good");
    for (index, case) in ["owner", "parameters", "duplicate", "malformed"].into_iter().enumerate() {
        let mut result = parse(&mut host, index as u64 + 1, SOURCE);
        let node_id = selected(&result);
        assert!(host.semantic_transfer_session(&result, None).evaluate(node_id, bound()).unwrap().is_some());
        let node = result.ast.nodes.iter_mut().find(|node| node.id == node_id).unwrap();
        let entry = node.metadata.iter_mut().find(|entry| entry.key == METADATA).unwrap();
        match case {
            "owner" => entry.owner_component_id = Some("foreign.provider".into()),
            "parameters" => entry.value = entry.value.replace("\"1\"", "\"2\""),
            "duplicate" => { let duplicate = entry.clone(); node.metadata.push(duplicate); }
            "malformed" => entry.value = "{".into(),
            _ => unreachable!(),
        }
        let mut session = host.semantic_transfer_session(&result, None);
        assert!(session.evaluate(node_id, bound()).unwrap().is_none(), "{case}");
        assert_eq!(session.calls, 0, "public metadata edits cannot enter the guest: {case}");
    }
}

#[test]
fn a_preparation_generation_mismatch_cannot_reuse_an_old_operation() {
    let mut host = host("good");
    let result = parse(&mut host, 1, SOURCE);
    let node = selected(&result);
    assert!(host.semantic_transfer_session(&result, None).evaluate(node, bound()).unwrap().is_some());
    // A focused comparator test, not an invented public re-preparation API.
    // Real restarts additionally revoke the host authority epoch.
    let owner = host.components.iter_mut().find(|entry| entry.manifest.component_id == OWNER).unwrap();
    owner.preparation_generation = owner.preparation_generation.checked_add(1).unwrap();
    let mut stale = host.semantic_transfer_session(&result, None);
    assert!(stale.evaluate(node, bound()).unwrap().is_none());
    assert_eq!(stale.calls, 0);
}

#[test]
fn dropping_the_host_revokes_materialized_transfer_results() {
    let certificate = {
        let mut host = host("good");
        let result = parse(&mut host, 1, SOURCE);
        let node = selected(&result);
        let mut session = host.semantic_transfer_session(&result, None);
        assert!(session.evaluate(node, bound()).unwrap().is_some());
        session.certificate().unwrap()
    };
    assert!(!certificate.is_current());
}

#[test]
fn checked_value_identity_requires_explicit_current_v3_and_rejects_contradictory_outputs() {
    for mode in ["good","identity","identity-known-source","identity-changed-bound","identity-writeback"] {
        let mut host=host(mode);let result=parse(&mut host,1,SOURCE);let node=selected(&result);
        let mut session=host.semantic_transfer_session(&result,None);
        assert!(!session.binding_identity(node));
        let answer=session.evaluate(node,bound());
        if matches!(mode,"identity-changed-bound"|"identity-writeback") {
            assert!(matches!(answer,Err(HostError::InvalidHookOutput {..})),"{mode}: {answer:?}");
            assert!(!session.binding_identity(node));
        }else{
            assert!(answer.unwrap().is_some());
            assert_eq!(session.binding_identity(node),mode=="identity","{mode}");
            // A failed/unsupported follow-up cannot leave a previous identity.
            assert!(session.evaluate(u64::MAX,bound()).unwrap().is_none());
            assert!(!session.binding_identity(node));
        }
    }
}
#[test]
fn unknown_or_incomplete_value_relation_descriptors_are_not_identity_promises() {
    for bad in [serde_json::json!({"version":2,"outputUse":"flow","valueRelation":"same-source-when-unchanged"}),
        serde_json::json!({"version":3,"outputUse":"flow","valueRelation":"same-source-when-unchanged"}),
        serde_json::json!({"version":3,"outputUse":"completion-candidate","valueRelation":"same-source-when-unchanged","bindingValues":"preserve"}),
        serde_json::json!({"version":3,"outputUse":"flow","valueRelation":"same-type","bindingValues":"preserve"})] {
        let mut value=bad;value["operation"]="fixture".into();value["parameters"]="{}".into();
        let op:Operation=serde_json::from_value(value).unwrap();assert!(!op.supported());
    }
}



fn host_with_restartable_text() -> (ParserHost, usize) {
    let mut host = host("good");
    host.load_addon(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/../artifacts/text-macro-addon.wasm"))).unwrap();
    let index = host.components.iter().position(|entry|
        entry.manifest.component_id == "nlaocs.test.text-macro").unwrap();
    assert!(component_promises(&host.components[index].manifest.capabilities,
        crate::CAPABILITY_EPOCH_CANCELLATION_RESTART_SAFE));
    (host, index)
}

#[test]
fn document_recovery_creates_a_current_transfer_fence() {
    let (mut host, recovering) = host_with_restartable_text();
    let old = parse(&mut host, 1, SOURCE);
    let old_node = selected(&old);
    let old_certificate = host.semantic_transfer_session(&old, None).certificate().unwrap();
    let generation = host.components[recovering].preparation_generation;
    // Actual marker creation and rollback are covered by the in-guest
    // cancellation tests. This test covers the next ordinary document.
    host.components[recovering].store.data_mut().epoch_cancelled = true;
    let fresh = parse(&mut host, 2, SOURCE);
    let fresh_node = selected(&fresh);
    assert!(!host.components[recovering].store.data().epoch_cancelled);
    assert!(host.components[recovering].preparation_generation > generation);
    assert!(!old_certificate.is_current(), "restart cannot rebase an old certificate");
    assert!(host.semantic_transfer_session(&old, None).evaluate(old_node, bound()).unwrap().is_none());
    let mut session = host.semantic_transfer_session(&fresh, None);
    assert!(session.current(), "the new result captured its transfer fence before recovery");
    assert!(session.evaluate(fresh_node, bound()).unwrap().is_some());
    assert!(session.certificate().unwrap().is_current());
}

#[test]
fn document_recovery_cancellation_restores_the_caller_token_and_allows_retry() {
    let (mut host, recovering) = host_with_restartable_text();
    let old = parse(&mut host, 1, SOURCE);
    let old_certificate = host.semantic_transfer_session(&old, None).certificate().unwrap();
    let previous = DocumentCancellationToken::new();
    host.native_cancellation = Some(previous.clone());
    let cancellation = DocumentCancellationToken::new();
    host.components[recovering].restart_pending = true;
    host.components[recovering].cancel_during_restart_initialize = Some(cancellation.clone());
    let error = host.parse_document(DocumentParseRequest::new(PROJECT, DOCUMENT, 2, SOURCE.to_owned()),
        DocumentParserConfig {cancellation:cancellation.clone(),..Default::default()}).unwrap_err();
    assert!(matches!(error, super::super::document::DocumentParseError::Cancelled {
        stage:super::super::document::DocumentParseStage::Start,..}), "{error:?}");
    assert!(cancellation.is_cancelled());
    assert!(!previous.is_cancelled());
    assert!(host.components[recovering].restart_pending && !host.components[recovering].disabled);
    assert!(!old_certificate.is_current(), "recovery revocation survives a cancelled new document");
    previous.cancel();
    assert!(host.native_cancellation.as_ref().unwrap().is_cancelled(), "the exact caller token must be restored");
    host.native_cancellation = None;
    let fresh = parse(&mut host, 3, SOURCE);
    let node = selected(&fresh);
    let mut session = host.semantic_transfer_session(&fresh, None);
    assert!(session.evaluate(node, bound()).unwrap().is_some());
    assert!(session.certificate().unwrap().is_current());
}

#[test]
fn document_recovery_failure_keeps_the_existing_scoped_fault_path() {
    let (mut host, recovering) = host_with_restartable_text();
    let old = parse(&mut host, 1, SOURCE);
    let old_certificate = host.semantic_transfer_session(&old, None).certificate().unwrap();
    // Same retained-manifest incompatibility used by the restart handshake
    // tests: preflight must disable it, not add a new fatal Start failure.
    host.components[recovering].manifest.component_version.push_str("-incompatible");
    host.components[recovering].restart_pending = true;
    let fresh = host.parse_document(DocumentParseRequest::new(PROJECT, DOCUMENT, 2, SOURCE.to_owned()),
        DocumentParserConfig::default()).expect("disabled-provider effects remain ordinary scoped fault data");
    assert!(host.components[recovering].disabled);
    assert!(fresh.component_failures.iter().any(|failure|
        failure.component_id == "nlaocs.test.text-macro"));
    assert_eq!(fresh.execution_status, super::super::document::DocumentExecutionStatus::Partial);
    assert!(!fresh.fault_regions.is_empty(), "failed preprocessing keeps its unresolved region");
    assert!(!old_certificate.is_current());
    assert!(host.semantic_transfer_session(&fresh, None).certificate().is_some(),
        "the fresh result must use the actual disabled-provider universe, without certifying its effects");
}

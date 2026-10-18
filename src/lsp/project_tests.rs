use super::*;
use parser_wasm::DocumentCancellationToken;

#[test]
fn unknown_component_project_stability_refuses_cross_document_facts() {
    assert!(require_stable_project(1, false).is_ok());
    assert!(require_stable_project(2, true).is_ok());
    let error = require_stable_project(2, false).unwrap_err().to_string();
    assert!(error.contains("parser.project-stable-semantics v1"));
}

fn view(path: &Path, source: &str, generation: u64, source_epoch: u64) -> ProjectSource {
    ProjectSource {
        uri: path_uri(path).unwrap(),
        generation,
        source_epoch,
        source: Arc::from(source),
    }
}
#[test]
fn open_buffers_override_disk_including_invalid_utf8_without_writing_files() {
    let root = tempfile::tempdir().unwrap();
    let opened = root.path().join("open.sk");
    fs::write(&opened, [0xff]).unwrap();
    fs::create_dir(root.path().join("nested")).unwrap();
    fs::write(root.path().join("nested/closed.sk"), "function closed():").unwrap();
    let snapshot = ProjectSnapshot {
        epoch: 1,
        root: root.path().into(),
        open: vec![view(&opened, "on load:", 2, 4)],
    };
    let found = sources(&snapshot, &DocumentCancellationToken::new()).unwrap();
    assert_eq!(found.len(), 2);
    let opened_source = &found[snapshot.open[0].uri.as_str()];
    assert_eq!(opened_source.view.source.as_ref(), "on load:");
    assert!(opened_source.path.is_none());
    assert_eq!(fs::read(&opened).unwrap(), [0xff]);
}
#[test]
fn new_nested_open_path_is_contained_and_duplicate_aliases_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let source = view(
        &root.path().join("not-created/nested/new.sk"),
        "on load:",
        1,
        1,
    );
    let mut snapshot = ProjectSnapshot {
        epoch: 1,
        root: root.path().into(),
        open: vec![source.clone()],
    };
    assert_eq!(
        sources(&snapshot, &DocumentCancellationToken::new())
            .unwrap()
            .len(),
        1
    );
    snapshot.open.push(source);
    assert!(
        sources(&snapshot, &DocumentCancellationToken::new())
            .unwrap_err()
            .to_string()
            .contains("same file")
    );
}
#[test]
fn cancellation_and_source_budget_fail_explicitly() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("large.sk");
    let snapshot = ProjectSnapshot {
        epoch: 1,
        root: root.path().into(),
        open: Vec::new(),
    };
    let cancel = DocumentCancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        sources(&snapshot, &cancel),
        Err(ProjectError::Cancelled)
    ));
    fs::write(path, vec![b' '; MAX_FILE_BYTES + 1]).unwrap();
    assert!(
        sources(&snapshot, &DocumentCancellationToken::new())
            .unwrap_err()
            .to_string()
            .contains("analysis limit")
    );
}
#[cfg(unix)]
#[test]
fn symlinks_are_not_followed_outside_or_inside_the_project() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    fs::write(external.path().join("external.sk"), "on load:").unwrap();
    symlink(external.path(), root.path().join("linked")).unwrap();
    symlink(
        external.path().join("external.sk"),
        root.path().join("linked.sk"),
    )
    .unwrap();
    let snapshot = ProjectSnapshot {
        epoch: 1,
        root: root.path().into(),
        open: Vec::new(),
    };
    assert!(
        sources(&snapshot, &DocumentCancellationToken::new())
            .unwrap()
            .is_empty()
    );
}
#[test]
fn source_generation_and_epoch_are_witnesses_even_when_text_is_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("source.sk");
    let snapshot = ProjectSnapshot {
        epoch: 1,
        root: root.path().into(),
        open: vec![view(&path, "on load:", 1, 2)],
    };
    let first = sources(&snapshot, &DocumentCancellationToken::new())
        .unwrap()
        .into_values()
        .next()
        .unwrap();
    let mut next = snapshot.clone();
    next.open[0].generation += 1;
    let changed = sources(&next, &DocumentCancellationToken::new())
        .unwrap()
        .into_values()
        .next()
        .unwrap();
    assert_eq!(first.witness, changed.witness);
    assert!(!first.same(&changed));
    next.open[0].generation -= 1;
    next.open[0].source_epoch += 1;
    let changed = sources(&next, &DocumentCancellationToken::new())
        .unwrap()
        .into_values()
        .next()
        .unwrap();
    assert!(!first.same(&changed));
}

fn host() -> (ParserHost, Arc<Catalog>) {
    let catalog = Arc::new(
        ssg::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4"),
        )
        .unwrap()
        .into_catalog(),
    );
    let host = crate::new_parser_host(parser_wasm::HostConfig {
        syntax_catalog: Some(Arc::clone(&catalog)),
        ..Default::default()
    })
    .unwrap();
    (host, catalog)
}
#[test]
fn actual_unopened_function_is_parsed_imported_and_jump_targets_current_source() {
    let root = tempfile::tempdir().unwrap();
    let definition = root.path().join("definition.sk");
    let caller = root.path().join("caller.sk");
    fs::write(
        &definition,
        "function project_value() :: number:\n    return 1\n",
    )
    .unwrap();
    let source = "on load:\n    set {_value} to project_value()\n";
    let snapshot = ProjectSnapshot {
        epoch: 1,
        root: root.path().into(),
        open: vec![view(&caller, source, 1, 1)],
    };
    let (mut host, catalog) = host();
    let mut index = ProjectIndex::new();
    let current = snapshot.open[0].uri.clone();
    let mut preparations = 0;
    let analyzed = index
        .analyze(
            &mut host,
            Arc::clone(&catalog),
            "file:///project",
            &snapshot,
            &current,
            Default::default(),
            &mut |_| preparations += 1,
        )
        .unwrap();
    assert_eq!(analyzed.scanned_documents, 2);
    assert!(preparations > 0);
    let call = analyzed
        .result
        .ast
        .nodes
        .iter()
        .find(|node| {
            node.function
                .as_ref()
                .is_some_and(|call| call.name == "project_value")
        })
        .expect("real native project Function call");
    assert!(
        call.metadata
            .iter()
            .any(|entry| entry.key == "function.source-witness")
    );
    assert!(
        analyzed
            .result
            .functions
            .lookups()
            .iter()
            .any(|lookup| lookup.name == "project_value" && !lookup.definitions.is_empty())
    );
    let project = analyzed
        .documents
        .iter()
        .map(Arc::as_ref)
        .collect::<Vec<_>>();
    let targets = analyzed
        .current_semantic
        .definitions_at(lsp_types::Position::new(1, 24), &project);
    assert_eq!(targets.len(), 1);
    assert_eq!(
        fs::canonicalize(uri_path(&targets[0].uri).unwrap()).unwrap(),
        fs::canonicalize(&definition).unwrap()
    );
    let native_hover = super::super::hover::Document::from_result(
        analyzed.result,
        Arc::from(source),
        catalog,
        lsp_types::MarkupKind::PlainText,
    )
    .unwrap();
    let text = serde_json::to_string(&native_hover.at(lsp_types::Position::new(1, 24))).unwrap();
    assert!(
        text.contains("Signature") && text.contains("project_value() -> number"),
        "an actual imported call must retain its current native signature: {text}"
    );
}
#[test]
fn removed_closed_definition_invalidates_actual_call_and_records_negative_lookup() {
    let root = tempfile::tempdir().unwrap();
    let definition = root.path().join("definition.sk");
    let caller = root.path().join("caller.sk");
    fs::write(
        &definition,
        "function project_value() :: number:\n    return 1\n",
    )
    .unwrap();
    let mut snapshot = ProjectSnapshot {
        epoch: 1,
        root: root.path().into(),
        open: vec![view(
            &caller,
            "on load:\n    set {_value} to project_value()\n",
            1,
            1,
        )],
    };
    let (mut host, catalog) = host();
    let mut index = ProjectIndex::new();
    let current = snapshot.open[0].uri.clone();
    index
        .analyze(
            &mut host,
            Arc::clone(&catalog),
            "file:///project",
            &snapshot,
            &current,
            Default::default(),
            &mut |_| {},
        )
        .unwrap();
    fs::remove_file(&definition).unwrap();
    snapshot.epoch += 1;
    let analyzed = index
        .analyze(
            &mut host,
            catalog,
            "file:///project",
            &snapshot,
            &current,
            Default::default(),
            &mut |_| {},
        )
        .unwrap();
    assert!(analyzed.result.ast.nodes.iter().all(|node| {
        node.function
            .as_ref()
            .is_none_or(|call| call.name != "project_value")
    }));
    assert!(
        analyzed
            .result
            .functions
            .lookups()
            .iter()
            .any(|lookup| lookup.name == "project_value" && lookup.definitions.is_empty())
    );
    let project = analyzed
        .documents
        .iter()
        .map(Arc::as_ref)
        .collect::<Vec<_>>();
    assert!(
        analyzed
            .current_semantic
            .definitions_at(lsp_types::Position::new(1, 24), &project)
            .is_empty()
    );
}

#[test]
fn unrelated_edit_reuses_contract_certified_closed_document_semantics() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("definition.sk"),
        "function project_value() :: number:\n    return 1\n",
    )
    .unwrap();
    let caller = root.path().join("caller.sk");
    let mut snapshot = ProjectSnapshot {
        epoch: 1,
        root: root.path().into(),
        open: vec![view(&caller, "on load:\n    send 1\n", 1, 1)],
    };
    let (mut host, catalog) = host();
    let mut index = ProjectIndex::new();
    let current = snapshot.open[0].uri.clone();
    index
        .analyze(
            &mut host,
            Arc::clone(&catalog),
            "file:///project",
            &snapshot,
            &current,
            Default::default(),
            &mut |_| {},
        )
        .unwrap();
    snapshot.epoch += 1;
    snapshot.open[0].source = Arc::from("on load:\n    send 2\n");
    snapshot.open[0].generation += 1;
    snapshot.open[0].source_epoch += 1;
    let analyzed = index
        .analyze(
            &mut host,
            catalog,
            "file:///project",
            &snapshot,
            &current,
            Default::default(),
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(analyzed.scanned_documents, 2);
    assert_eq!(
        analyzed.parsed_documents, 2,
        "changed current discovery + final current result; unchanged closed definition stays certified"
    );
}

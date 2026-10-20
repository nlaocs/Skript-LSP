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
    assert_eq!(
        preparations, 0,
        "the retained stable current result needs no provisional reparse"
    );
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
        analyzed.parsed_documents, 1,
        "the changed current result is retained with the full unchanged import namespace"
    );
}

#[test]
fn nested_project_markers_are_boundaries_but_ordinary_folder_names_are_not() {
    let root = tempfile::tempdir().unwrap();
    for name in [".hidden", "target", "node_modules", "vendor", "ordinary"] {
        let folder = root.path().join(name);
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("allowed.sk"), "on load:\n    send 1\n").unwrap();
    }
    for (name, directory_marker) in [("nested", true), ("worktree", false)] {
        let folder = root.path().join(name);
        fs::create_dir(&folder).unwrap();
        if directory_marker {
            fs::create_dir(folder.join(".git")).unwrap();
        } else {
            fs::write(folder.join(".git"), "gitdir: elsewhere").unwrap();
        }
        fs::write(folder.join("not-part-of-parent.sk"), [0xff]).unwrap();
    }
    let mut snapshot = ProjectSnapshot {
        epoch: 1,
        root: root.path().into(),
        open: vec![],
    };
    let found = sources(&snapshot, &DocumentCancellationToken::new()).unwrap();
    assert_eq!(found.len(), 5);
    snapshot.open.push(view(
        &root.path().join("nested/new.sk"),
        "on load:\n    send 1\n",
        1,
        1,
    ));
    assert!(matches!(
        sources(&snapshot, &DocumentCancellationToken::new()),
        Err(ProjectError::Incomplete(_))
    ));
}

#[test]
fn discovery_limits_report_incompleteness_without_raising_quotas() {
    let root = tempfile::tempdir().unwrap();
    for n in 0..=MAX_FILES {
        fs::write(root.path().join(format!("{n}.sk")), "on load:").unwrap();
    }
    let snapshot = ProjectSnapshot {
        epoch: 1,
        root: root.path().into(),
        open: vec![],
    };
    assert!(matches!(
        sources(&snapshot, &DocumentCancellationToken::new()),
        Err(ProjectError::Incomplete(_))
    ));
    let deep = tempfile::tempdir().unwrap();
    let mut directory = deep.path().to_path_buf();
    for _ in 0..=MAX_DEPTH {
        directory.push("d");
        fs::create_dir(&directory).unwrap();
    }
    let snapshot = ProjectSnapshot {
        root: deep.path().into(),
        ..snapshot
    };
    let error = sources(&snapshot, &DocumentCancellationToken::new()).unwrap_err();
    assert!(matches!(error, ProjectError::Incomplete(_)));
    assert!(error.to_string().contains("nesting"));
}

#[test]
fn complete_incomplete_complete_withdraws_imports_and_keeps_native_revisions_monotonic() {
    let root = tempfile::tempdir().unwrap();
    let definition = root.path().join("definition.sk");
    let contents = "function project_value() :: number:\n    return 1\n";
    fs::write(&definition, contents).unwrap();
    let caller = root.path().join("caller.sk");
    let source = "on load:\n    send 1 to console\n    send project_value() to console\n    send 2 to console\n";
    let mut snapshot = ProjectSnapshot {
        epoch: 1,
        root: root.path().into(),
        open: vec![view(&caller, source, 1, 1)],
    };
    let current = snapshot.open[0].uri.clone();
    let (mut host, catalog) = host();
    let mut index = ProjectIndex::new();
    let first = index
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
    assert!(first.incomplete_reason.is_none());
    assert!(
        first
            .result
            .ast
            .nodes
            .iter()
            .any(|node| node.function.is_some())
    );
    fs::write(&definition, [0xff]).unwrap();
    snapshot.epoch += 1;
    let local = index
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
    assert!(local.incomplete_reason.as_ref().unwrap().contains("UTF-8"));
    assert!(!local.current_semantic.project_complete);
    assert_eq!(local.documents.len(), 1);
    assert!(
        local
            .current_semantic
            .project_definition_unavailable(lsp_types::Position::new(2, 12))
    );
    assert!(
        !local
            .current_semantic
            .project_definition_unavailable(lsp_types::Position::new(1, 5))
    );
    assert!(local.native_revision > first.native_revision);
    assert!(
        local
            .result
            .ast
            .nodes
            .iter()
            .all(|node| node.function.is_none())
    );
    let found = super::super::diagnostics::from_result(&current, &local.result).unwrap();
    assert!(
        found
            .iter()
            .all(|d| d.severity != Some(lsp_types::DiagnosticSeverity::ERROR)),
        "{found:#?}"
    );
    let hover = super::super::hover::Document::from_result(
        local.result,
        Arc::from(source),
        Arc::clone(&catalog),
        lsp_types::MarkupKind::PlainText,
    )
    .unwrap();
    assert!(hover.at(lsp_types::Position::new(1, 5)).is_some());
    assert!(hover.at(lsp_types::Position::new(3, 5)).is_some());
    assert!(hover.at(lsp_types::Position::new(2, 12)).is_none());
    fs::write(&definition, contents).unwrap();
    snapshot.epoch += 1;
    let recovered = index
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
    assert!(recovered.incomplete_reason.is_none());
    assert!(recovered.current_semantic.project_complete);
    assert!(recovered.native_revision > local.native_revision);
    assert!(
        recovered
            .result
            .ast
            .nodes
            .iter()
            .any(|node| node.function.is_some())
    );
    assert_eq!(fs::read_to_string(definition).unwrap(), contents);
    assert!(
        !caller.exists(),
        "the open buffer was never written to disk"
    );
}

#[test]
fn nested_boundary_added_after_discovery_cannot_certify_same_byte_exports() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("definitions");
    fs::create_dir(&folder).unwrap();
    fs::write(
        folder.join("definition.sk"),
        "function project_value() :: number:\n    return 1\n",
    )
    .unwrap();
    let caller = root.path().join("caller.sk");
    let snapshot = ProjectSnapshot {
        epoch: 1,
        root: root.path().into(),
        open: vec![view(
            &caller,
            "on load:\n    send project_value() to console\n    send 2 to console\n",
            1,
            1,
        )],
    };
    let current = snapshot.open[0].uri.clone();
    let (mut host, catalog) = host();
    let mut index = ProjectIndex::new();
    let discovered = sources(&snapshot, &DocumentCancellationToken::new()).unwrap();
    let analyzed = index
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
    assert!(analyzed.result.ast.nodes.iter().any(|node| {
        node.function
            .as_ref()
            .is_some_and(|call| call.name == "project_value")
    }));
    let working = index.documents.clone();
    let canonical_root = fs::canonicalize(root.path()).unwrap();
    fs::create_dir(folder.join(".git")).unwrap();
    let error = index
        .finish(
            discovered,
            working,
            BTreeMap::new(),
            BTreeSet::new(),
            analyzed.result,
            analyzed.current_semantic,
            analyzed.native_revision,
            snapshot.epoch,
            analyzed.parsed_documents,
            &canonical_root,
        )
        .err()
        .expect("a retained result cannot cross a new project boundary");
    assert!(matches!(error, ProjectError::Incomplete(_)));
    assert!(error.to_string().contains("nested project"));
    let next = index
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
    assert_eq!(next.documents.len(), 1);
    assert!(
        next.result
            .ast
            .nodes
            .iter()
            .all(|node| node.function.is_none())
    );
}

#[test]
fn missing_workspace_keeps_valid_open_text_without_reading_another_root() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("not-created");
    let caller = root.join("caller.sk");
    let snapshot = ProjectSnapshot {
        epoch: 1,
        root,
        open: vec![view(&caller, "on load:\n    send 1 to console\n", 1, 1)],
    };
    let current = snapshot.open[0].uri.clone();
    let (mut host, catalog) = host();
    let result = ProjectIndex::new()
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
    assert!(result.incomplete_reason.is_some());
    assert_eq!(result.parsed_documents, 1);
    assert!(
        result
            .result
            .ast
            .nodes
            .iter()
            .any(|node| node.text == "send 1 to console" && node.summary.is_some())
    );
    assert!(!caller.exists());
}

#[test]
fn directory_entry_quota_counts_non_sources_and_never_exposes_a_partial_catalog() {
    assert_eq!(MAX_ENTRIES, 50_000);
    let root = tempfile::tempdir().unwrap();
    for n in 0..=MAX_ENTRIES {
        fs::write(root.path().join(format!("{n}.data")), []).unwrap();
    }
    let snapshot = ProjectSnapshot {
        epoch: 1,
        root: root.path().into(),
        open: vec![],
    };
    let error = sources(&snapshot, &DocumentCancellationToken::new()).unwrap_err();
    assert!(matches!(error, ProjectError::Incomplete(_)));
    assert!(error.to_string().contains("50000 directory entries"));
}

#[cfg(windows)]
#[test]
fn windows_junction_is_not_followed_into_or_outside_the_project() {
    let root = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    fs::write(external.path().join("must-not-read.sk"), [0xff]).unwrap();
    let junction = root.path().join("junction");
    let status = std::process::Command::new("cmd")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&junction)
        .arg(external.path())
        .status()
        .unwrap();
    assert!(status.success());
    assert!(link(&fs::symlink_metadata(&junction).unwrap()));
    let snapshot = ProjectSnapshot {
        epoch: 1,
        root: root.path().into(),
        open: vec![],
    };
    assert!(
        sources(&snapshot, &DocumentCancellationToken::new())
            .unwrap()
            .is_empty()
    );
}

#[cfg(windows)]
#[test]
fn windows_open_uri_through_an_inside_junction_degrades_instead_of_admitting_an_alias() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("source.sk"), "on load:\n    send 1\n").unwrap();
    let alias = root.path().join("alias");
    let status = std::process::Command::new("cmd")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&alias)
        .arg(&real)
        .status()
        .unwrap();
    assert!(status.success());
    let snapshot = ProjectSnapshot {
        epoch: 1,
        root: root.path().into(),
        open: vec![view(
            &alias.join("source.sk"),
            "on load:\n    send 2\n",
            1,
            1,
        )],
    };
    assert!(matches!(
        sources(&snapshot, &DocumentCancellationToken::new()),
        Err(ProjectError::Incomplete(_))
    ));
}

#[test]
fn total_source_budget_remains_bounded_and_is_a_discovery_incompleteness() {
    let root = tempfile::tempdir().unwrap();
    let data = vec![b' '; MAX_FILE_BYTES];
    for n in 0..=MAX_SOURCE_BYTES / MAX_FILE_BYTES {
        fs::write(root.path().join(format!("{n}.sk")), &data).unwrap();
    }
    let snapshot = ProjectSnapshot {
        epoch: 1,
        root: root.path().into(),
        open: vec![],
    };
    let error = sources(&snapshot, &DocumentCancellationToken::new()).unwrap_err();
    assert!(matches!(error, ProjectError::Incomplete(_)));
    assert!(error.to_string().contains("source budget"));
}

#[test]
fn cold_unqueried_function_enters_completion_namespace_before_result_reuse() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("definition.sk"),
        "function greet() :: text:\n    return \"hello\"\n",
    )
    .unwrap();
    let source = "on load:\n    send gr\n";
    let snapshot = ProjectSnapshot {
        epoch: 1,
        root: root.path().into(),
        open: vec![view(&root.path().join("caller.sk"), source, 1, 1)],
    };
    let current = snapshot.open[0].uri.clone();
    let (mut host, catalog) = host();
    let mut index = ProjectIndex::new();
    let analyzed = index
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
    assert_eq!(
        analyzed.parsed_documents, 3,
        "new unqueried imports require a final parse with the complete namespace"
    );
    let completions = super::super::completion::CatalogIndex::with_documentation_format(
        &catalog,
        host.raw_tree_options(),
        lsp_types::MarkupKind::PlainText,
    );
    let document = super::super::completion::Document::from_result(
        &analyzed.result,
        Arc::from(source),
        Arc::new(completions),
    )
    .unwrap();
    let proposals = document.at(lsp_types::Position::new(1, 11), false).unwrap();
    assert!(
        proposals
            .items
            .iter()
            .any(|item| item.label.starts_with("greet(")),
        "the actual completion list must include the never-called imported function"
    );
    assert_eq!(analyzed.native_revision, index.revisions[current.as_str()]);
    let next = index
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
    assert_eq!(
        next.parsed_documents, 1,
        "an unchanged current document still obtains its authoritative result once"
    );
}

fn handoff_fixture() -> (tempfile::TempDir, ProjectSnapshot) {
    let root = tempfile::tempdir().unwrap();
    let snapshot = ProjectSnapshot {
        epoch: 11,
        root: root.path().into(),
        open: vec![
            view(
                &root.path().join("foreground.sk"),
                "on load:
    if 1 is 1:
        send 1 to console
    invalid statement here
",
                1,
                1,
            ),
            view(
                &root.path().join("background.sk"),
                "on load:
    send 2 to console
",
                1,
                1,
            ),
        ],
    };
    (root, snapshot)
}
fn produce_handoff(
    index: &mut ProjectIndex,
    host: &mut ParserHost,
    catalog: &Arc<Catalog>,
    snapshot: &ProjectSnapshot,
) {
    index
        .analyze(
            host,
            Arc::clone(catalog),
            "file:///project",
            snapshot,
            &snapshot.open[1].uri,
            Default::default(),
            &mut |_| {},
        )
        .unwrap();
    assert!(
        index.handoff.is_some(),
        "a completed side open document must be available"
    );
}
#[test]
fn completed_side_result_is_transferred_once_with_unchanged_diagnostics_and_hover() {
    let (_root, snapshot) = handoff_fixture();
    let (mut host, catalog) = host();
    let mut index = ProjectIndex::new();
    produce_handoff(&mut index, &mut host, &catalog, &snapshot);
    let revision = index.handoff.as_ref().unwrap().parsed.revision;
    let current = &snapshot.open[0];
    let reused = index
        .analyze(
            &mut host,
            Arc::clone(&catalog),
            "file:///project",
            &snapshot,
            &current.uri,
            Default::default(),
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(reused.parsed_documents, 0);
    assert_eq!(
        reused.native_revision, revision,
        "the original native revision is transferred, never reissued"
    );
    assert!(index.handoff.is_none());
    let actual = super::super::diagnostics::from_result(&current.uri, &reused.result).unwrap();
    assert!(actual.iter().any(|d| d.range.start.line == 3));
    let hover = super::super::hover::Document::from_result(
        reused.result,
        Arc::clone(&current.source),
        Arc::clone(&catalog),
        lsp_types::MarkupKind::PlainText,
    )
    .unwrap();
    let actual_hover = hover.at(lsp_types::Position::new(2, 9));
    assert!(
        actual_hover.is_some(),
        "the nested effect keeps its actual original range"
    );
    let clean = index
        .analyze(
            &mut host,
            Arc::clone(&catalog),
            "file:///project",
            &snapshot,
            &current.uri,
            Default::default(),
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(
        clean.parsed_documents, 1,
        "the handoff cannot be consumed twice"
    );
    let expected = super::super::diagnostics::from_result(&current.uri, &clean.result).unwrap();
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    let clean_hover = super::super::hover::Document::from_result(
        clean.result,
        Arc::clone(&current.source),
        catalog,
        lsp_types::MarkupKind::PlainText,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(actual_hover).unwrap(),
        serde_json::to_value(clean_hover.at(lsp_types::Position::new(2, 9))).unwrap()
    );
}
#[test]
fn result_handoff_rechecks_the_whole_batch_and_semantic_configuration() {
    let (mut host, catalog) = host();
    for change in 0..8 {
        let (_root, mut snapshot) = handoff_fixture();
        let mut index = ProjectIndex::new();
        produce_handoff(&mut index, &mut host, &catalog, &snapshot);
        let retained_revision = index.handoff.as_ref().unwrap().parsed.revision;
        let mut config = DocumentParserConfig::default();
        let mut selected_catalog = Arc::clone(&catalog);
        match change {
            0 => snapshot.epoch += 1,
            1 => snapshot.open[0].generation += 1,
            2 => snapshot.open[0].source_epoch += 1,
            3 => {
                snapshot.open[0].source = Arc::from(
                    "on load:
    send 3 to console
",
                )
            }
            4 => snapshot.open[1].generation += 1,
            5 => config.raw_tree = Some(skript_parser::RawTreeOptions::for_skript_version(2, 8)),
            6 => config.structure.headers_only = true,
            7 => selected_catalog = self::host().1,
            _ => unreachable!(),
        }
        let analyzed = index
            .analyze(
                &mut host,
                selected_catalog,
                "file:///project",
                &snapshot,
                &snapshot.open[0].uri,
                config,
                &mut |_| {},
            )
            .unwrap();
        assert!(
            analyzed.native_revision > retained_revision,
            "the current document itself must be reparsed for changed input {change}"
        );
        assert!(
            analyzed.parsed_documents > 0,
            "changed batch/config input {change} must not consume the old result"
        );
    }
}
#[test]
fn handoff_discovers_new_uncalled_imports_even_without_a_new_project_epoch() {
    let (root, snapshot) = handoff_fixture();
    let (mut host, catalog) = host();
    let mut index = ProjectIndex::new();
    produce_handoff(&mut index, &mut host, &catalog, &snapshot);
    fs::write(
        root.path().join("new.sk"),
        "function greet() :: text:
    return \"hello\"
",
    )
    .unwrap();
    let analyzed = index
        .analyze(
            &mut host,
            Arc::clone(&catalog),
            "file:///project",
            &snapshot,
            &snapshot.open[0].uri,
            Default::default(),
            &mut |_| {},
        )
        .unwrap();
    assert!(
        analyzed.parsed_documents >= 2,
        "fresh closed-file discovery changes the full import namespace"
    );
    assert!(
        analyzed
            .result
            .functions
            .imports()
            .iter()
            .any(|item| item.definition.name == "greet")
    );
}
#[test]
fn cancellation_release_and_native_revision_replacement_discard_handoffs() {
    let (mut host, catalog) = host();
    for change in 0..3 {
        let (_root, snapshot) = handoff_fixture();
        let mut index = ProjectIndex::new();
        produce_handoff(&mut index, &mut host, &catalog, &snapshot);
        let current = &snapshot.open[0].uri;
        let config = DocumentParserConfig::default();
        match change {
            0 => config.cancellation.cancel(),
            1 => index
                .release_uri(&mut host, "file:///project", current)
                .unwrap(),
            2 => {
                let source = sources(&snapshot, &config.cancellation)
                    .unwrap()
                    .remove(current.as_str())
                    .unwrap();
                index
                    .parse(
                        &mut host,
                        Arc::clone(&catalog),
                        "file:///project",
                        &source,
                        config.clone(),
                        Vec::new(),
                        snapshot.epoch,
                        None,
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let result = index.analyze(
            &mut host,
            Arc::clone(&catalog),
            "file:///project",
            &snapshot,
            current,
            config,
            &mut |_| {},
        );
        if change == 0 {
            assert!(matches!(result, Err(ProjectError::Cancelled)));
            assert!(index.handoff.is_none());
        } else {
            assert!(result.unwrap().parsed_documents > 0);
        }
    }
}
#[test]
fn incomplete_discovery_discards_a_prior_handoff() {
    let (root, snapshot) = handoff_fixture();
    let (mut host, catalog) = host();
    let mut index = ProjectIndex::new();
    produce_handoff(&mut index, &mut host, &catalog, &snapshot);
    fs::create_dir(root.path().join(".git")).unwrap();
    // A nested boundary created after the producer must be discovered again.
    let nested = root.path().join("nested");
    fs::create_dir(&nested).unwrap();
    let mut changed = snapshot.clone();
    changed.open[0].uri = path_uri(&nested.join("foreground.sk")).unwrap();
    fs::create_dir(nested.join(".git")).unwrap();
    let local = index
        .analyze(
            &mut host,
            catalog,
            "file:///project",
            &changed,
            &changed.open[0].uri,
            Default::default(),
            &mut |_| {},
        )
        .unwrap();
    assert!(local.incomplete_reason.is_some());
    assert!(index.handoff.is_none());
}

fn function_handoff_fixture() -> (tempfile::TempDir, ProjectSnapshot) {
    let (root, mut snapshot) = handoff_fixture();
    snapshot.open[0].source = Arc::from(
        "function from_foreground() :: number:\n    return 2\non load:\n    send from_background() to console\n    send fr\n",
    );
    snapshot.open[1].source = Arc::from(
        "function from_background() :: number:\n    return 1\non load:\n    send 2 to console\n",
    );
    (root, snapshot)
}
#[test]
fn side_handoff_retains_imported_call_proofs_after_provider_revision_changes() {
    let (_root, snapshot) = function_handoff_fixture();
    let (mut host, catalog) = host();
    let mut index = ProjectIndex::new();
    produce_handoff(&mut index, &mut host, &catalog, &snapshot);
    let original = &index.handoff.as_ref().unwrap().parsed;
    let imported_revision = original
        .result
        .functions
        .imports()
        .iter()
        .find(|function| function.definition.name == "from_background")
        .unwrap()
        .document_revision;
    assert!(
        index.revisions[snapshot.open[1].uri.as_str()] > imported_revision,
        "the producer's final full-import parse must have advanced the provider revision"
    );
    let current = &snapshot.open[0];
    let reused = index
        .analyze(
            &mut host,
            Arc::clone(&catalog),
            "file:///project",
            &snapshot,
            &current.uri,
            Default::default(),
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(reused.parsed_documents, 0);
    let project = reused.documents.iter().map(Arc::as_ref).collect::<Vec<_>>();
    let targets = reused
        .current_semantic
        .definitions_at(lsp_types::Position::new(3, 15), &project);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].uri, snapshot.open[1].uri);
    assert_eq!(targets[0].range.start.line, 0);
    let completion_index = Arc::new(
        super::super::completion::CatalogIndex::with_documentation_format(
            &catalog,
            host.raw_tree_options(),
            lsp_types::MarkupKind::PlainText,
        ),
    );
    let completion = super::super::completion::Document::from_result(
        &reused.result,
        Arc::clone(&current.source),
        completion_index,
    )
    .unwrap();
    let items = completion
        .at(lsp_types::Position::new(4, 11), false)
        .unwrap();
    assert!(
        items
            .items
            .iter()
            .any(|item| item.label.starts_with("from_background("))
    );
    assert!(
        items
            .items
            .iter()
            .any(|item| item.label.starts_with("from_foreground("))
    );
    let actual_diagnostics =
        super::super::diagnostics::from_result(&current.uri, &reused.result).unwrap();
    let actual_hover = super::super::hover::Document::from_result(
        reused.result,
        Arc::clone(&current.source),
        Arc::clone(&catalog),
        lsp_types::MarkupKind::PlainText,
    )
    .unwrap()
    .at(lsp_types::Position::new(3, 15));
    assert!(
        serde_json::to_string(&actual_hover)
            .unwrap()
            .contains("from_background() -> number")
    );
    let clean = index
        .analyze(
            &mut host,
            Arc::clone(&catalog),
            "file:///project",
            &snapshot,
            &current.uri,
            Default::default(),
            &mut |_| {},
        )
        .unwrap();
    assert_eq!(clean.parsed_documents, 1);
    let clean_diagnostics =
        super::super::diagnostics::from_result(&current.uri, &clean.result).unwrap();
    assert_eq!(
        serde_json::to_value(actual_diagnostics).unwrap(),
        serde_json::to_value(clean_diagnostics).unwrap()
    );
    let clean_hover = super::super::hover::Document::from_result(
        clean.result,
        Arc::clone(&current.source),
        catalog,
        lsp_types::MarkupKind::PlainText,
    )
    .unwrap()
    .at(lsp_types::Position::new(3, 15));
    assert_eq!(
        serde_json::to_value(actual_hover).unwrap(),
        serde_json::to_value(clean_hover).unwrap()
    );
}
#[test]
fn producer_cancellation_and_final_membership_failure_discard_staged_side_results() {
    let (mut host, catalog) = host();
    for cancel in [false, true] {
        let (root, mut snapshot) = function_handoff_fixture();
        let nested = root.path().join("nested");
        fs::create_dir(&nested).unwrap();
        snapshot.open[0].uri = path_uri(&nested.join("foreground.sk")).unwrap();
        let mut index = ProjectIndex::new();
        let config = DocumentParserConfig::default();
        let cancellation = config.cancellation.clone();
        let mut observed = false;
        let result = index.analyze(
            &mut host,
            Arc::clone(&catalog),
            "file:///project",
            &snapshot,
            &snapshot.open[1].uri,
            config,
            &mut |event| {
                if matches!(event, StructurePreparationEvent::Headers { .. }) && !observed {
                    observed = true;
                    if cancel {
                        cancellation.cancel();
                    } else {
                        fs::create_dir(nested.join(".git")).unwrap();
                    }
                }
            },
        );
        assert!(
            observed,
            "the producer must reach final parsing after staging its side document"
        );
        if cancel {
            assert!(matches!(result, Err(ProjectError::Cancelled)));
        } else {
            let local = result.unwrap();
            assert!(local.incomplete_reason.is_some());
            assert_eq!(local.documents.len(), 1);
            assert!(!local.current_semantic.project_complete);
        }
        assert!(
            index.handoff.is_none(),
            "no staged data may escape a cancelled or incomplete producer"
        );
    }
}

//! Actual ParserAnalyzer outputs after changed input versus a fresh current parse.
use super::*;
use lsp_types::{DiagnosticSeverity, MarkupKind, NumberOrString, Position, Range};

fn analyzer() -> ParserAnalyzer {
    let catalog = crate::lsp::test_support::runtime_catalog();
    let host = crate::new_parser_host(HostConfig {
        syntax_catalog: Some(Arc::clone(&catalog)),
        runtime_profile: parser_wasm::RuntimeProfile {
            skript_version: Some("2.15.4".into()),
            ..Default::default()
        },
        ..HostConfig::default()
    })
    .unwrap();
    let completions = Arc::new(completion::CatalogIndex::with_documentation_format(
        &catalog,
        host.raw_tree_options(),
        MarkupKind::Markdown,
    ));
    ParserAnalyzer {
        project_uri: "file:///workspace".into(),
        host,
        catalog,
        hover_format: MarkupKind::Markdown,
        semantic_enabled: true,
        completions,
        prepare_completion: false,
        project: super::super::project::ProjectIndex::default(),
    }
}

fn analyze(analyzer: &mut ParserAnalyzer, source: &str, revision: u64) -> Analysis {
    analyzer.analyze(&Job {
        uri: "file:///workspace/output-equivalence.sk".parse().unwrap(),
        project: None,
        generation: 1,
        revision,
        version: revision as i32,
        source: Arc::from(source),
        control: Control::new(),
        queued_at: Instant::now(),
    })
}

#[test]
fn rename_uses_function_identity_and_rejects_collisions_and_incomplete_coverage() {
    let source = "function first(value: number) :: number:\n    return {_value}\nfunction second(value: number) :: number:\n    return {_value}\non join:\n    send first(1)\n";
    let mut parser = analyzer();
    let result = analyze(&mut parser, source, 1);
    let semantic = result.semantic.as_ref().unwrap();
    assert!(
        diagnostics(&result).0.is_empty(),
        "{:?}",
        diagnostics(&result).0
    );
    let at = position(source, source.rfind("first(1)").unwrap());
    let project = result
        .project_semantics
        .iter()
        .map(Arc::as_ref)
        .collect::<Vec<_>>();
    let plan = semantic
        .rename_plan(at, &project, Some("renamed"))
        .unwrap()
        .unwrap();
    assert_eq!(plan.placeholder, "first");
    assert_eq!(plan.edits.len(), 2);
    assert_eq!(
        plan.edits
            .iter()
            .map(|edit| edit.range.start.line)
            .collect::<Vec<_>>(),
        vec![0, 5]
    );
    for name in ["second", "max", "not a name", "2invalid"] {
        assert!(
            semantic.rename_plan(at, &project, Some(name)).is_err(),
            "{name}"
        );
    }
    let modified = format!("{source}    this is not valid syntax\n");
    let incomplete = analyze(&mut parser, &modified, 2);
    let incomplete = incomplete.semantic.as_ref().unwrap();
    assert!(incomplete.rename_plan(at, &[], Some("renamed")).is_err());
}

#[test]
fn scalar_local_rename_stays_in_its_execution_scope_and_preserves_name_geometry() {
    let source = "on join:\n    set {_item} to 1\n    if 1 is 1:\n        send {_item}\non quit:\n    set {_item} to 2\n    send {_item}\n";
    let result = analyze(&mut analyzer(), source, 1);
    let semantic = result.semantic.as_ref().unwrap();
    assert!(
        diagnostics(&result).0.is_empty(),
        "{:?}",
        diagnostics(&result).0
    );
    let at = position(source, source.find("_item").unwrap() + 1);
    assert_eq!(semantic.references_at(at, &[], true).len(), 2);
    let plan = semantic
        .rename_plan(at, &[], Some("renamed"))
        .unwrap()
        .unwrap();
    assert_eq!(plan.placeholder, "item");
    assert_eq!(plan.edits.len(), 2);
    assert_eq!(
        plan.edits
            .iter()
            .map(|edit| edit.range.start.line)
            .collect::<Vec<_>>(),
        vec![1, 3]
    );
    for edit in &plan.edits {
        assert_eq!(
            edit.range.end.character - edit.range.start.character,
            4,
            "braces and local prefix must remain untouched"
        );
    }
    assert!(semantic.rename_plan(at, &[], Some("not a name")).is_err());
}

#[test]
fn local_rename_refuses_case_alias_dynamic_list_and_parameter_bindings() {
    for source in [
        "on join:\n    set {_item} to 1\n    send {_ITEM}\n",
        "on join:\n    set {_item} to 1\n    send {_%player%}\n",
        "on join:\n    set {_item::*} to 1, 2\n    send {_item::*}\n",
        "function identity(item: number) :: number:\n    return {_item}\non join:\n    send identity(1)\n",
    ] {
        let result = analyze(&mut analyzer(), source, 1);
        let semantic = result.semantic.as_ref().unwrap();
        let offset = source.find("{_item").unwrap() + 2;
        assert!(
            semantic
                .rename_plan(position(source, offset), &[], Some("renamed"))
                .is_err(),
            "{source}"
        );
    }
}

fn diagnostics(analysis: &Analysis) -> (&[Diagnostic], &str) {
    assert!(
        analysis.annotations_error.is_none(),
        "{:?}",
        analysis.annotations_error
    );
    assert!(
        analysis.semantic_error.is_none(),
        "{:?}",
        analysis.semantic_error
    );
    assert!(
        analysis.completion_error.is_none(),
        "{:?}",
        analysis.completion_error
    );
    let Outcome::Diagnostics(diagnostics, status) = &analysis.outcome else {
        panic!("a current successful analysis is required");
    };
    (diagnostics, status)
}

fn position(source: &str, offset: usize) -> Position {
    super::super::positions::LineIndex::new(source)
        .position(source, offset)
        .unwrap()
}

fn semantic_token_legend() -> semantic_tokens::Legend {
    let capabilities = serde_json::from_value(json!({
        "requests":{"full":true}, "formats":["relative"], "tokenModifiers":[],
        "tokenTypes":["skriptEvent","skriptCondition","skriptEffect","skriptExpression","skriptSection","skriptStructure","skriptLiteral","function","variable","skriptInterpolationDelimiter"]
    })).unwrap();
    semantic_tokens::Legend::negotiate(Some(&capabilities)).unwrap()
}

#[test]
fn actual_parser_analyzer_colors_typed_condition_and_effect_with_native_structural_evidence() {
    let source = "on join:\n    if 1 is 2:\n        send \"marker\" to player\n";
    let mut parser = analyzer();
    let result = parser
        .host
        .parse_document(
            parser_wasm::DocumentParseRequest::new(
                "file:///workspace",
                "file:///workspace/typed-colors.sk",
                1,
                source,
            ),
            parser_wasm::DocumentParserConfig::default(),
        )
        .unwrap();
    assert!(
        result.component_failures.is_empty(),
        "{:?}",
        result.component_failures
    );
    assert!(
        result.syntax.diagnostics.is_empty(),
        "{:?}",
        result.syntax.diagnostics
    );
    let witnesses = result.semantic_witnesses();
    for kind in ["effect", "condition"] {
        let node = result
            .ast
            .nodes
            .iter()
            .find(|node| {
                node.summary
                    .as_ref()
                    .is_some_and(|summary| summary.kind == kind)
            })
            .expect("the genuine typed statement must actually parse");
        assert!(witnesses.native_valid(node));
        assert!(
            node.captures.iter().any(|capture| {
                parser_wasm::host::source_geometry::pattern_expected_types(node, capture).is_some()
            }),
            "actual {kind} must carry native structural Text evidence"
        );
    }
    let analysis = analyze(&mut parser, source, 2);
    assert!(diagnostics(&analysis).0.is_empty());
    assert_eq!(diagnostics(&analysis).1, "complete");
    let legend = semantic_token_legend();
    let wire = legend.wire();
    let tokens = analysis
        .semantic_tokens
        .as_ref()
        .expect("actual semantic token output")
        .full(&legend);
    let mut row = 0;
    let mut column = 0;
    let mut decoded = Vec::new();
    for token in tokens.data {
        if token.delta_line == 0 {
            column += token.delta_start;
        } else {
            row += token.delta_line;
            column = token.delta_start;
        }
        decoded.push((
            row,
            column,
            token.length,
            wire.token_types[token.token_type as usize]
                .as_str()
                .to_owned(),
        ));
    }
    for expected in [
        (1, 7, 1, "skriptLiteral"),
        (1, 9, 2, "skriptCondition"),
        (1, 12, 1, "skriptLiteral"),
        (2, 8, 4, "skriptEffect"),
        (2, 13, 8, "skriptLiteral"),
        (2, 22, 2, "skriptEffect"),
    ] {
        assert!(
            decoded.iter().any(|token| token.0 == expected.0
                && token.1 == expected.1
                && token.2 == expected.2
                && token.3 == expected.3),
            "missing actual token {expected:?}: {decoded:?}"
        );
    }
}

fn annotation_outputs(analysis: &Analysis, source: &str) -> Value {
    let semantic = analysis.semantic.as_ref().expect("actual semantic output");
    let project = analysis
        .project_semantics
        .iter()
        .map(Arc::as_ref)
        .collect::<Vec<_>>();
    let native_hover = analysis.hover.as_ref().expect("actual native hover output");
    let queries = source
        .char_indices()
        .map(|(offset, _)| {
            let at = position(source, offset);
            json!({
                "position": at,
                "hover": native_hover.at(at),
                "inferredHover": semantic.hover(at),
                "type": semantic.type_at(at).map(|(range, ty)| (range, format!("{ty:?}"))),
                "definitions": semantic.definitions_at(at, &project),
                "references": semantic.references_at(at, &project, false),
                "referencesWithDeclarations": semantic.references_at(at, &project, true),
            })
        })
        .collect::<Vec<_>>();
    let full_range = Range::new(Position::new(0, 0), position(source, source.len()));
    json!({
        "queries": queries,
        "semanticTokens": analysis.semantic_tokens.as_ref().expect("actual semantic token output").full(&semantic_token_legend()),
        "inlay": analysis.inlay.as_ref().expect("actual inlay output").at(full_range),
    })
}

#[test]
fn real_parser_analyzer_reuse_preserves_current_lsp_outputs() {
    // The untouched first statement stays at the same physical range, while
    // the later write changes the authoritative type and diagnostic evidence.
    let before = "on join:\n    send 1\n    set {_place} to 1\n    teleport player to {_place}\n";
    let current = before.replace("to 1\n    teleport", "to \"bad\"\n    teleport");
    let mut warm_analyzer = analyzer();
    let initial = analyze(&mut warm_analyzer, before, 1);
    assert_eq!(diagnostics(&initial).1, "complete");
    let old_read = position(before, before.rfind("{_place}").unwrap() + 2);
    assert!(
        matches!(
            initial.semantic.as_ref().unwrap().type_at(old_read),
            Some((_, semantic::InferredType::Values { classes, maybe_unset: false, .. }))
                if classes.len() == 1 && classes.contains("java.lang.Long")
        ),
        "initial integer literal must establish actual java.lang.Long"
    );

    let warm = analyze(&mut warm_analyzer, &current, 2);
    let mut clean_analyzer = analyzer();
    let clean = analyze(&mut clean_analyzer, &current, 2);
    let (warm_diagnostics, warm_status) = diagnostics(&warm);
    let (clean_diagnostics, clean_status) = diagnostics(&clean);
    assert_eq!(warm_status, "complete");
    assert_eq!(warm_status, clean_status);
    assert_eq!(
        serde_json::to_value(warm_diagnostics).unwrap(),
        serde_json::to_value(clean_diagnostics).unwrap()
    );
    assert!(
        warm.analysis_metrics["incremental"]["hits"]
            .as_u64()
            .unwrap()
            > 0,
        "untouched current-source work must actually be reused: {}",
        warm.analysis_metrics
    );
    assert_eq!(clean.analysis_metrics["incremental"]["hits"], 0);
    assert_eq!(
        annotation_outputs(&warm, &current),
        annotation_outputs(&clean, &current)
    );

    let read = position(&current, current.rfind("{_place}").unwrap() + 2);
    let write = position(&current, current.find("{_place}").unwrap() + 2);
    let semantic = warm.semantic.as_ref().unwrap();
    let project = warm
        .project_semantics
        .iter()
        .map(Arc::as_ref)
        .collect::<Vec<_>>();
    assert!(
        matches!(semantic.type_at(read),
        Some((_, semantic::InferredType::Values { classes, maybe_unset: false, .. }))
            if classes.len() == 1 && classes.contains("java.lang.String")),
        "changed input must establish the current String type, not the prior Long"
    );
    assert!(semantic.hover(read).is_some());
    assert!(
        semantic
            .definitions_at(read, &project)
            .iter()
            .any(|location| location.range.start <= write && write < location.range.end)
    );
    assert!(semantic.references_at(read, &project, true).len() >= 2);
    assert!(
        warm_diagnostics.iter().any(|diagnostic| diagnostic.severity
            == Some(DiagnosticSeverity::WARNING)
            && diagnostic.code
                == Some(NumberOrString::String(
                    "semantic.variable-type-mismatch".into()
                ))
            && diagnostic.range.start <= read
            && read < diagnostic.range.end
            && diagnostic
                .data
                .as_ref()
                .is_some_and(|data| data["actualTypes"]
                    .as_array()
                    .is_some_and(|types| types.contains(&json!("java.lang.String")))
                    && data["expectedTypes"]
                        .as_array()
                        .is_some_and(|types| !types.is_empty()))),
        "actual inferred diagnostic must preserve expected/actual evidence: {warm_diagnostics:?}"
    );
    let hints = warm.inlay.as_ref().unwrap().at(Range::new(
        Position::new(0, 0),
        position(&current, current.len()),
    ));
    assert!(hints.iter().any(|hint|
        hint.position == Position::new(1, 10)
        && matches!(&hint.label, lsp_types::InlayHintLabel::String(label) if label == "to <event player>")),
        "actual omitted recipient hint must survive reuse: {hints:?}");
    eprintln!(
        "lsp-output-equivalence,hits={},diagnostics={},definitions={},references={},inlay={}",
        warm.analysis_metrics["incremental"]["hits"],
        warm_diagnostics.len(),
        semantic.definitions_at(read, &project).len(),
        semantic.references_at(read, &project, true).len(),
        hints.len(),
    );
}

#[test]
fn genuine_chance_condition_keeps_native_slot_and_both_reaching_writes() {
    let source = "on join:\n    set {_destination} to \"home\"\n    if chance of 50%:\n        set {_destination} to \"away\"\n    teleport player to {_destination}\n";
    let document_id = super::document_key("file:///workspace/output-equivalence.sk");
    let mut parser = analyzer();
    let result = parser
        .host
        .parse_document(
            parser_wasm::DocumentParseRequest::new("file:///workspace", document_id, 1, source),
            parser_wasm::DocumentParserConfig::default(),
        )
        .unwrap();
    assert_eq!(
        result.execution_status,
        parser_wasm::DocumentExecutionStatus::Complete
    );
    assert!(
        result.component_failures.is_empty(),
        "{:?}",
        result.component_failures
    );
    assert!(
        result.fault_regions.is_empty(),
        "{:?}",
        result.fault_regions
    );
    assert!(
        result.syntax.diagnostics.is_empty(),
        "{:?}",
        result.syntax.diagnostics
    );
    let condition = result
        .ast
        .nodes
        .iter()
        .find(|node| {
            node.summary
                .as_ref()
                .is_some_and(|summary| summary.kind == "condition")
                && node
                    .metadata
                    .iter()
                    .any(|entry| entry.key == "semantic-mode" && entry.value == "chance")
        })
        .expect("the actual Core CondChance must be selected");
    let registration = condition
        .summary
        .as_ref()
        .unwrap()
        .registration_id
        .as_deref()
        .unwrap();
    assert!(
        parser
            .catalog
            .source()
            .unwrap()
            .records_by_registration_id(registration)
            .iter()
            .any(|record| {
                serde_json::from_slice::<Value>(&record.json).unwrap()["elementClass"]
                    == "ch.njol.skript.conditions.CondChance"
            })
    );
    assert!(result.semantic_witnesses().native_valid(condition));
    let nodes = result
        .ast
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<std::collections::HashMap<_, _>>();
    let child = semantic::capture_node(condition, 0, &nodes)
        .and_then(|id| nodes.get(&id).copied())
        .expect("the native numeric capture must own its actual expression child");
    assert_eq!(child.text, "50");
    let start = source.find("50").unwrap();
    assert_eq!(child.span.virtual_range.start, start as u64);
    assert_eq!(child.span.virtual_range.end, (start + 2) as u64);

    let analysis = analyze(&mut parser, source, 2);
    let (actual_diagnostics, status) = diagnostics(&analysis);
    assert_eq!(status, "complete");
    let read = position(source, source.rfind("{_destination}").unwrap() + 2);
    let document = analysis.semantic.as_ref().unwrap();
    assert!(
        matches!(document.type_at(read),
        Some((_, semantic::InferredType::Values { classes, maybe_unset: false, .. }))
            if classes.len() == 1 && classes.contains("java.lang.String")),
        "actual chance evaluation must preserve both known String writes"
    );
    let definitions = document.definitions_at(read, &[]);
    let mut rows = definitions
        .iter()
        .map(|value| value.range.start.line)
        .collect::<Vec<_>>();
    rows.sort_unstable();
    assert_eq!(
        rows,
        vec![1, 3],
        "both branches must remain genuine reaching definitions"
    );
    assert_eq!(actual_diagnostics.len(), 1, "{actual_diagnostics:#?}");
    let diagnostic = &actual_diagnostics[0];
    assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::WARNING));
    assert_eq!(
        diagnostic.code,
        Some(NumberOrString::String(
            "semantic.variable-type-mismatch".into()
        ))
    );
    let data = diagnostic.data.as_ref().unwrap();
    assert_eq!(data["parserAccepted"], true);
    assert_eq!(data["actualTypes"], json!(["java.lang.String"]));
    assert!(
        data["expectedTypes"]
            .as_array()
            .unwrap()
            .contains(&json!("org.bukkit.Location"))
    );
    for row in [1, 3] {
        assert!(
            diagnostic
                .related_information
                .as_ref()
                .unwrap()
                .iter()
                .any(|related| related.location.range.start.line == row)
        );
    }
}

#[test]
fn actual_imported_void_statement_preserves_lsp_identity_and_is_a_flow_barrier() {
    use super::super::project::{ProjectSnapshot, ProjectSource};

    // Existing analyzer() loads the genuine95c5 typeRuntime fixture. Baseline
    // warning proves local String inference is active before checking the call.
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let definition_path = root.join("definition.sk");
    let definition_source = "function imported_void():\n    send \"marker\" to console\n";
    std::fs::write(&definition_path, definition_source).unwrap();
    let definition_uri: lsp_types::Uri = url::Url::from_file_path(&definition_path)
        .unwrap()
        .as_str()
        .parse()
        .unwrap();
    let caller_uri: lsp_types::Uri = url::Url::from_file_path(root.join("caller.sk"))
        .unwrap()
        .as_str()
        .parse()
        .unwrap();
    let mut parser = analyzer();
    parser.project_uri = url::Url::from_directory_path(&root).unwrap().to_string();
    let job = |source: &str, revision: u64| Job {
        uri: caller_uri.clone(),
        project: Some(Arc::new(ProjectSnapshot {
            epoch: revision,
            root: root.clone(),
            open: vec![ProjectSource {
                uri: caller_uri.clone(),
                generation: 1,
                source_epoch: revision,
                source: Arc::from(source),
            }],
        })),
        generation: 1,
        revision,
        version: revision as i32,
        source: Arc::from(source),
        control: Control::new(),
        queued_at: Instant::now(),
    };
    let before = "on join:\n    set {_t} to \"somewhere\"\n    teleport player to {_t}\n";
    let initial = parser.analyze(&job(before, 1));
    let (initial_diagnostics, initial_status) = diagnostics(&initial);
    assert_eq!(initial_status, "complete");
    let initial_read = position(before, before.rfind("{_t}").unwrap() + 2);
    assert!(matches!(
        initial.semantic.as_ref().unwrap().type_at(initial_read),
        Some((_, semantic::InferredType::Values { classes, maybe_unset: false, .. }))
            if classes.len() == 1 && classes.contains("java.lang.String")
    ));
    assert_eq!(initial_diagnostics.len(), 1, "{initial_diagnostics:#?}");
    assert_eq!(
        initial_diagnostics[0].code,
        Some(NumberOrString::String(
            "semantic.variable-type-mismatch".into()
        ))
    );
    assert_eq!(
        initial_diagnostics[0].data.as_ref().unwrap()["actualTypes"],
        json!(["java.lang.String"])
    );

    let current = "on join:\n    set {_t} to \"somewhere\"\n    imported_void()\n    teleport player to {_t}\n";
    let analyzed = parser.analyze(&job(current, 2));
    let (found, status) = diagnostics(&analyzed);
    assert_eq!(status, "complete");
    assert!(
        found.is_empty(),
        "valid call must not leave an unclaimed node or stale warning: {found:#?}"
    );
    let semantic = analyzed.semantic.as_ref().unwrap();
    let write = position(current, current.find("{_t}").unwrap() + 2);
    assert!(matches!(
        semantic.type_at(write),
        Some((_, semantic::InferredType::Values { classes, maybe_unset: false, .. }))
            if classes.len() == 1 && classes.contains("java.lang.String")
    ));
    let read = position(current, current.rfind("{_t}").unwrap() + 2);
    assert!(matches!(
        semantic.type_at(read),
        Some((_, semantic::InferredType::Unknown))
    ));
    let project = analyzed
        .project_semantics
        .iter()
        .map(Arc::as_ref)
        .collect::<Vec<_>>();
    let owner = project
        .iter()
        .copied()
        .find(|document| document.uri == definition_uri)
        .unwrap();
    let symbol = owner
        .functions
        .iter()
        .find(|symbol| symbol.definition.name == "imported_void")
        .unwrap();
    assert!(symbol.definition.return_type.is_none());
    assert_eq!(
        symbol.definition.metadata.get("function.document-id"),
        Some(&super::super::project::document_key(
            definition_uri.as_str()
        ))
    );
    assert_eq!(
        owner.source_witness,
        format!("{:x}", Sha256::digest(definition_source.as_bytes()))
    );
    let call_start = current.find("imported_void").unwrap();
    let call_at = position(current, call_start + 1);
    let call_range = Range::new(
        position(current, call_start),
        position(current, call_start + "imported_void".len()),
    );
    let targets = semantic.definitions_at(call_at, &project);
    assert_eq!(targets.len(), 1, "{targets:#?}");
    assert_eq!(targets[0].uri, definition_uri);
    assert_eq!(targets[0].range, symbol.range);
    let references = semantic.references_at(call_at, &project, false);
    assert_eq!(
        references,
        vec![lsp_types::Location::new(caller_uri.clone(), call_range)]
    );
    let all_references = semantic.references_at(call_at, &project, true);
    assert_eq!(all_references.len(), 2, "{all_references:#?}");
    assert!(all_references.contains(&lsp_types::Location::new(
        definition_uri.clone(),
        symbol.name_range.unwrap()
    )));
    assert_eq!(
        owner.references_at(symbol.name_range.unwrap().start, &project, false),
        references
    );
    let hover = analyzed
        .hover
        .as_ref()
        .unwrap()
        .at(call_at)
        .expect("real void statement hover");
    let hover = serde_json::to_string(&hover).unwrap();
    assert!(
        hover.contains("imported_void") && hover.contains("Signature"),
        "{hover}"
    );
    let legend = semantic_token_legend();
    let wire = legend.wire();
    let tokens = analyzed.semantic_tokens.as_ref().unwrap().full(&legend);
    let mut row = 0;
    let mut column = 0;
    let mut function_tokens = Vec::new();
    for token in tokens.data {
        if token.delta_line == 0 {
            column += token.delta_start;
        } else {
            row += token.delta_line;
            column = token.delta_start;
        }
        if wire.token_types[token.token_type as usize].as_str() == "function" {
            function_tokens.push((row, column, token.length));
        }
    }
    assert_eq!(function_tokens, vec![(2, 4, "imported_void".len() as u32)]);
}

#[test]
fn local_rename_includes_string_interpolation_with_utf16_name_geometry() {
    let source = "on join:\n    set {_item} to 1\n    send \"😀 %{_item}%\"\n";
    let analyzed = analyze(&mut analyzer(), source, 1);
    assert!(
        diagnostics(&analyzed).0.is_empty(),
        "{:?}",
        diagnostics(&analyzed).0
    );
    let document = analyzed.semantic.as_ref().unwrap();
    let at = position(source, source.find("_item").unwrap() + 1);
    let plan = document
        .rename_plan(at, &[], Some("renamed"))
        .unwrap()
        .unwrap();
    assert_eq!(plan.edits.len(), 2);
    let start = position(source, source.rfind("_item").unwrap() + 1);
    let end = Position::new(start.line, start.character + 4);
    assert_eq!(plan.edits[1].range, Range::new(start, end));
}

#[test]
fn function_rename_rejects_actual_source_macro_expansion() {
    let source = "options:\n    value: 1\nfunction first() :: number:\n    return {@value}\non join:\n    send first()\n";
    let analyzed = analyze(&mut analyzer(), source, 1);
    assert!(
        diagnostics(&analyzed).0.is_empty(),
        "{:?}",
        diagnostics(&analyzed).0
    );
    let document = analyzed.semantic.as_ref().unwrap();
    let at = position(source, source.rfind("first()").unwrap());
    assert!(document.rename_plan(at, &[], Some("renamed")).is_err());
}

#[test]
fn function_rename_collects_exact_cross_document_bindings() {
    let folder = tempfile::tempdir().unwrap();
    let definition_path = folder.path().join("definition.sk");
    let caller_path = folder.path().join("caller.sk");
    let definition_source = "function cross_value(value: number) :: number:\n    return {_value}\n";
    let caller_source = "on join:\n    send cross_value(2)\n";
    std::fs::write(&definition_path, definition_source).unwrap();
    std::fs::write(&caller_path, caller_source).unwrap();
    let uri = |path: &std::path::Path| -> lsp_types::Uri {
        url::Url::from_file_path(path)
            .unwrap()
            .as_str()
            .parse()
            .unwrap()
    };
    let definition_uri = uri(&definition_path);
    let caller_uri = uri(&caller_path);
    let snapshot = super::super::project::ProjectSnapshot {
        epoch: 1,
        root: folder.path().into(),
        open: vec![super::super::project::ProjectSource {
            uri: caller_uri.clone(),
            generation: 1,
            source_epoch: 1,
            source: Arc::from(caller_source),
        }],
    };
    let mut parser = analyzer();
    let analyzed = parser.analyze(&Job {
        uri: caller_uri.clone(),
        project: Some(Arc::new(snapshot)),
        generation: 1,
        revision: 1,
        version: 1,
        source: Arc::from(caller_source),
        control: Control::new(),
        queued_at: Instant::now(),
    });
    assert!(
        diagnostics(&analyzed).0.is_empty(),
        "{:?}",
        diagnostics(&analyzed).0
    );
    let document = analyzed.semantic.as_ref().unwrap();
    let project = analyzed
        .project_semantics
        .iter()
        .map(Arc::as_ref)
        .collect::<Vec<_>>();
    let at = position(
        caller_source,
        caller_source.find("cross_value").unwrap() + 2,
    );
    let plan = document
        .rename_plan(at, &project, Some("renamed"))
        .unwrap()
        .unwrap();
    assert_eq!(plan.edits.len(), 2);
    assert_eq!(plan.edits[0].uri, caller_uri);
    assert_eq!(plan.edits[1].uri, definition_uri);
}

#[test]
fn rename_rejects_previously_noop_macro_without_name_stability_contract() {
    let source = "function before(value: number) :: number:\n    return {_value}\non join:\n    send before(1)\n";
    let mut parser = analyzer();
    parser
        .host
        .load_addon(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/artifacts/text-macro-addon.wasm"
        )))
        .unwrap();
    let analyzed = analyze(&mut parser, source, 1);
    assert!(
        diagnostics(&analyzed).0.is_empty(),
        "{:?}",
        diagnostics(&analyzed).0
    );
    let document = analyzed.semantic.as_ref().unwrap();
    let at = position(source, source.rfind("before(1)").unwrap());
    assert!(
        document.rename_plan(at, &[], Some("alpha")).is_err(),
        "no original expansion does not certify behavior under the new spelling"
    );

    let proposed = source.replace("before", "alpha");
    let transaction = parser
        .host
        .begin_parse("file:///workspace", "conditional-macro-proposal", 2)
        .unwrap();
    let expanded = parser
        .host
        .expand_text_in_parse(
            &transaction,
            parser_wasm::host::TextMacroRequest {
                context: parser_wasm::host::InvocationContext {
                    invocation_id: 2,
                    subscription_id: String::new(),
                    document_id: "conditional-macro-proposal".into(),
                    document_revision: 2,
                    expansion: None,
                    syntax_context: 0,
                },
                source: skript_parser::MappedSource::identity(proposed.as_str()),
            },
        )
        .unwrap();
    assert!(expanded.calls.iter().any(|call| call.expansion.is_some()));
    assert_ne!(expanded.source.virtual_source(), proposed.as_str());
    transaction.cancel().unwrap();
}

#[test]
fn rename_revokes_an_already_published_document_when_provider_set_changes() {
    let source = "function before() :: number:\n    return 1\non join:\n    send before()\n";
    let mut parser = analyzer();
    let analyzed = analyze(&mut parser, source, 1);
    let document = analyzed.semantic.as_ref().unwrap();
    let at = position(source, source.rfind("before()").unwrap());
    assert!(
        document
            .rename_plan(at, &[], Some("renamed"))
            .unwrap()
            .is_some()
    );
    assert!(parser.host.load_addon(&[]).is_err());
    assert!(document.rename_plan(at, &[], Some("renamed")).is_err());
}

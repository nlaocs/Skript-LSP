use super::*;
use crate::lsp::completion::{PreparedDocument, PreparedQuery, PreparedSection};
use parser_wasm::{
    DocumentParseRequest, DocumentParserConfig, RuntimeProfile,
    host::{HostConfig, ParserHost},
};
use skript_parser::{StructureHeaderPreparation, StructurePreparationEvent};
use std::{path::Path, sync::OnceLock};

const SOURCE: &str = "# 日本🦊\nfunction first(value: number, values: numbers, arg%%Name: text):\n    send {_} to console\n    if {_value} is 1:\n        send {_} to console\n    send {_arg%%%%Name} to console\nfunction second(other: text):\n    send {_}\non load:\n    send \"end\"\n";

fn catalog() -> Arc<Catalog> {
    static CATALOG: OnceLock<Arc<Catalog>> = OnceLock::new();
    Arc::clone(CATALOG.get_or_init(|| {
        Arc::new(
            ssg::load(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("parser-wasm/tests/data/type-parser-versions/skript-2.15.4-type-runtime"),
            )
            .unwrap()
            .into_catalog(),
        )
    }))
}
fn position(source: &str, needle: &str, prefix: usize) -> Position {
    LineIndex::new(source)
        .position(source, source.find(needle).unwrap() + prefix)
        .unwrap()
}
fn variables(list: &CompletionList) -> Vec<&CompletionItem> {
    list.items
        .iter()
        .filter(|i| i.kind == Some(CompletionItemKind::VARIABLE))
        .collect()
}
#[track_caller]
fn ready(view: &PreparedDocument, position: Position) -> CompletionList {
    match view.at(position, false).unwrap() {
        PreparedQuery::Ready(list) => list,
        PreparedQuery::NeedsAnalysis => panic!("expected certified parameter proposals"),
    }
}
fn parse(
    source: &str,
) -> (
    PreparedDocument,
    Document,
    DocumentParseResult,
    Vec<CompletionList>,
) {
    let catalog = catalog();
    let index = Arc::new(CatalogIndex::with_documentation_format(
        &catalog,
        skript_parser::RawTreeOptions::for_skript_version(2, 15),
        lsp_types::MarkupKind::PlainText,
    ));
    let mut host = ParserHost::new(
        crate::core_library_component(),
        HostConfig {
            syntax_catalog: Some(catalog),
            runtime_profile: RuntimeProfile {
                skript_version: Some("2.15.4".into()),
                ..RuntimeProfile::default()
            },
            ..HostConfig::default()
        },
    )
    .unwrap();
    let mut prepared: Option<PreparedDocument> = None;
    let mut initial = Vec::new();
    let mut observer = |event: StructurePreparationEvent<'_>| match event {
        StructurePreparationEvent::Headers {
            source: mapped,
            tree,
            headers,
        } => {
            let view = PreparedDocument::new(
                Arc::from(source),
                Arc::clone(&index),
                mapped,
                tree,
                &headers,
            )
            .unwrap();
            for position in [Position::new(2, 11), Position::new(7, 11)] {
                initial.push(match view.at(position, false).unwrap() {
                    PreparedQuery::Ready(list) => list,
                    PreparedQuery::NeedsAnalysis => CompletionList {
                        is_incomplete: true,
                        items: Vec::new(),
                    },
                });
            }
            prepared = Some(view);
        }
        StructurePreparationEvent::StructureHeader {
            source: mapped,
            tree,
            header,
        } => {
            let root = header.candidate.raw_node_id;
            let delta = PreparedDocument::new(
                Arc::from(source),
                Arc::clone(&index),
                mapped,
                tree,
                &[header],
            )
            .unwrap();
            prepared.as_mut().unwrap().prepare_structure(root, delta);
        }
        StructurePreparationEvent::SectionHeader {
            source: mapped,
            tree,
            candidate,
            attempt_id,
            parent_attempt_id,
        } => {
            if let Some(section) = PreparedSection::new(
                source,
                mapped,
                tree,
                candidate,
                attempt_id,
                parent_attempt_id,
            )
            .unwrap()
            {
                prepared.as_mut().unwrap().prepare_section(section);
            }
        }
        StructurePreparationEvent::Invalidated {
            raw_node_id,
            registration_id,
            pattern_index,
            attempt_id,
        } => {
            prepared
                .as_mut()
                .unwrap()
                .revoke(&crate::lsp::completion::OwnerAttempt {
                    root: raw_node_id,
                    registration: registration_id.into(),
                    pattern: pattern_index,
                    attempt_id,
                });
        }
    };
    let result = host
        .parse_document_with_preparation(
            DocumentParseRequest::new(
                "file:///workspace",
                "file:///workspace/parameters.sk",
                1,
                source,
            ),
            DocumentParserConfig::default(),
            &mut observer,
        )
        .unwrap();
    let final_view = Document::from_result(&result, Arc::from(source), index).unwrap();
    (prepared.unwrap(), final_view, result, initial)
}
fn assert_first(list: &CompletionList) {
    let params = variables(list);
    for label in ["{_value}", "{_values::*}", "{_arg%%%%Name}"] {
        assert!(
            params.iter().any(|item| item.label == label),
            "missing {label}: {params:?}"
        );
    }
    assert!(!params.iter().any(|item| item.label == "{_other}"));
    let value = params.iter().find(|item| item.label == "{_value}").unwrap();
    assert_eq!(
        value.data.as_ref().unwrap()["declaredType"],
        serde_json::json!({"className":"java.lang.Number","single":true})
    );
    let plural = params
        .iter()
        .find(|item| item.label == "{_values::*}")
        .unwrap();
    assert_eq!(
        plural.data.as_ref().unwrap()["declaredType"]["single"],
        false
    );
    assert_eq!(
        plural.data.as_ref().unwrap()["parameterDeclaration"]["list"],
        true
    );
}

#[test]
fn declaration_parameters_are_available_before_first_use_in_early_and_final_scopes() {
    let (prepared, mut final_view, result, initial) = parse(SOURCE);
    assert_first(&initial[0]);
    assert_eq!(
        variables(&initial[1])
            .iter()
            .map(|i| i.label.as_str())
            .collect::<Vec<_>>(),
        ["{_other}"]
    );
    let at = position(SOURCE, "    send {_}", 11);
    assert_first(&ready(&prepared, at));
    assert_first(&final_view.at(at, false).unwrap());
    final_view.function_namespace_complete = false;
    assert_first(&final_view.at(at, false).unwrap());
    let nested = position(SOURCE, "        send {_}", 15);
    assert_first(&ready(&prepared, nested));
    assert_first(&final_view.at(nested, false).unwrap());
    let outside = position(SOURCE, "    send \"end\"", 9);
    assert!(variables(&ready(&prepared, outside)).is_empty());
    assert!(variables(&final_view.at(outside, false).unwrap()).is_empty());
    // Escaped insertion parses back to exactly the initialized parameter name.
    assert!(
        result
            .ast
            .nodes
            .iter()
            .any(|node| runtime_identity(node) == Some(("local".into(), "arg%%Name".into())))
    );
    let bracketed = Position::new(2, 11);
    assert!(
        ready(&prepared, bracketed)
            .items
            .iter()
            .any(|i| i.label == "{_value}")
    );
}

#[test]
fn parameter_retry_replaces_types_and_revocation_does_not_leak_other_functions() {
    let (mut prepared, final_view, result, _) = parse(SOURCE);
    let at = position(SOURCE, "    send {_}", 11);
    let old = prepared.owner_at(at).unwrap().clone();
    let mut candidate = result
        .syntax
        .roots
        .iter()
        .find_map(|node| {
            let skript_parser::StructureDocumentNode::Structure(matches) = node else {
                return None;
            };
            matches
                .selected
                .as_ref()
                .filter(|c| c.raw_node_id == old.root)
                .cloned()
        })
        .unwrap();
    for (key, value) in &mut candidate.metadata {
        if key.ends_with("/skript.semantic.parameters") {
            *value = value.replace("java.lang.Number", "java.lang.Object");
        }
    }
    let delta = PreparedDocument::new(
        Arc::from(SOURCE),
        Arc::clone(&final_view.index),
        &result.source,
        &result.raw_tree,
        &[StructureHeaderPreparation {
            attempt_id: old.attempt_id + 100,
            candidate: &candidate,
            body_mode: skript_parser::StructureBodyMode::Trigger,
            ambiguous: false,
        }],
    )
    .unwrap();
    assert!(prepared.prepare_structure(old.root, delta));
    let list = ready(&prepared, at);
    assert_eq!(
        list.items
            .iter()
            .find(|i| i.label == "{_value}")
            .unwrap()
            .data
            .as_ref()
            .unwrap()["declaredType"]["className"],
        "java.lang.Object"
    );
    assert!(!prepared.revoke(&old));
    let current = prepared.owner_at(at).unwrap().clone();
    assert!(prepared.revoke(&current));
    assert!(matches!(
        prepared.at(at, false).unwrap(),
        PreparedQuery::NeedsAnalysis
    ));
    let other = Position::new(7, 11);
    assert_eq!(
        variables(&ready(&prepared, other))
            .iter()
            .map(|i| i.label.as_str())
            .collect::<Vec<_>>(),
        ["{_other}"]
    );
}

#[test]
fn modified_final_parameter_metadata_requires_current_native_witness() {
    let (_, final_view, mut result, _) = parse(SOURCE);
    for node in &mut result.ast.nodes {
        for metadata in &mut node.metadata {
            if metadata.key == "skript.semantic.parameters" {
                metadata.value = metadata
                    .value
                    .replace("java.lang.Number", "java.lang.Object");
            }
        }
    }
    let forged = Document::from_result(&result, Arc::from(SOURCE), final_view.index).unwrap();
    assert!(
        variables(
            &forged
                .at(position(SOURCE, "    send {_}", 11), false)
                .unwrap()
        )
        .is_empty()
    );
}

#[test]
fn parameter_contract_projection_rejects_bad_geometry_and_preserves_unknown() {
    let source = "# 日本\nfunction value: number:";
    let start = source.find("function").unwrap();
    let header = TextRange::new(start, source.len());
    let record = serde_json::json!({"version":2,"parameters":[{
        "identity":{"name":"value","scope":"local","resolution":"static","aliasKey":"value","list":false},
        "className":"java.lang.Object","single":true,"start":9,"end":14
    }]});
    let project_value = |value: &serde_json::Value| {
        let json = value.to_string();
        project(
            crate::lsp::public_data::parameter_bindings(std::iter::once((
                "skript.semantic.parameters",
                json.as_str(),
            )))?,
            source,
            header,
            start as u64,
        )
    };
    assert_eq!(
        project_value(&record).unwrap()[0].class_name,
        "java.lang.Object"
    );
    for (field, value) in [
        ("start", serde_json::json!(999)),
        ("end", serde_json::json!(999)),
    ] {
        let mut bad = record.clone();
        bad["parameters"][0][field] = value;
        assert!(project_value(&bad).is_none());
    }
    let mut old = record.clone();
    old["version"] = serde_json::json!(1);
    old["parameters"][0]["start"] = serde_json::json!(start + 9);
    old["parameters"][0]["end"] = serde_json::json!(start + 14);
    assert_eq!(
        project_value(&old).unwrap()[0].range,
        TextRange::new(start + 9, start + 14)
    );
    let mut duplicate = record.clone();
    duplicate["parameters"]
        .as_array_mut()
        .unwrap()
        .push(record["parameters"][0].clone());
    assert!(project_value(&duplicate).is_none());
    let mut foreign = record.clone();
    foreign["parameters"][0]["identity"]["scope"] = serde_json::json!("global");
    assert!(project_value(&foreign).is_none());
    let mut unknown = record.clone();
    unknown["version"] = serde_json::json!(999);
    assert!(project_value(&unknown).is_none());
    let mid = "é";
    let data = record.to_string();
    let mut bindings = crate::lsp::public_data::parameter_bindings(std::iter::once((
        "skript.semantic.parameters",
        data.as_str(),
    )))
    .unwrap();
    bindings.parameters[0].start = 0;
    bindings.parameters[0].end = 1;
    assert!(project(bindings, mid, TextRange::new(0, mid.len()), 0).is_none());
}

#[test]
fn a_different_owned_metadata_key_cannot_declare_parameters() {
    let (mut prepared, final_view, result, _) = parse(SOURCE);
    let at = position(SOURCE, "    send {_}", 11);
    let old = prepared.owner_at(at).unwrap().clone();
    let mut candidate = result
        .syntax
        .roots
        .iter()
        .find_map(|node| {
            let skript_parser::StructureDocumentNode::Structure(matches) = node else {
                return None;
            };
            matches
                .selected
                .as_ref()
                .filter(|c| c.raw_node_id == old.root)
                .cloned()
        })
        .unwrap();
    let key = candidate
        .metadata
        .keys()
        .find(|key| key.ends_with("/skript.semantic.parameters"))
        .unwrap()
        .clone();
    let data = candidate.metadata.remove(&key).unwrap();
    candidate
        .metadata
        .insert("addon/other/skript.semantic.parameters".into(), data);
    let delta = PreparedDocument::new(
        Arc::from(SOURCE),
        Arc::clone(&final_view.index),
        &result.source,
        &result.raw_tree,
        &[StructureHeaderPreparation {
            attempt_id: old.attempt_id + 1,
            candidate: &candidate,
            body_mode: skript_parser::StructureBodyMode::Trigger,
            ambiguous: false,
        }],
    )
    .unwrap();
    assert!(prepared.prepare_structure(old.root, delta));
    if let PreparedQuery::Ready(list) = prepared.at(at, false).unwrap() {
        assert!(variables(&list).is_empty());
    }
}

#[test]
fn escaped_parameter_edits_replace_the_complete_variable_from_middle_cursor() {
    let (prepared, final_view, result, _) = parse(SOURCE);
    let at = position(SOURCE, "{_arg%%%%Name}", 5);
    for list in [ready(&prepared, at), final_view.at(at, false).unwrap()] {
        let item = list
            .items
            .iter()
            .find(|item| item.label == "{_arg%%%%Name}")
            .unwrap();
        assert_eq!(
            item.data.as_ref().unwrap()["parameterDeclaration"]["name"],
            "arg%%Name"
        );
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("plain edit expected")
        };
        let lines = LineIndex::new(SOURCE);
        let start = lines.offset(SOURCE, edit.range.start).unwrap();
        let end = lines.offset(SOURCE, edit.range.end).unwrap();
        let mut accepted = SOURCE.to_owned();
        accepted.replace_range(start..end, &edit.new_text);
        assert_eq!(
            accepted, SOURCE,
            "suffix or closing brace must not be duplicated"
        );
    }
    assert!(
        result
            .ast
            .nodes
            .iter()
            .any(|node| runtime_identity(node) == Some(("local".into(), "arg%%Name".into())))
    );
    assert_eq!(
        cursor::variable_edit_end("{_arg%%%%Name} tail", 5),
        Ok(Some("{_arg%%%%Name}".len()))
    );
    assert_eq!(
        cursor::variable_edit_end("{_arg%player%}", 5),
        Ok(Some("{_arg%player%}".len()))
    );
    assert_eq!(
        cursor::variable_edit_end("{_arg{nested}}", 5),
        Ok(Some("{_arg{nested}}".len()))
    );
    assert_eq!(cursor::variable_edit_end("{_arg%player", 5), Err(()));
    assert_eq!(cursor::variable_edit_end("{_outer{in", 10), Ok(None));
}

#[test]
fn partial_body_reference_keeps_current_header_parameters() {
    let source = SOURCE.replacen("send {_} to console", "send {_v", 1);
    let (prepared, final_view, _, initial) = parse(&source);
    // The fixed capture position is before the existing right suffix "v".
    // Preserve that suffix policy; the actual partial-input request is at EOL.
    assert!(initial[0].items.is_empty());
    let at = position(&source, "{_v\n", 3);
    assert!(
        ready(&prepared, at)
            .items
            .iter()
            .any(|item| item.label == "{_value}")
    );
    assert!(
        final_view
            .at(at, false)
            .unwrap()
            .items
            .iter()
            .any(|item| item.label == "{_value}")
    );
}

fn runtime_identity(node: &AstNode) -> Option<(String, String)> {
    let record = node.summary.as_ref()?.public_data.iter().find(|data| {
        data.schema_id == "nlaocs.skript.variable-identity" && data.schema_version == 1
    })?;
    let value: serde_json::Value = serde_json::from_str(&record.json).ok()?;
    Some((
        value["scope"].as_str()?.into(),
        value["name"].as_str()?.into(),
    ))
}

#[test]
fn complex_variable_suffixes_never_receive_partial_parameter_edits() {
    for variable in ["{_arg%player%}", "{_arg{nested}}"] {
        let source = SOURCE.replacen("{_arg%%%%Name}", variable, 1);
        let (prepared, final_view, _, _) = parse(&source);
        let at = position(&source, variable, 5);
        let check = |list: &CompletionList| {
            for item in variables(list) {
                assert!(
                    item.data
                        .as_ref()
                        .is_none_or(|data| data.get("parameterDeclaration").is_none()),
                    "a different static parameter must not replace a complex suffix: {item:?}"
                );
                // Existing history can preserve this same complete name. Verify
                // its actual edit rather than prohibiting a valid old candidate.
                let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                    panic!("plain edit expected")
                };
                let lines = LineIndex::new(&source);
                let start = lines.offset(&source, edit.range.start).unwrap();
                let end = lines.offset(&source, edit.range.end).unwrap();
                let mut accepted = source.clone();
                accepted.replace_range(start..end, &edit.new_text);
                assert_eq!(accepted, source, "a right suffix must not be duplicated");
            }
        };
        if let PreparedQuery::Ready(list) = prepared.at(at, false).unwrap() {
            check(&list);
        }
        check(&final_view.at(at, false).unwrap());
    }
}

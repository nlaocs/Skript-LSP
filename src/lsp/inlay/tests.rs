use super::*;
use lsp_types::Position;
use parser_wasm::{
    DocumentParseRequest, DocumentParserConfig, RuntimeProfile,
    host::{HostConfig, ParserHost},
};
use std::path::Path;

use crate::lsp::test_support::runtime_catalog as catalog;
fn parse(source: &str) -> DocumentParseResult {
    parse_with_catalog(source, catalog())
}
fn parse_with_catalog(source: &str, catalog: Arc<Catalog>) -> DocumentParseResult {
    let mut host = ParserHost::new(
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/artifacts/core-library.wasm"
        )),
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
    host.parse_document(
        DocumentParseRequest::new("file:///workspace", "file:///workspace/inlay.sk", 1, source),
        DocumentParserConfig::default(),
    )
    .unwrap()
}
fn all(document: &Document) -> Vec<InlayHint> {
    document.at(Range::new(
        Position::new(0, 0),
        document
            .lines
            .position(&document.source, document.source.len())
            .unwrap(),
    ))
}
fn label(hint: &InlayHint) -> &str {
    match &hint.label {
        InlayHintLabel::String(text) => text,
        _ => panic!("expected text hint"),
    }
}

#[test]
fn actual_join_sender_default_has_connective_context_and_no_text_edit() {
    let source = "on join:\n    send 1\n";
    let result = parse(source);
    assert!(
        result.component_failures.is_empty(),
        "{:?}",
        result.component_failures
    );
    let default = result
        .ast
        .nodes
        .iter()
        .find(|node| {
            node.summary
                .as_ref()
                .is_some_and(|summary| summary.default_expression.is_some())
        })
        .expect("the genuine snapshot must initialize the omitted recipient");
    let summary = default.summary.as_ref().unwrap();
    let info = summary.default_expression.as_ref().unwrap();
    assert!(
        summary
            .public_data
            .iter()
            .any(|data| data.schema_id == DEFAULT_SOURCE_SCHEMA && data.schema_version == 1)
    );
    assert!(matches!(
        info.omission.as_ref().map(|value| value.kind),
        Some(PatternOmissionKind::SkippedOption)
    ));
    assert_eq!(anchor(&default.span), source.rfind('\n'));
    assert!(
        result
            .semantic_witnesses()
            .provider_valid(default, &info.component_id)
    );
    let document = Document::from_result(&result, Arc::from(source), catalog()).unwrap();
    let hints = all(&document);
    assert_eq!(hints.len(), 1);
    assert_eq!(label(&hints[0]), "to <event player>");
    assert_eq!(hints[0].position, Position::new(1, 10));
    assert!(hints[0].text_edits.is_none());
    let Some(InlayHintTooltip::MarkupContent(tooltip)) = &hints[0].tooltip else {
        panic!()
    };
    assert!(tooltip.value.contains("org.bukkit.command.CommandSender"));
    assert!(tooltip.value.contains("org.bukkit.entity.Player"));
    assert!(tooltip.value.contains("core.default-expression.skript"));
    assert!(
        document
            .at(Range::new(Position::new(0, 0), Position::new(0, 8)))
            .is_empty()
    );
}

#[test]
fn initialized_literal_default_identifies_type_without_inventing_stored_value() {
    let source = "on join:\n    knockback player\n";
    let result = parse(source);
    let document = Document::from_result(&result, Arc::from(source), catalog()).unwrap();
    let hints = all(&document);
    let hint = hints
        .iter()
        .find(|hint| label(hint).contains("<registered default: direction>"))
        .expect("actual omitted direction has a registered SimpleLiteral default");
    assert!(hint.text_edits.is_none());
    let Some(InlayHintTooltip::MarkupContent(tooltip)) = &hint.tooltip else {
        panic!()
    };
    assert!(
        tooltip
            .value
            .contains("stored literal value is unavailable")
    );
    assert!(tooltip.value.contains("does not assert a value"));
}

#[test]
fn explicit_receiver_does_not_gain_a_guessed_hint() {
    let source = "on join:\n    send 1 to player\n";
    let result = parse(source);
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
    assert!(
        result.ast.nodes.iter().any(|node| node
            .summary
            .as_ref()
            .is_some_and(|summary| summary.kind == "effect")),
        "the explicit receiver must belong to an actually parsed effect"
    );
    assert!(result.ast.nodes.iter().all(|node| {
        node.summary
            .as_ref()
            .is_none_or(|summary| summary.default_expression.is_none())
    }));
    let document = Document::from_result(&result, Arc::from(source), catalog()).unwrap();
    assert!(
        all(&document).is_empty(),
        "an explicit recipient is not omitted"
    );
}

#[test]
fn actual_script_load_sender_default_is_proven_without_inventing_a_console_literal() {
    const EVENT: &str = "ch.njol.skript.events.bukkit.ScriptEvent";
    const SENDER: &str = "org.bukkit.command.CommandSender";
    const REGISTRATION: &str =
        "event-value:skript:c22bfd7ae68d34bf5c6f5c3a33c0f28a19553ef3e56063389421e2ec99913a60";
    let source = "on load:\n    send 1\n";
    let result = parse(source);
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
    let catalog = catalog();
    let event = result
        .ast
        .nodes
        .iter()
        .find(|node| {
            node.summary
                .as_ref()
                .is_some_and(|summary| summary.kind == "event")
        })
        .expect("the Script Load header must actually parse");
    assert!(result.semantic_witnesses().native_valid(event));
    let records = catalog
        .source()
        .unwrap()
        .records_by_registration_id(&event.syntax_id);
    assert!(records.iter().any(|record| {
        let json: serde_json::Value = serde_json::from_slice(&record.json).unwrap();
        json["elementClass"] == "ch.njol.skript.events.EvtScript"
            && json["referenceEvents"] == serde_json::json!([EVENT])
    }));
    let defaults = result
        .ast
        .nodes
        .iter()
        .filter(|node| {
            node.summary
                .as_ref()
                .is_some_and(|summary| summary.default_expression.is_some())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        defaults.len(),
        1,
        "only the omitted actual recipient is initialized"
    );
    let default = defaults[0];
    let summary = default.summary.as_ref().unwrap();
    let info = summary.default_expression.as_ref().unwrap();
    assert_eq!(info.capture_index, 1);
    assert_eq!(info.event_classes, [EVENT]);
    assert_eq!(info.provider_id, "core.default-expression.skript");
    assert_eq!(summary.return_type.as_deref(), Some(SENDER));
    assert!(!info.is_literal);
    assert!(matches!(
        info.omission.as_ref().map(|omission| omission.kind),
        Some(PatternOmissionKind::SkippedOption)
    ));
    assert!(
        result
            .semantic_witnesses()
            .provider_valid(default, &info.component_id)
    );
    assert_eq!(anchor(&default.span), source.rfind('\n'));
    let data = summary
        .public_data
        .iter()
        .find(|data| data.schema_id == DEFAULT_SOURCE_SCHEMA && data.schema_version == 1)
        .expect("initialized source evidence is required");
    let data: DefaultSource = serde_json::from_str(&data.json).unwrap();
    assert_eq!(data.source_kind, "event-value");
    assert_eq!(
        data.implementation_class,
        "ch.njol.skript.expressions.base.EventValueExpression"
    );
    assert_eq!(data.target_class, SENDER);
    assert_eq!(data.time, 0);
    assert_eq!(data.event_values.len(), 1);
    let value = &data.event_values[0];
    assert_eq!(value.event_class, EVENT);
    assert_eq!(value.value_class, SENDER);
    assert_eq!(value.time, 0);
    assert_eq!(value.registration_id, REGISTRATION);
    let reference = info
        .catalog_references
        .iter()
        .find(|reference| {
            reference.role == "event-value"
                && reference.registration_id.as_deref() == Some(REGISTRATION)
        })
        .unwrap();
    let provenance = catalog.source().unwrap();
    assert_eq!(
        reference.source_digest.as_deref(),
        Some(provenance.source_digest.as_str())
    );
    assert_eq!(
        reference.snapshot_id.as_deref(),
        Some(provenance.snapshot_id.as_str())
    );
    assert_eq!(reference.document.as_deref(), Some("EventValues.json"));
    let referenced = provenance
        .record(
            reference.document.as_deref().unwrap(),
            usize::try_from(reference.index.unwrap()).unwrap(),
        )
        .expect("the emitted native source reference must identify its actual current raw record");
    let referenced: serde_json::Value = serde_json::from_slice(&referenced.json).unwrap();
    assert_eq!(referenced["registrationId"], REGISTRATION);
    assert_eq!(referenced["eventClass"], EVENT);
    assert_eq!(referenced["valueClass"], SENDER);
    assert_eq!(referenced["time"], 0);
    let nodes = result
        .ast
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<HashMap<_, _>>();
    let effect = result
        .ast
        .nodes
        .iter()
        .find(|node| {
            node.summary
                .as_ref()
                .is_some_and(|summary| summary.kind == "effect")
        })
        .unwrap();
    assert!(result.semantic_witnesses().native_valid(effect));
    assert!(owns_capture(effect, default, info, &nodes));
    let document = Document::from_result(&result, Arc::from(source), catalog).unwrap();
    let hints = all(&document);
    assert_eq!(hints.len(), 1);
    assert_eq!(label(&hints[0]), "to <event commandsender>");
    assert_eq!(hints[0].position, Position::new(1, 10));
    assert!(hints[0].text_edits.is_none());
    let Some(InlayHintTooltip::MarkupContent(tooltip)) = &hints[0].tooltip else {
        panic!()
    };
    assert!(
        tooltip.value.contains(EVENT)
            && tooltip.value.contains(SENDER)
            && tooltip.value.contains(REGISTRATION)
    );
    // SSG identifies the initialized getter's declared event value, not its
    // runtime result; rendering does not fabricate a stored console value.
}

#[test]
fn actual_periodical_event_without_sender_does_not_gain_a_guessed_hint() {
    const EVENT: &str = "ch.njol.skript.events.bukkit.ScheduledNoWorldEvent";
    const SENDER: &str = "org.bukkit.command.CommandSender";
    let source = "every 1 tick:\n    send 1\n";
    let catalog = catalog();
    assert!(
        catalog
            .event_values_for(EVENT)
            .iter()
            .all(|value| !catalog.can_convert(value.value_class.as_str(), SENDER)),
        "the authentic event has no assignable or convertible sender value"
    );
    let result = parse(source);
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
    let event = result
        .ast
        .nodes
        .iter()
        .find(|node| {
            node.summary
                .as_ref()
                .is_some_and(|summary| summary.kind == "event")
        })
        .expect("the genuine Periodical header must actually parse");
    assert!(result.semantic_witnesses().native_valid(event));
    assert!(
        catalog
            .source()
            .unwrap()
            .records_by_registration_id(&event.syntax_id)
            .iter()
            .any(|record| {
                let json: serde_json::Value = serde_json::from_slice(&record.json).unwrap();
                json["elementClass"] == "ch.njol.skript.events.EvtPeriodical"
                    && json["referenceEvents"] == serde_json::json!([EVENT])
            })
    );
    assert!(
        result.ast.nodes.iter().all(|node| node
            .summary
            .as_ref()
            .is_none_or(|summary| summary.default_expression.is_none())),
        "no resolved recipient default may be fabricated"
    );
    let uri: lsp_types::Uri = "file:///workspace/inlay.sk".parse().unwrap();
    let diagnostics = super::super::diagnostics::from_result(&uri, &result).unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.range.start.line == 1
                && diagnostic.message.contains(SENDER)),
        "the actual omitted recipient must fail with its required sender type: {diagnostics:#?}"
    );
    let document = Document::from_result(&result, Arc::from(source), catalog).unwrap();
    assert!(
        all(&document).is_empty(),
        "unavailable sender values cannot produce an omission hint"
    );
}

#[test]
fn current_source_and_sealed_geometry_are_required_even_with_a_default_record() {
    let source = "on join:\n    send 1\n";
    let mut result = parse(source);
    assert!(
        Document::from_result(&result, Arc::from("on join:\n    send 2\n"), catalog()).is_err()
    );
    let effect = result
        .ast
        .nodes
        .iter_mut()
        .find(|node| {
            node.summary
                .as_ref()
                .is_some_and(|summary| summary.kind == "effect")
        })
        .unwrap();
    assert!(source_geometry::pattern_capture_geometry_is_current(effect));
    effect.text.push(' ');
    let document = Document::from_result(&result, Arc::from(source), catalog()).unwrap();
    assert!(
        all(&document).is_empty(),
        "an AST macro cannot retain an obsolete native seal"
    );
}

#[test]
fn semantic_rewrite_cannot_reuse_source_records_with_an_unchanged_parent_seal() {
    let source = "on join:\n    send 1\n";
    let mut result = parse(source);
    let summary = result
        .ast
        .nodes
        .iter_mut()
        .filter_map(|node| node.summary.as_mut())
        .find(|summary| summary.default_expression.is_some())
        .unwrap();
    let record = summary
        .public_data
        .iter_mut()
        .find(|data| data.schema_id == DEFAULT_SOURCE_SCHEMA)
        .unwrap();
    let mut data: serde_json::Value = serde_json::from_str(&record.json).unwrap();
    data["eventValues"][0]["valueClass"] = serde_json::json!("java.lang.String");
    record.json = serde_json::to_string(&data).unwrap();
    let parent = result
        .ast
        .nodes
        .iter()
        .find(|node| {
            node.summary
                .as_ref()
                .is_some_and(|summary| summary.kind == "effect")
        })
        .unwrap();
    assert!(source_geometry::pattern_capture_geometry_is_current(parent));
    assert!(!result.semantic_witnesses().native_valid(parent));
    let document = Document::from_result(&result, Arc::from(source), catalog()).unwrap();
    assert!(
        all(&document).is_empty(),
        "retained source geometry alone cannot authenticate rewritten default semantics"
    );
}

#[test]
fn default_records_without_actual_skipped_option_or_source_evidence_are_not_hints() {
    let source = "on join:\n    send 1\n";
    for remove_source in [false, true] {
        let mut result = parse(source);
        let summary = result
            .ast
            .nodes
            .iter_mut()
            .filter_map(|node| node.summary.as_mut())
            .find(|summary| summary.default_expression.is_some())
            .unwrap();
        if remove_source {
            summary.public_data.clear();
        } else {
            summary
                .default_expression
                .as_mut()
                .unwrap()
                .omission
                .as_mut()
                .unwrap()
                .kind = PatternOmissionKind::UnselectedChoice;
        }
        let document = Document::from_result(&result, Arc::from(source), catalog()).unwrap();
        assert!(all(&document).is_empty());
    }
}

#[test]
fn actual_default_uses_unique_native_slot_binding_and_exact_source_span() {
    let result = parse("on join:\n    send 1\n");
    let nodes = result
        .ast
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<HashMap<_, _>>();
    let default = result
        .ast
        .nodes
        .iter()
        .find(|node| {
            node.summary
                .as_ref()
                .is_some_and(|summary| summary.default_expression.is_some())
        })
        .unwrap();
    let info = default
        .summary
        .as_ref()
        .unwrap()
        .default_expression
        .as_ref()
        .unwrap();
    let parent = result
        .ast
        .nodes
        .iter()
        .find(|node| node.children.contains(&default.id))
        .unwrap();
    assert!(owns_capture(parent, default, info, &nodes));
    let prefix = format!("{}:", info.capture_index);
    let binding = parent
        .captures
        .iter()
        .find(|capture| capture.name.starts_with(&prefix))
        .expect("actual {index}:{binding_parser_id} child binding");
    assert!(matches!(binding.value, CaptureValue::Node(id) if id == default.id));

    let mut wrong_name = parent.clone();
    wrong_name
        .captures
        .iter_mut()
        .find(|capture| capture.name == binding.name)
        .unwrap()
        .name = format!("expression:{}", info.capture_index);
    assert!(source_geometry::pattern_capture_geometry_is_current(
        &wrong_name
    ));
    assert!(!owns_capture(&wrong_name, default, info, &nodes));

    let mut duplicate = parent.clone();
    duplicate.captures.push(binding.clone());
    assert!(source_geometry::pattern_capture_geometry_is_current(
        &duplicate
    ));
    assert!(!owns_capture(&duplicate, default, info, &nodes));

    let mut detached = parent.clone();
    detached.children.retain(|id| *id != default.id);
    assert!(!owns_capture(&detached, default, info, &nodes));

    let mut wrong_child = parent.clone();
    wrong_child
        .captures
        .iter_mut()
        .find(|capture| capture.name == binding.name)
        .unwrap()
        .value = CaptureValue::Node(parent.id);
    assert!(!owns_capture(&wrong_child, default, info, &nodes));

    let mut wrong_info = info.clone();
    wrong_info.span.origins[0].original_range.start += 1;
    wrong_info.span.origins[0].original_range.end += 1;
    assert!(!owns_capture(parent, default, &wrong_info, &nodes));
}

#[test]
fn old_snapshot_default_class_without_shape_does_not_gain_guessed_hints() {
    let old = Arc::new(
        ssg::load(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("parser-wasm/tests/data/type-parser-versions/skript-2.15.4"),
        )
        .unwrap()
        .into_catalog(),
    );
    let descriptor = old
        .type_by_code_name("audience")
        .unwrap()
        .default_expression
        .as_ref()
        .unwrap();
    assert!(descriptor.return_type.is_none());
    assert!(descriptor.single.is_none());
    let source = "on join:\n    send 1\n";
    let result = parse_with_catalog(source, Arc::clone(&old));
    assert!(result.component_failures.is_empty());
    assert!(!result.ast.nodes.iter().any(|node| {
        node.summary
            .as_ref()
            .is_some_and(|summary| summary.default_expression.is_some())
    }));
    let document = Document::from_result(&result, Arc::from(source), old).unwrap();
    assert!(all(&document).is_empty());
}

#[test]
fn only_unambiguous_exact_or_anchored_original_points_are_hint_anchors() {
    use parser_wasm::bindings::nlaocs::skript_parser_addon::types::{
        OriginKind, SourceOrigin, TextRange,
    };
    let exact = MappedSpan {
        virtual_range: TextRange { start: 9, end: 9 },
        origins: vec![SourceOrigin {
            original_range: TextRange { start: 25, end: 25 },
            kind: OriginKind::Exact,
            expansion: None,
        }],
    };
    assert_eq!(anchor(&exact), Some(25));
    let mut anchored = exact.clone();
    anchored.origins[0].kind = OriginKind::Anchored;
    anchored.origins[0].expansion = Some(7);
    assert_eq!(anchor(&anchored), Some(25));
    let mut replaced = exact.clone();
    replaced.origins[0].kind = OriginKind::Replaced;
    assert_eq!(anchor(&replaced), None);
    let mut nonempty = exact.clone();
    nonempty.virtual_range.end += 1;
    assert_eq!(anchor(&nonempty), None);
    let mut nonempty_origin = exact.clone();
    nonempty_origin.origins[0].original_range.end += 1;
    assert_eq!(anchor(&nonempty_origin), None);
    let mut ambiguous = exact.clone();
    anchored.origins[0].original_range = TextRange { start: 26, end: 26 };
    ambiguous.origins.push(anchored.origins[0].clone());
    assert_eq!(anchor(&ambiguous), None);
    let mut mixed_replaced = exact.clone();
    mixed_replaced.origins.push(replaced.origins[0].clone());
    assert_eq!(anchor(&mixed_replaced), None);
    let mut missing = exact;
    missing.origins.clear();
    assert_eq!(anchor(&missing), None);
}

#[test]
fn tooltip_fences_keep_provider_reason_and_pattern_injection_literal() {
    let details = "Initialized by: malicious` provider\n\n```\n# reason heading\n[link](https://example.invalid) <img src=x>\n```";
    let pattern = "send %objects% [to %audiences%]\n````\n# pattern heading\n```";
    let rendered = render_tooltip(details, pattern);
    let mut open: Option<(usize, String)> = None;
    let mut blocks = Vec::new();
    let mut outside = Vec::new();
    for line in rendered.lines() {
        let run = line.bytes().take_while(|byte| *byte == b'`').count();
        if let Some((width, content)) = &mut open {
            if run >= *width && line[run..].trim().is_empty() {
                blocks.push(content.trim_end_matches('\n').to_owned());
                open = None;
            } else {
                content.push_str(line);
                content.push('\n');
            }
        } else if run >= 3 {
            open = Some((run, String::new()));
        } else if !line.trim().is_empty() {
            outside.push(line);
        }
    }
    assert!(open.is_none());
    assert_eq!(blocks, vec![details, pattern]);
    assert_eq!(outside, vec!["Registered pattern:"]);
    assert!(rendered.starts_with("````text\n"));
    assert!(rendered.contains("`````skript-pattern\n"));
}

#[test]
fn display_does_not_choose_nested_optional_or_choice_words() {
    let rules = syntax_pattern_parser::syntax::PluralRules::from_json(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/syntax-pattern-parser/tests/data/PluralRules-2.15.4.json"
    )))
    .unwrap();
    let parsed = syntax_pattern_parser::syntax::parse("do [to %audiences%]", &rules).unwrap();
    let option = parsed
        .elements
        .iter()
        .find_map(|element| match &element.value {
            PatternElement::Option(children) => Some(children),
            _ => None,
        })
        .unwrap();
    let slot = option
        .iter()
        .find(|element| matches!(element.value, PatternElement::TypeExpr(_)))
        .unwrap()
        .span;
    assert_eq!(
        connective(option, slot, "<event player>").as_deref(),
        Some("to <event player>")
    );
    let mut ambiguous = option.clone();
    ambiguous.push(syntax_pattern_parser::syntax::Spanned {
        value: PatternElement::Option(Vec::new()),
        span: Span::new(0, 0),
    });
    assert!(connective(&ambiguous, slot, "<event player>").is_none());
}

fn parameters(source: &str) -> Vec<(Position, String)> {
    let result = parse(source);
    assert!(
        result.component_failures.is_empty(),
        "{:?}",
        result.component_failures
    );
    let document = Document::from_result(&result, Arc::from(source), catalog()).unwrap();
    all(&document)
        .iter()
        .filter(|hint| hint.kind == Some(lsp_types::InlayHintKind::PARAMETER))
        .map(|hint| {
            assert!(hint.text_edits.is_none());
            assert_eq!(hint.padding_right, Some(true));
            (hint.position, label(hint).to_owned())
        })
        .collect()
}

#[test]
fn explicit_function_arguments_use_selected_names_and_coexist_with_defaults() {
    let source = "function announce(message: text, times: number = 1):\n    send {_message} to console\non join:\n    announce(\"hi\", 2)\n    announce(\"bye\")\n    announce(times: 3, message: \"named\")\n    send 1\n";
    let hints = parameters(source);
    assert_eq!(
        hints,
        vec![
            (Position::new(3, 13), "message:".into()),
            (Position::new(3, 19), "times:".into()),
            (Position::new(4, 13), "message:".into()),
        ]
    );
    let result = parse(source);
    let document = Document::from_result(&result, Arc::from(source), catalog()).unwrap();
    assert!(
        all(&document)
            .iter()
            .any(|hint| hint.kind.is_none() && label(hint).contains("event")),
        "the pre-existing omission hints still appear"
    );
    assert!(
        document
            .at(Range::new(Position::new(3, 19), Position::new(3, 19)))
            .iter()
            .any(|hint| label(hint) == "times:")
    );
}

#[test]
fn catalog_nested_plural_and_utf16_argument_positions_are_preserved() {
    let source = "function collect(values: numbers):\n    send {_values::*} to console\non join:\n    collect(1, 2, 3)\n    send max(1, 2)\n    send substring(\"\u{1f600}\", 1, 2)\nfunction substring(value: text, start: number, end: number) :: text:\n    return {_value}\n";
    let hints = parameters(source);
    assert!(
        hints
            .iter()
            .filter(|(position, _)| position.line == 3)
            .cloned()
            .collect::<Vec<_>>()
            == vec![
                (Position::new(3, 12), "values:".into()),
                (Position::new(3, 15), "values:".into()),
                (Position::new(3, 18), "values:".into())
            ],
        "{hints:?}"
    );
    assert!(
        hints.iter().any(|(position, _)| position.line == 4),
        "{hints:?}"
    );
    // The supplementary character before the numeric argument uses two UTF-16 units.
    assert!(
        hints
            .iter()
            .any(|(position, _)| *position == Position::new(5, 25)),
        "{hints:?}"
    );
}

#[test]
fn calls_inside_nested_arguments_keep_both_parameter_bindings() {
    let source = "function identity(value: number) :: number:\n    return {_value}\non join:\n    send identity(identity(2))\n";
    assert_eq!(
        parameters(source),
        vec![
            (Position::new(3, 18), "value:".into()),
            (Position::new(3, 27), "value:".into()),
        ]
    );
}

#[test]
fn incomplete_unknown_and_ambiguous_calls_do_not_gain_guessed_names() {
    for call in [
        "missing(1)",
        "identity(",
        "identity(2",
        "identity(2,",
        "identity(\"no number\")",
    ] {
        let source = format!(
            "function identity(value: number) :: number:\n    return {{_value}}\non join:\n    send {call}\n"
        );
        assert!(parameters(&source).is_empty(), "{source}");
    }
    let source = "function identity(first: number) :: number:\n    return {_first}\nfunction identity(second: integer) :: number:\n    return {_second}\non join:\n    send identity(2)\n";
    let result = parse(source);
    assert!(
        !result.ast.nodes.iter().any(|node| node
            .function
            .as_ref()
            .is_some_and(|call| call.name == "identity")),
        "an ambiguous call must not be selected"
    );
    assert!(parameters(source).is_empty());
}

#[test]
fn selected_overload_and_local_shadow_supply_their_own_names() {
    let overloads = "function choose(numeric: number) :: number:\n    return {_numeric}\nfunction choose(words: text) :: text:\n    return {_words}\non join:\n    send choose(\"hello\")\n";
    assert_eq!(
        parameters(overloads),
        vec![(Position::new(5, 16), "words:".into())]
    );
    let shadow = "function identity(global_value: number) :: number:\n    return {_global_value}\nlocal function identity(local_value: number) :: number:\n    return {_local_value}\non join:\n    send identity(2)\n";
    assert_eq!(
        parameters(shadow),
        vec![(Position::new(5, 18), "local_value:".into())]
    );
}

#[test]
fn changed_or_detached_call_metadata_cannot_borrow_the_native_witness() {
    let source = "on join:\n    send max(1, 2)\n";
    assert!(!parameters(source).is_empty());
    for change in 0..3 {
        let mut result = parse(source);
        let call = result
            .ast
            .nodes
            .iter_mut()
            .find(|node| node.function.is_some())
            .unwrap();
        match change {
            0 => call.function.as_mut().unwrap().arguments[0].parameter_name = "forged".into(),
            1 => call.children.clear(),
            _ => call.span.origins[0].original_range.start += 1,
        }
        let document = Document::from_result(&result, Arc::from(source), catalog()).unwrap();
        assert!(
            all(&document)
                .iter()
                .all(|hint| hint.kind != Some(lsp_types::InlayHintKind::PARAMETER))
        );
    }
}

//! ClassInfo editing proposals from registered user spellings, not syntax names.
use super::*;

impl Document {
    pub(super) fn push_type_names(
        &self,
        line: &Line,
        offset: usize,
        word_start: usize,
        word_end: usize,
        references: bool,
        pool: &mut ranking::Pool<'_>,
    ) -> Result<bool, String> {
        let Some(index) = &self.index.typing else {
            return Ok(false);
        };
        if !index.current() {
            return Ok(false);
        }
        if index.type_names.is_empty() {
            return Ok(index.type_names_limited);
        }
        let mut budget = continuation::Budget::new();
        let mut start_offsets = vec![word_start];
        // Multiword registered names may own earlier words. Never cross a
        // source boundary or scan an unbounded expression to find a type slot.
        let mut scan_start = word_start.saturating_sub(256).max(line.code_start);
        while !self.source.is_char_boundary(scan_start) {
            scan_start += 1;
        }
        let before = &self.source[scan_start..word_start];
        let mut earlier = before.char_indices().rev().filter_map(|(i, c)| {
            let absolute = scan_start + i;
            (!c.is_whitespace()
                && (absolute == line.code_start
                    || self.source[..absolute].ends_with(char::is_whitespace)))
            .then_some(absolute)
        });
        start_offsets.extend(earlier.by_ref().take(5));
        // Detect omitted positions without running more grammar-hole queries.
        let mut search_truncated = scan_start > line.code_start || earlier.next().is_some();
        for start in start_offsets {
            if offset - start > 256 {
                search_truncated = true;
                break;
            }
            let query = &self.source[start..offset];
            if !index.type_names.iter().any(|(text, _)| starts(text, query)) {
                continue;
            }
            let expected = self.expected_literal_at(
                line,
                TextRange::new(start, word_end),
                references,
                &mut budget,
            );
            let Some(_) = expected.filter(|types| {
                !types.is_empty()
                    && types
                        .iter()
                        .all(|t| t.class_name.as_str() == "ch.njol.skript.classes.ClassInfo")
            }) else {
                continue;
            };
            for (text, registration) in &index.type_names {
                if !starts(text, query) {
                    continue;
                }
                if !pool.may_accept(text, start) {
                    continue;
                }
                let Some(ty) = index
                    .catalog
                    .types()
                    .find(|ty| ty.registration_id.as_str() == registration)
                else {
                    continue;
                };
                let parser = if ty.has_parser {
                    "available"
                } else {
                    "unavailable"
                };
                let contexts = if !ty.has_parser {
                    "not applicable".into()
                } else if ty.parse_contexts.is_empty() {
                    "unconfirmed".into()
                } else {
                    format!(
                        "observed: {}; other contexts unconfirmed",
                        ty.parse_contexts.join(", ")
                    )
                };
                let mut item = self.item(
                    text,
                    text,
                    &format!(
                        "Type / {}; parser {parser}; {}",
                        ty.code_name.as_str(),
                        ty.addon.name
                    ),
                    &format!("type-name:{registration}:{text}"),
                    CompletionItemKind::CLASS,
                    TextRange::new(start, word_end),
                    false,
                )?;
                item.data = Some(serde_json::json!({
                    "syntaxIdentity": format!("type-name:{registration}:{text}"),
                    "typeName": {
                    "definitionId": ty.definition_id.as_str(), "registrationId": registration,
                    "representedClass": ty.original_class.as_str(),
                    "parserAvailable": ty.has_parser,
                    "observedParserContexts": ty.parse_contexts,
                    "targetRequirementsVerified": false,
                }}));
                let fields = vec![
                    ("Type", text.clone()), ("Value class", ty.original_class.0.clone()),
                    ("Parser", parser.into()), ("Parser contexts", contexts),
                    ("Status", "Registered type spelling; enclosing provider target requirements unverified".into()),
                ];
                let body = super::super::hover_text::documentation_body(
                    &ty.documentation,
                    &self.index.documentation_format,
                );
                item.documentation = Some(super::super::hover_text::markup(
                    super::super::hover_text::render_cached(
                        "Type",
                        &fields,
                        Some(&body),
                        &self.index.documentation_format,
                    ),
                    &self.index.documentation_format,
                ));
                pool.push(item);
            }
            return Ok(index.type_names_limited || search_truncated || budget.exhausted());
        }
        Ok(index.type_names_limited || search_truncated || budget.exhausted())
    }
}

#[cfg(test)]
mod tests {
    use super::super::parameters::tests::{catalog, parse};
    use super::*;
    fn names(list: &CompletionList) -> Vec<&CompletionItem> {
        list.items
            .iter()
            .filter(|item| {
                item.data
                    .as_ref()
                    .is_some_and(|d| d["typeName"].is_object())
            })
            .collect()
    }
    #[test]
    fn literal_type_names_reach_early_and_final_type_slots() {
        for source in [
            "on load:\n    set {_v} to \"42\" parsed as n\n",
            "command /public:\n    trigger:\n        set {_v} to \"42\" parsed as n\n",
        ] {
            let (early, final_view, _, _) = parse(source);
            let at = LineIndex::new(source)
                .position(source, source.rfind('n').unwrap() + 1)
                .unwrap();
            let PreparedQuery::Ready(early) = early.at(at, false).unwrap() else {
                panic!("prepared type slot")
            };
            for list in [early, final_view.at(at, false).unwrap()] {
                let number = names(&list)
                    .into_iter()
                    .find(|item| item.label == "number")
                    .expect("registered Number type name");
                assert_eq!(number.kind, Some(CompletionItemKind::CLASS));
                let data = number.data.as_ref().unwrap();
                assert_eq!(data["typeName"]["representedClass"], "java.lang.Number");
                assert_eq!(data["typeName"]["parserAvailable"], true);
                assert_eq!(data["typeName"]["targetRequirementsVerified"], false);
                assert!(
                    !number
                        .detail
                        .as_ref()
                        .unwrap()
                        .contains("runtime parse succeeds")
                );
                assert!(list.items.len() <= MAX_ITEMS);
            }
        }
    }
    #[test]
    fn general_type_positions_keep_no_parser_and_unknown_context_types() {
        let source = "on load:\n    set {_v} to \"42\" parsed as \n";
        let (_, view, _, _) = parse(source);
        let at = LineIndex::new(source)
            .position(source, source.len() - 1)
            .unwrap();
        let list = view.at(at, false).unwrap();
        let proposals = names(&list);
        let object = proposals
            .iter()
            .find(|i| i.label == "object")
            .expect("ClassInfo names need not have parsers");
        assert_eq!(
            object.data.as_ref().unwrap()["typeName"]["parserAvailable"],
            false
        );
        let location = proposals
            .iter()
            .find(|i| i.label == "location")
            .expect("missing context is not unsupported");
        assert!(
            serde_json::to_string(location.documentation.as_ref().unwrap())
                .unwrap()
                .contains("unconfirmed")
        );
        assert!(
            !proposals.iter().any(|i| i.label == "double"),
            "internal code names without user patterns are not type literals"
        );
    }
    #[test]
    fn type_names_do_not_leak_into_strings_comments_or_audience_positions() {
        for (source, needle) in [
            ("on load:\n    send \"parsed as n\"\n", "parsed as n"),
            ("on load:\n    # parsed as n\n", "parsed as n"),
            ("on join:\n    send \"a\" to p\n", "to p"),
        ] {
            let (_, view, _, _) = parse(source);
            let at = LineIndex::new(source)
                .position(source, source.rfind(needle).unwrap() + needle.len())
                .unwrap();
            assert!(names(&view.at(at, false).unwrap()).is_empty(), "{source}");
        }
    }
    #[test]
    fn type_name_edit_uses_utf16_and_preserves_an_exact_right_suffix() {
        let source = "# 日本🦊\non load:\n    set {_v} to \"🦊\" parsed as number\n";
        let (_, view, _, _) = parse(source);
        let start = source.rfind("number").unwrap();
        let at = LineIndex::new(source).position(source, start + 1).unwrap();
        let list = view.at(at, false).unwrap();
        let number = names(&list)
            .into_iter()
            .find(|i| i.label == "number")
            .unwrap();
        let Some(CompletionTextEdit::Edit(edit)) = &number.text_edit else {
            panic!()
        };
        assert_eq!(
            edit.range.start,
            LineIndex::new(source).position(source, start).unwrap()
        );
        assert_eq!(edit.range.end, at);
        assert_eq!(edit.new_text, "n");
        assert_eq!(source, view.source.as_ref());
    }
    #[test]
    fn revoked_provider_proof_removes_type_position_proposals() {
        let source = "on load:\n    set {_v} to \"42\" parsed as n\n";
        let (mut early, mut view, _, _) = parse(source);
        let mut index = CatalogIndex::with_documentation_format(
            &catalog(),
            skript_parser::RawTreeOptions::for_skript_version(2, 15),
            lsp_types::MarkupKind::PlainText,
        );
        let mut host = crate::new_parser_host(parser_wasm::HostConfig {
            syntax_catalog: Some(catalog()),
            ..Default::default()
        })
        .unwrap();
        index.certify_types(&host);
        let index = Arc::new(index);
        early.document.index = Arc::clone(&index);
        view.index = index;
        let at = LineIndex::new(source)
            .position(source, source.rfind('n').unwrap() + 1)
            .unwrap();
        assert!(!names(&view.at(at, false).unwrap()).is_empty());
        assert!(host.load_addon(&[]).is_err());
        assert!(names(&view.at(at, false).unwrap()).is_empty());
        if let PreparedQuery::Ready(list) = early.at(at, false).unwrap() {
            assert!(names(&list).is_empty());
        }
    }
    #[test]
    fn parse_type_bounds_do_not_promise_constant_runtime_success() {
        for value in ["42", "not-a-number"] {
            let source = format!("on load:\n    set {{_v}} to \"{value}\" parsed as number\n");
            let (_, _, result, _) = parse(&source);
            assert!(
                result.syntax.diagnostics.is_empty(),
                "{:?}",
                result.syntax.diagnostics
            );
            let node = result
                .ast
                .nodes
                .iter()
                .find(|node| {
                    node.metadata
                        .iter()
                        .any(|m| m.key == "semantic-mode" && m.value == "parse-type")
                })
                .expect("adopted Core parse target semantics");
            let summary = node.summary.as_ref().unwrap();
            assert_eq!(summary.return_type.as_deref(), Some("java.lang.Number"));
            assert!(
                !node
                    .metadata
                    .iter()
                    .any(|d| d.key == "skript.semantic.value-result")
            );
        }
    }
    #[test]
    fn typed_parse_rejection_keeps_exact_target_range_after_utf16_text() {
        let source = "# 日本🦊\non load:\n    set {_v} to \"🦊\" parsed as object\n";
        let (_, _, result, _) = parse(source);
        let uri = "file:///public/type-target.sk".parse().unwrap();
        let diagnostics = crate::lsp::diagnostics::from_result(&uri, &result).unwrap();
        let diagnostic = diagnostics
            .iter()
            .find(|d| d.message.contains("has no parser"))
            .unwrap_or_else(|| panic!("{diagnostics:#?}"));
        let start = source.rfind("object").unwrap();
        assert_eq!(
            diagnostic.range,
            LineIndex::new(source)
                .range(source, TextRange::new(start, start + 6))
                .unwrap()
        );
        assert!(diagnostic.message.contains("java.lang.Object"));
    }
    #[test]
    fn literal_mode_respects_slot_flags_and_does_not_reuse_expression_only_cache() {
        let source = "on load:\n    consume n\n";
        let (early, mut view, _, _) = parse(source);
        drop(early);
        let start = source.rfind('n').unwrap();
        let edit = TextRange::new(start, start + 1);
        view.accepted_slots.insert(
            (edit.start, edit.end),
            vec![skript_parser::ExpressionExpectedType {
                class_name: syntaxes::ClassName("ch.njol.skript.classes.ClassInfo".into()),
                plural: false,
            }],
        );
        for (pattern, allowed) in [
            ("consume %~classinfo%", false),
            ("consume %*classinfo%", true),
        ] {
            let index = Arc::get_mut(&mut view.index).unwrap();
            let parsed =
                syntax_pattern_parser::syntax::parse(pattern, catalog().plural_rules()).unwrap();
            index.patterns = vec![Pattern {
                documentation: None,
                kind: SyntaxKind::Effect,
                label: pattern.into(),
                detail: "public fixture".into(),
                identity: "public:type-slot".into(),
                graph: continuation::Graph::new(&parsed.elements).unwrap(),
            }];
            let at = LineIndex::new(source).position(source, start + 1).unwrap();
            let list = view.at(at, false).unwrap();
            assert_eq!(
                names(&list).iter().any(|i| i.label == "number"),
                allowed,
                "{pattern}"
            );
        }
    }
    #[test]
    fn incomplete_type_name_sources_never_claim_a_complete_empty_result() {
        let source = "on load:\n    consume n\n";
        let (early, mut view, _, _) = parse(source);
        drop(early);
        let offset = source.rfind('n').unwrap() + 1;
        let at = LineIndex::new(source).position(source, offset).unwrap();
        for empty in [true, false] {
            for limited in [false, true] {
                let index = Arc::get_mut(&mut view.index)
                    .unwrap()
                    .typing
                    .as_mut()
                    .unwrap();
                index.type_names = if empty {
                    Vec::new()
                } else {
                    vec![("zzz".into(), "missing.fixture".into())]
                };
                index.type_names_limited = limited;
                let mut pool = ranking::Pool::new(source, &view.lines, offset, false);
                assert_eq!(
                    view.push_type_names(
                        &view.context[&at.line],
                        offset,
                        offset - 1,
                        offset,
                        true,
                        &mut pool
                    )
                    .unwrap(),
                    limited
                );
            }
        }
    }
    #[test]
    fn omitted_multiword_type_positions_mark_the_result_incomplete() {
        let source = "on load:\n    set {_v} to \"42\" parsed as one two three four five six n\n";
        let (early, mut view, _, _) = parse(source);
        drop(early);
        let end = source.rfind('n').unwrap() + 1;
        let start = end - 1;
        let at = LineIndex::new(source).position(source, end).unwrap();
        let index = Arc::get_mut(&mut view.index)
            .unwrap()
            .typing
            .as_mut()
            .unwrap();
        index.type_names = vec![(
            "one two three four five six number".into(),
            "public:finite-name".into(),
        )];
        index.type_names_limited = false;
        let mut pool = ranking::Pool::new(source, &view.lines, end, false);
        // The finite table is complete and the grammar budget is untouched:
        // incompleteness comes from omitted starts, not either existing flag.
        assert!(
            view.push_type_names(&view.context[&at.line], end, start, end, true, &mut pool)
                .unwrap()
        );
    }

    #[test]
    fn type_spellings_use_core_articles_ordered_identity_and_bounded_fallback() {
        let base = catalog();
        let template = base.types().next().unwrap();
        let mut definitions = Vec::new();
        for (index, (code, noun, pattern)) in [
            ("base", "widget", "(widget|gear)s?"),
            ("article", "a widget", "a widget"),
            ("double", "a a widget", "a a widget"),
            ("local", "une gear", "une gear"),
            ("opaque", "gizmo7", "gizmo[0-9]+"),
            ("caps", "caps", "(?-i:CAPS)"),
            ("bounded", "zzzzzzzz", "(z|y){8}"),
            ("later", "widget", "widget"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut value = template.clone();
            value.registration_id = format!("public:type-name:{index}").into();
            value.definition_id = format!("public:type-definition:{index}").into();
            value.code_name = code.to_owned().into();
            value.type_parse_order = index;
            value.original_class = format!("public.Type{index}").into();
            value.noun.singular = noun.into();
            value.noun.plural = noun.into();
            value.user_input_patterns = vec![pattern.into()];
            definitions.push(syntaxes::Syntax::Type(value));
        }
        let catalog = syntaxes::Catalog::new(syntaxes::CatalogParts {
            syntaxes: definitions,
            converters: base.converters().to_vec(),
            comparators: base.comparators().to_vec(),
            event_values: base.event_values().to_vec(),
            properties: base.properties().to_vec(),
            operators: base.operators().to_vec(),
            operations: base.operations().clone(),
            differences: base.differences().to_vec(),
            classes: base.classes().to_vec(),
            aliases: base.aliases().clone(),
            plural_rules: base.plural_rules().clone(),
            language: std::collections::BTreeMap::from([
                ("genders.0.id".into(), "article".into()),
                ("genders.0.indefinite article".into(), "une".into()),
            ]),
        });
        let host = crate::new_parser_host(parser_wasm::HostConfig {
            syntax_catalog: Some(Arc::new(catalog)),
            runtime_profile: parser_wasm::RuntimeProfile {
                skript_version: Some("2.15.4".into()),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
        let (names, limited) = host.completion_type_names();
        assert!(
            limited,
            "opaque and over-budget patterns retain fallback only"
        );
        for (name, index) in [
            ("widget", 0),
            ("widgets", 0),
            ("gear", 0),
            ("gears", 0),
            ("a widget", 0),
            ("a a widget", 1),
            ("une gear", 0),
            ("gizmo7", 4),
            ("CAPS", 5),
            ("zzzzzzzz", 6),
        ] {
            assert!(
                names
                    .iter()
                    .any(|(text, id)| text == name && id == &format!("public:type-name:{index}")),
                "{name}: {names:?}"
            );
        }
        assert!(!names.iter().any(|(text, _)| text == "caps"));
        assert!(!names.iter().any(|(_, id)| id == "public:type-name:7"));
        // A literal-only article registration is never advertised as though
        // Core would match the unprocessed spelling to that same registration.
        assert!(
            !names
                .iter()
                .any(|(text, id)| text == "a widget" && id == "public:type-name:1")
        );
    }

    #[test]
    fn registered_addon_spellings_include_finite_forms_missing_from_nouns() {
        let catalog = crate::lsp::test_support::multi_addon_catalog();
        let host = crate::new_parser_host(parser_wasm::HostConfig {
            syntax_catalog: Some(Arc::clone(&catalog)),
            ..Default::default()
        })
        .unwrap();
        let (names, limited) = host.completion_type_names();
        // The fixture's response noun/code is singular, but its user pattern
        // accepts only the plural. This tests the registration, not an addon rule.
        let response = catalog
            .types()
            .find(|ty| ty.code_name.as_str() == "response")
            .unwrap();
        assert_ne!(response.addon.name, "Skript");
        assert!(
            names
                .iter()
                .any(|(name, id)| name == "responses" && id == response.registration_id.as_str())
        );
        assert!(
            !names
                .iter()
                .any(|(name, id)| name == "response" && id == response.registration_id.as_str())
        );
        let number = catalog
            .types()
            .find(|ty| ty.code_name.as_str() == "number")
            .unwrap();
        for alias in ["num", "nums", "number", "numbers"] {
            assert!(
                names
                    .iter()
                    .any(|(name, id)| name == alias && id == number.registration_id.as_str()),
                "{alias}; names={}; limited={limited}",
                names.len()
            );
        }
    }
}

use super::*;
use crate::lsp::completion::parameters::tests::{catalog, parse};
fn index_and_host() -> (CatalogIndex, parser_wasm::ParserHost) {
    let catalog = catalog();
    let index = CatalogIndex::with_documentation_format(
        &catalog,
        skript_parser::RawTreeOptions::for_skript_version(2, 15),
        lsp_types::MarkupKind::PlainText,
    );
    let host = crate::new_parser_host(parser_wasm::HostConfig {
        syntax_catalog: Some(catalog),
        ..Default::default()
    })
    .unwrap();
    (index, host)
}
fn old_order(index: &CatalogIndex) -> Vec<(String, String)> {
    index
        .templates
        .iter()
        .filter(|t| !t.finite_capture)
        .map(|t| (t.identity.clone(), render(&t.parts, false)))
        .collect()
}
fn entity_identity(catalog: &Catalog) -> String {
    let common = catalog
        .syntaxes()
        .iter()
        .filter_map(Syntax::common)
        .find(|c| c.element_class.as_str() == "ch.njol.skript.expressions.ExprEntity")
        .unwrap();
    format!("{}:0", common.registration_id.as_str())
}
#[test]
fn finite_capture_values_restore_player_in_prepared_and_final_event_and_command_bodies() {
    for (source, row) in [
        ("on join:\n    send \"a\" to p\n", 1),
        (
            "command /public:\n    trigger:\n        send \"a\" to p\n",
            2,
        ),
    ] {
        let (early, final_view, _, _) = parse(source);
        let line = source.lines().nth(row).unwrap();
        let at = Position::new(row as u32, line.len() as u32);
        let PreparedQuery::Ready(early) = early.at(at, false).unwrap() else {
            panic!("prepared body required")
        };
        let final_list = final_view.at(at, false).unwrap();
        let identity = entity_identity(&catalog());
        for list in [&early, &final_list] {
            assert!(list.items.len() <= MAX_ITEMS);
            let player = list
                .items
                .iter()
                .find(|i| {
                    i.label == "player"
                        && i.data.as_ref().is_some_and(|d| {
                            d["syntaxIdentity"] == identity && d["finiteCaptureValue"] == true
                        })
                })
                .expect("plain registered player proposal");
            assert!(
                player
                    .detail
                    .as_ref()
                    .unwrap()
                    .contains("context unverified")
            );
            assert!(
                player.detail.as_ref().unwrap().contains("dynamic"),
                "never borrow EntityData's represented Player type"
            );
            assert!(
                !list
                    .items
                    .iter()
                    .any(|i| i.label == "pi" || i.label.starts_with("product(")),
                "Number output exclusions remain active"
            );
        }
    }
}
#[test]
fn finite_capture_recertification_preserves_original_order_and_rejects_revoked_proofs() {
    let (mut index, mut host) = index_and_host();
    let old = old_order(&index);
    index.certify_types(&host);
    let count = index.templates.iter().filter(|t| t.finite_capture).count();
    assert!(count > 0 && count <= MAX_SUPPLEMENTS);
    assert_eq!(old_order(&index), old);
    index.certify_types(&host);
    assert_eq!(
        index.templates.iter().filter(|t| t.finite_capture).count(),
        count
    );
    assert_eq!(old_order(&index), old);
    assert!(index.typing.as_ref().unwrap().finite_captures_current());
    assert!(host.load_addon(&[]).is_err());
    assert!(!index.typing.as_ref().unwrap().finite_captures_current());
}
#[test]
fn finite_capture_routes_respect_literal_mode_and_unknown_provider_fallback() {
    let (mut index, host) = index_and_host();
    index.certify_types(&host);
    let catalog = catalog();
    let identity = entity_identity(&catalog);
    for mode in [Some("expressions-only"), Some("not-a-mode"), None] {
        let mut routes = HashMap::new();
        if let Some(mode) = mode {
            routes.insert(
                identity.clone(),
                vec![RegisteredCaptureBinding {
                    capture_index: 0,
                    parser_id: "host.expression".into(),
                    required: true,
                    options: BTreeMap::from([
                        (
                            "expression.expected-types".into(),
                            "ch.njol.skript.entity.EntityData".into(),
                        ),
                        ("parse.mode".into(), mode.into()),
                    ]),
                }],
            );
        }
        rebuild(&mut index, &catalog, &routes);
        assert!(!index.templates.iter().any(|t| t.finite_capture));
        assert!(
            index.templates.iter().any(|t| t.identity == identity),
            "existing template remains available"
        );
    }
}
#[test]
fn finite_capture_matching_uses_native_regex_and_bounded_canonical_values() {
    let catalog = catalog();
    let pattern = syntaxes::Pattern {
        source: "[the] <[0-9]+>".into(),
        parsed: syntax_pattern_parser::syntax::parse("[the] <[0-9]+>", catalog.plural_rules())
            .unwrap(),
    };
    assert_eq!(
        lexically_matches(&pattern, 0, "the 12", "fixture"),
        Ok(true)
    );
    assert_eq!(
        lexically_matches(&pattern, 0, "player", "fixture"),
        Ok(false)
    );
    let (values, _) = finite_values(&catalog, &["ch.njol.skript.entity.EntityData".into()]);
    assert!(values.iter().any(|v| v == "player"));
    assert!(values.len() <= MAX_VALUES);
    assert!(
        values
            .iter()
            .all(|v| v.len() <= 256 && !v.chars().any(char::is_control))
    );
    assert!(
        finite_values(&catalog, &["not.in.the.catalog".into()])
            .0
            .is_empty()
    );
}

#[test]
fn canonical_literal_bytes_are_never_normalized_into_an_unregistered_value() {
    for (text, expected) in [
        ("player", true),
        ("powered creeper", true),
        ("foo  bar", false),
        ("foo\u{a0}bar", false),
        (" player", false),
        ("player\n", false),
        ("", false),
    ] {
        assert_eq!(supported_literal(text), expected, "{text:?}");
    }
}
#[test]
fn native_attempt_limit_reports_incomplete_for_successful_and_rejected_values() {
    let stock = catalog();
    for (regex, matches) in [(".+", true), ("[A-Z]+", false)] {
        let mut expression = stock
            .expressions()
            .find(|e| e.common.element_class.as_str() == "ch.njol.skript.expressions.ExprEntity")
            .unwrap()
            .clone();
        let pattern = format!("[a] [b] [c] <{regex}>");
        expression.common.patterns = vec![syntaxes::Pattern {
            parsed: syntax_pattern_parser::syntax::parse(&pattern, stock.plural_rules()).unwrap(),
            source: pattern,
        }];
        let identity = format!("{}:0", expression.common.registration_id.as_str());
        let mut ty = stock
            .types()
            .find(|t| t.original_class.as_str() == "ch.njol.skript.entity.EntityData")
            .unwrap()
            .clone();
        let literal = ty.type_literals[0].clone();
        ty.type_literals = (0..200)
            .map(|i| {
                let mut value = literal.clone();
                value.text = format!("value{i:03}");
                value
            })
            .collect();
        let c = Arc::new(Catalog::new(syntaxes::CatalogParts {
            syntaxes: vec![Syntax::Expression(expression), Syntax::Type(ty)],
            converters: vec![],
            comparators: vec![],
            event_values: vec![],
            properties: vec![],
            operators: vec![],
            operations: BTreeMap::new(),
            differences: vec![],
            classes: vec![],
            aliases: syntaxes::AliasRegistry {
                aliases: BTreeMap::new(),
                targets: vec![],
            },
            plural_rules: stock.plural_rules().clone(),
            language: BTreeMap::new(),
        }));
        let mut index = CatalogIndex::with_documentation_format(
            &c,
            skript_parser::RawTreeOptions::for_skript_version(2, 15),
            lsp_types::MarkupKind::PlainText,
        );
        assert!(!index.limited);
        let routes = HashMap::from([(
            identity,
            vec![RegisteredCaptureBinding {
                capture_index: 0,
                parser_id: "host.expression".into(),
                required: true,
                options: BTreeMap::from([(
                    "expression.expected-types".into(),
                    "ch.njol.skript.entity.EntityData".into(),
                )]),
            }],
        )]);
        rebuild(&mut index, &c, &routes);
        assert!(
            index.limited,
            "attempt exhaustion must advertise incompleteness even when every native match fails"
        );
        let count = index.templates.iter().filter(|t| t.finite_capture).count();
        if matches {
            assert!(count > 0 && count <= MAX_PATTERN_ATTEMPTS);
            for proposal in index.templates.iter().filter(|t| t.finite_capture) {
                let syntax = c.expressions().next().unwrap();
                assert_eq!(
                    lexically_matches(
                        &syntax.common.patterns[0],
                        0,
                        &proposal.leading,
                        &proposal.identity
                    ),
                    Ok(true)
                );
            }
        } else {
            assert_eq!(count, 0);
        }
    }
}

#[test]
fn expired_capture_route_proof_removes_finite_values_from_actual_early_and_final_menus() {
    let source = "on join:\n    send \"a\" to p\n";
    let (mut early, mut final_view, _, _) = parse(source);
    let (mut index, mut host) = index_and_host();
    index.certify_types(&host);
    let index = Arc::new(index);
    early.document.index = Arc::clone(&index);
    final_view.index = index;
    let at = Position::new(1, 17);
    let lists = |early: &PreparedDocument, final_view: &Document| {
        let PreparedQuery::Ready(early) = early.at(at, false).unwrap() else {
            panic!()
        };
        [early, final_view.at(at, false).unwrap()]
    };
    for list in lists(&early, &final_view) {
        assert!(list.items.iter().any(|i| {
            i.label == "player"
                && i.data
                    .as_ref()
                    .is_some_and(|d| d["finiteCaptureValue"] == true)
        }));
    }
    assert!(host.load_addon(&[]).is_err());
    for list in lists(&early, &final_view) {
        assert!(!list.items.is_empty());
        assert!(!list.items.iter().any(|i| {
            i.data
                .as_ref()
                .is_some_and(|d| d["finiteCaptureValue"] == true)
        }));
    }
}

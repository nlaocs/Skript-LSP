use super::parameters::tests::{catalog, parse};
use super::*;
use skript_parser::{ExpressionExpectedType, expression_types_are_incompatible};
use syntaxes::{ClassName, Multiplicity};
fn expected(class: &str, plural: bool) -> Vec<ExpressionExpectedType> {
    vec![ExpressionExpectedType {
        class_name: ClassName(class.into()),
        plural,
    }]
}
fn position(source: &str, needle: &str) -> Position {
    LineIndex::new(source)
        .position(source, source.rfind(needle).unwrap() + needle.len())
        .unwrap()
}
fn labels(list: &CompletionList) -> Vec<&str> {
    list.items.iter().map(|i| i.label.as_str()).collect()
}
fn declaration_labels(list: &CompletionList) -> Vec<&str> {
    list.items
        .iter()
        .filter(|i| i.kind == Some(CompletionItemKind::VARIABLE))
        .map(|i| i.label.as_str())
        .collect()
}
#[test]
fn native_compatibility_keeps_unknown_and_uses_hierarchy_converters_union_and_cardinality() {
    let c = catalog();
    let actual = |s: &str| vec![ClassName(s.into())];
    let incompatible = |s: &str, m, e: &[ExpressionExpectedType]| {
        expression_types_are_incompatible(&c, &actual(s), true, m, e)
    };
    assert!(!incompatible(
        "org.bukkit.entity.Player",
        Some(Multiplicity::Single),
        &expected("org.bukkit.entity.Entity", false)
    ));
    assert!(c.can_convert("org.bukkit.entity.Player", "org.bukkit.Location"));
    assert!(!incompatible(
        "org.bukkit.entity.Player",
        Some(Multiplicity::Single),
        &expected("org.bukkit.Location", false)
    ));
    assert!(incompatible(
        "java.lang.Number",
        Some(Multiplicity::Single),
        &expected("org.bukkit.entity.Entity", true)
    ));
    for s in ["java.lang.Object", "not.in.the.snapshot"] {
        assert!(!incompatible(
            s,
            Some(Multiplicity::Multiple),
            &expected("org.bukkit.entity.Entity", false)
        ));
    }
    assert!(!expression_types_are_incompatible(
        &c,
        &actual("java.lang.Number"),
        false,
        Some(Multiplicity::Single),
        &expected("org.bukkit.entity.Entity", true)
    ));
    assert!(incompatible(
        "java.lang.Number",
        Some(Multiplicity::Multiple),
        &expected("java.lang.Number", false)
    ));
    assert!(!incompatible(
        "java.lang.Number",
        None,
        &expected("java.lang.Number", false)
    ));
    assert!(!incompatible(
        "java.lang.Number",
        Some(Multiplicity::Multiple),
        &expected("java.lang.Number", true)
    ));
    assert!(!expression_types_are_incompatible(
        &c,
        &[
            ClassName("java.lang.Number".into()),
            ClassName("org.bukkit.entity.Player".into())
        ],
        true,
        None,
        &expected("org.bukkit.entity.Entity", true)
    ));
    assert!(!incompatible(
        "java.lang.Number",
        Some(Multiplicity::Single),
        &expected("missing.Expected", true)
    ));
}
#[test]
fn complete_native_slot_expectation_agrees_early_and_final_without_waiting_for_body() {
    let source = "on load:\n    teleport {_arg_} to {_destination}\n";
    let (early, final_view, _, _) = parse(source);
    let p = position(source, "{_arg_");
    let line = &final_view.context[&p.line];
    let start = source.find("{_arg_").unwrap();
    let edit = TextRange::new(start, start + 7);
    let expected = final_view
        .expected_at(line, edit, true)
        .expect("complete native slot proof");
    assert!(
        expected
            .iter()
            .any(|t| t.class_name.as_str() == "org.bukkit.entity.Entity" && t.plural)
    );
    assert_eq!(
        early
            .document
            .expected_at(&early.document.context[&p.line], edit, false),
        Some(expected)
    );
    let source = "on load:\n    teleport {_arg_}\n";
    let (_, view, _, _) = parse(source);
    let p = position(source, "{_arg_");
    let start = source.find("{_arg_").unwrap();
    assert!(
        view.expected_at(
            &view.context[&p.line],
            TextRange::new(start, start + 7),
            true
        )
        .is_none(),
        "missing destination grammar is not a type proof"
    );
}
#[test]
fn declared_function_return_types_filter_but_object_is_kept() {
    let source = "function choose_number() :: number:\n    return 1\nfunction choose_player() :: player:\n    return player\nfunction choose_unknown() :: object:\n    return player\non load:\n    teleport choose to {_destination}\n";
    let (_, view, _, _) = parse(source);
    let list = view.at(position(source, "teleport choose"), false).unwrap();
    let names = labels(&list);
    assert!(
        names.iter().any(|n| n.starts_with("choose_player(")),
        "{names:?}"
    );
    assert!(
        names.iter().any(|n| n.starts_with("choose_unknown(")),
        "{names:?}"
    );
    assert!(
        !names.iter().any(|n| n.starts_with("choose_number(")),
        "{names:?}"
    );
}
#[test]
fn parameter_candidate_types_follow_current_operand_facts_not_header_after_reassignment() {
    for (middle, text_expected) in [("", true), ("    set {_arg_text} to 1\n", false)] {
        let source = format!(
            "function check_types(arg_text: text, arg_number: number, arg_unknown: object):\n{middle}    {{_arg_}} matches \"x\"\n"
        );
        let (early, view, _, _) = parse(&source);
        let p = position(&source, "{_arg_");
        let list = view.at(p, false).unwrap();
        let names = declaration_labels(&list);
        assert_eq!(
            names.contains(&"{_arg_text}"),
            text_expected,
            "{middle:?}: {names:?}"
        );
        assert!(!names.contains(&"{_arg_number}"), "{names:?}");
        assert!(names.contains(&"{_arg_unknown}"), "{names:?}");
        // Prepared views do not pretend the declaration is a current flow type.
        let PreparedQuery::Ready(list) = early.at(p, false).unwrap() else {
            panic!("early candidates")
        };
        assert!(declaration_labels(&list).contains(&"{_arg_number}"));
    }
}
#[test]
fn opaque_prior_effect_and_revision_mismatch_keep_unknown_variable_candidates() {
    let source = "function check_types(arg_number: number, arg_unknown: object):\n    execute console command \"noop\"\n    teleport {_arg_} to {_destination}\n";
    let (_, mut view, _, _) = parse(source);
    let p = position(source, "{_arg_");
    let list = view.at(p, false).unwrap();
    assert!(declaration_labels(&list).contains(&"{_arg_number}"));
    view.source = Arc::from(source.replace("noop", "nope"));
    let list = view.at(p, false).unwrap();
    assert!(declaration_labels(&list).contains(&"{_arg_number}"));
}
#[test]
fn all_hole_paths_and_regex_unknown_are_preserved_instead_of_signature_ranking() {
    let rules = catalog();
    let parse_pattern =
        |p: &str| syntax_pattern_parser::syntax::parse(p, rules.plural_rules()).unwrap();
    let input = "choose \u{fffc} with value";
    let pos = input.find('\u{fffc}').unwrap();
    let p = parse_pattern("choose (%numbers%|%entities%) with %objects%");
    let g = continuation::Graph::new(&p.elements).unwrap();
    let slots = g
        .hole_slots(input, pos..pos + 3, &mut continuation::Budget::new())
        .unwrap();
    assert_eq!(slots.iter().map(|s| s.0).collect::<HashSet<_>>().len(), 2);
    let p = parse_pattern("choose <.+> with %objects%");
    let g = continuation::Graph::new(&p.elements).unwrap();
    let slots = g
        .hole_slots(input, pos..pos + 3, &mut continuation::Budget::new())
        .unwrap();
    assert_eq!(slots.iter().map(|s| s.0).collect::<HashSet<_>>().len(), 1);
    assert!(g.slot_types[slots[0].0].is_none());
}
#[test]
fn native_function_argument_mapping_reuses_named_optional_and_defers_list_binding() {
    let source = "function accept_target(target: entity, amount: number = 1):\n    send \"ready\" to console\non load:\n    accept_target(amount: 1, target: {_arg_})\n";
    let (_, view, _, _) = parse(source);
    let p = position(source, "{_arg_");
    let start = source.find("{_arg_").unwrap();
    let e = view
        .expected_at(
            &view.context[&p.line],
            TextRange::new(start, start + 7),
            true,
        )
        .expect("native named binder");
    assert!(
        e.iter()
            .any(|t| t.class_name.as_str() == "org.bukkit.entity.Entity")
    );
    let definition = view
        .functions
        .iter()
        .find(|f| f.name == "accept_target")
        .unwrap()
        .definition
        .as_ref()
        .unwrap();
    assert!(
        definition
            .argument_expectations(
                "accept_target(target: x)",
                skript_parser::FunctionVersionPolicy::modern()
            )
            .is_some()
    );
    // Legacy grammar treats the colon source as one positional expression;
    // the binder does not claim it is a named-argument mapping.
    let legacy = definition
        .argument_expectations(
            "accept_target(target: x)",
            skript_parser::FunctionVersionPolicy::legacy_2_6_4(),
        )
        .unwrap();
    assert_eq!(
        legacy[0].0.slice("accept_target(target: x)"),
        Some("target: x")
    );
    let mut list = definition.as_ref().clone();
    list.parameters.truncate(1);
    list.parameters[0].single = false;
    assert!(
        list.argument_expectations(
            "accept_target(x, y)",
            skript_parser::FunctionVersionPolicy::modern()
        )
        .is_none()
    );
}

#[test]
fn native_literal_boundary_spaces_and_adjacent_regex_paths_are_not_lost() {
    let rules = catalog();
    for pattern in [
        "[:force] teleport %entities% (to|%direction%) %location% [[while] retaining %-teleportflags%]",
        "teleport %entities% to %location% ",
    ] {
        let parsed = syntax_pattern_parser::syntax::parse(pattern, rules.plural_rules()).unwrap();
        let graph = continuation::Graph::new(&parsed.elements).unwrap();
        let input = "teleport \u{fffc} to {_destination}";
        let start = input.find('\u{fffc}').unwrap();
        let slots = graph
            .hole_slots(input, start..start + 3, &mut continuation::Budget::new())
            .unwrap();
        assert!(
            slots.iter().any(
                |(parameter, _)| graph.slot_types[*parameter].as_ref().is_some_and(|ty| ty
                    .alternatives
                    .iter()
                    .any(|a| a.name == "entity" && a.plural))
            ),
            "{pattern}: {slots:?}"
        );
    }
    let parsed =
        syntax_pattern_parser::syntax::parse("take <[A-Z]+>%entities%", rules.plural_rules())
            .unwrap();
    let graph = continuation::Graph::new(&parsed.elements).unwrap();
    let input = "take ID\u{fffc}tail";
    let start = input.find('\u{fffc}').unwrap();
    let slots = graph
        .hole_slots(input, start..start + 3, &mut continuation::Budget::new())
        .unwrap();
    assert!(
        slots
            .iter()
            .any(|(parameter, _)| graph.slot_types[*parameter].is_none())
    );
    assert!(
        slots
            .iter()
            .any(|(parameter, _)| graph.slot_types[*parameter].is_some())
    );
}

#[test]
fn body_domain_is_native_and_intrinsic_or_incomplete_expressions_keep_unknown() {
    use skript_parser::{BodySyntaxDomain as D, SectionBodyMode as M};
    assert!(D::for_node(M::Trigger, RawNodeKind::Simple).includes(SyntaxKind::Effect, false));
    assert!(!D::for_node(M::Trigger, RawNodeKind::Simple).includes(SyntaxKind::Section, false));
    assert!(D::for_node(M::Trigger, RawNodeKind::Section).includes(SyntaxKind::Expression, true));
    assert!(!D::for_node(M::Trigger, RawNodeKind::Section).includes(SyntaxKind::Expression, false));
    assert!(!D::for_node(M::Conditions, RawNodeKind::Section).includes(SyntaxKind::Section, false));
    assert!(!D::for_node(M::Conditions, RawNodeKind::Simple).allows_function_call());
    let rules = catalog();
    for text in ["(x)", "x and y", "x + y"] {
        assert!(
            skript_parser::expression_has_intrinsic_shape(&rules, text),
            "{text}"
        );
    }
    let source = "on load:\n    teleport ({_arg_}) to {_destination}\n";
    let (_, view, _, _) = parse(source);
    let p = position(source, "{_arg_");
    let start = source.find("{_arg_").unwrap();
    assert!(
        view.expected_at(
            &view.context[&p.line],
            TextRange::new(start, start + 7),
            true
        )
        .is_none()
    );
}

#[test]
fn registered_expression_candidates_use_native_slot_types_early_and_final() {
    let source = "on load:\n    kill proj\n";
    let (early, view, _, _) = parse(source);
    let p = position(source, "kill proj");
    let list = view.at(p, false).unwrap();
    assert!(
        !labels(&list)
            .iter()
            .any(|label| label.contains("projectile force"))
    );
    let PreparedQuery::Ready(list) = early.at(p, false).unwrap() else {
        panic!("prepared candidates")
    };
    assert!(
        !labels(&list)
            .iter()
            .any(|label| label.contains("projectile force"))
    );
    let source = "function accept_number(value: number):\n    send \"ready\" to console\non load:\n    accept_number(proj)\n";
    let (_, view, _, _) = parse(source);
    let list = view
        .at(position(source, "accept_number(proj"), false)
        .unwrap();
    assert!(
        labels(&list)
            .iter()
            .any(|label| label.contains("projectile force")),
        "{:?}",
        labels(&list)
    );
    // Dynamic literal metadata cannot be treated as a static negative proof.
    let source = "on load:\n    teleport max to {_destination}\n";
    let (_, view, _, _) = parse(source);
    let list = view.at(position(source, "teleport max"), false).unwrap();
    assert!(
        labels(&list)
            .iter()
            .any(|label| label.contains("integer value")),
        "{:?}",
        labels(&list)
    );
}

#[test]
fn function_cardinality_and_converters_reuse_the_native_rules() {
    let source = "function choose_plural() :: players:\n    return all players\nfunction choose_player() :: player:\n    return player\nfunction choose_unknown() :: object:\n    return player\nfunction accept_location(value: location):\n    send \"ready\" to console\non load:\n    accept_location(choose)\n";
    let (_, view, _, _) = parse(source);
    let list = view
        .at(position(source, "accept_location(choose"), false)
        .unwrap();
    let names = labels(&list);
    assert!(
        names.iter().any(|n| n.starts_with("choose_player(")),
        "{names:?}"
    );
    assert!(
        names.iter().any(|n| n.starts_with("choose_unknown(")),
        "{names:?}"
    );
    assert!(
        !names.iter().any(|n| n.starts_with("choose_plural(")),
        "{names:?}"
    );
}

#[test]
fn actual_scalar_replacement_checks_passive_literal_and_regex_parent_competitors() {
    let source =
        "function check_types(arg_text: text, arg_number: number):\n    {_arg_} matches \"x\"\n";
    for (competing, reachable) in [
        ("{_arg_number} matches %strings%", true),
        ("<.+> matches %strings%", true),
        ("{_%objects%} matches %strings%", true),
        ("%objects%} matches %strings%", false),
        ("{%objects%%objects%} matches %strings%", true),
    ] {
        let (early, mut view, _, _) = parse(source);
        drop(early);
        let p = position(source, "{_arg_");
        assert!(!declaration_labels(&view.at(p, false).unwrap()).contains(&"{_arg_number}"));
        let parsed =
            syntax_pattern_parser::syntax::parse(competing, catalog().plural_rules()).unwrap();
        Arc::get_mut(&mut view.index)
            .unwrap()
            .patterns
            .push(Pattern {
                documentation: None,
                kind: SyntaxKind::Effect,
                label: competing.into(),
                detail: "passive Catalog fixture".into(),
                identity: "fixture:passive:0".into(),
                graph: continuation::Graph::new(&parsed.elements).unwrap(),
            });
        // A balanced native Variable cannot end before its closing brace;
        // an extra literal closing brace is an unreachable negative control.
        assert_eq!(
            declaration_labels(&view.at(p, false).unwrap()).contains(&"{_arg_number}"),
            reachable,
            "{competing}"
        );
    }
}

#[test]
fn variable_facts_respect_branch_union_opaque_barriers_scalar_names_and_teleport_order() {
    for (middle, expected_text) in [
        (
            "    if chance of 100%:\n        set {_arg_text} to 1\n",
            true,
        ),
        ("    set {_arg_text} to 1\n", false),
    ] {
        let source = format!(
            "function typed(arg_text: text, arg_number: number, arg_unknown: object, arg_%%Number: number):\n{middle}    {{_arg_}} matches \"x\"\n"
        );
        let (_, view, _, _) = parse(&source);
        let list = view.at(position(&source, "{_arg_"), false).unwrap();
        let names = declaration_labels(&list);
        assert_eq!(
            names.contains(&"{_arg_text}"),
            expected_text,
            "{middle}: {names:?}"
        );
        assert!(!names.contains(&"{_arg_number}"), "{names:?}");
        assert!(!names.contains(&"{_arg_%%%%Number}"), "{names:?}");
        assert!(names.contains(&"{_arg_unknown}"), "{names:?}");
    }
    for body in [
        "    teleport {_arg_} to {_destination}\n",
        "    execute console command \"noop\"\n    {_arg_} matches \"x\"\n",
    ] {
        let source = format!("function typed(arg_number: number, arg_unknown: object):\n{body}");
        let (_, view, _, _) = parse(&source);
        assert!(
            declaration_labels(&view.at(position(&source, "{_arg_"), false).unwrap())
                .contains(&"{_arg_number}"),
            "{body}"
        );
    }
}

#[test]
fn differently_owned_parent_literal_candidates_do_not_change_a_definitions_output_type() {
    for (source, needle, competing, candidate) in [
        (
            "on load:\n    kill proj\n",
            "kill proj",
            "kill projectile force",
            "projectile force",
        ),
        (
            "function choose_number() :: number:\n    return 1\non load:\n    teleport choose to {_destination}\n",
            "teleport choose",
            "teleport choose_number\\(\\) to %location%",
            "choose_number()",
        ),
    ] {
        let (early, mut view, _, _) = parse(source);
        drop(early);
        let p = position(source, needle);
        assert!(
            !labels(&view.at(p, false).unwrap()).contains(&candidate),
            "{candidate}"
        );
        let parsed =
            syntax_pattern_parser::syntax::parse(competing, catalog().plural_rules()).unwrap();
        Arc::get_mut(&mut view.index)
            .unwrap()
            .patterns
            .push(Pattern {
                documentation: None,
                kind: SyntaxKind::Effect,
                label: competing.into(),
                detail: "passive Catalog fixture".into(),
                identity: "fixture:literal:0".into(),
                graph: continuation::Graph::new(&parsed.elements).unwrap(),
            });
        let list = view.at(p, false).unwrap();
        assert!(
            !list.items.iter().any(|item| {
                (item.kind == Some(CompletionItemKind::FUNCTION) && item.label == candidate)
                    || (item
                        .data
                        .as_ref()
                        .is_some_and(|d| d["syntaxKind"] == "Expression")
                        && item.label == candidate)
            }),
            "own definition remains incompatible: {candidate}"
        );
        assert!(
            list.items.iter().any(|item| item
                .data
                .as_ref()
                .is_some_and(|d| d["syntaxIdentity"] == "fixture:literal:0")),
            "separate parent syntax remains available: {:?}",
            labels(&list)
        );
    }
}

#[test]
fn new_required_calls_use_their_declared_type_but_name_only_existing_calls_keep_unknown() {
    for (line, kept) in [
        ("teleport choose to {_destination}", false),
        ("teleport choose(1) to {_destination}", true),
    ] {
        let source = format!(
            "function choose_number(value: number) :: number:\n    return 1\non load:\n    {line}\n"
        );
        let (_, view, _, _) = parse(&source);
        for snippets in [false, true] {
            let list = view
                .at(position(&source, "teleport choose"), snippets)
                .unwrap();
            assert_eq!(
                labels(&list)
                    .iter()
                    .any(|label| label.starts_with("choose_number(")),
                kept,
                "{:?}",
                labels(&list)
            );
        }
    }
}

#[test]
fn ambiguous_parent_keeps_current_known_variable_instead_of_selecting_a_capture() {
    let source = "function typed(arg_number: number):\n    send \"ready\" to {_arg_}\n";
    let (_, view, _, _) = parse(source);
    let start = source.find("{_arg_}").unwrap();
    let edit = TextRange::new(start, start + "{_arg_}".len());
    let (ty, _, _) = view
        .semantic_types
        .as_ref()
        .unwrap()
        .completion_input(source, edit, "arg_number")
        .expect("current operand facts");
    assert!(
        matches!(ty, crate::lsp::semantic::InferredType::Values { classes, .. }
        if classes.contains("java.lang.Number"))
    );
    assert!(
        declaration_labels(&view.at(position(source, "{_arg_"), false).unwrap())
            .contains(&"{_arg_number}")
    );
}

#[test]
fn optional_overloads_with_identical_insertions_keep_each_compatible_return() {
    let source = "function choose(value: number = 1) :: number:\n    return 1\nfunction choose(value: text = \"\") :: player:\n    return player\nfunction choose(value: boolean = true) :: object:\n    return player\non load:\n    teleport choose to {_destination}\n";
    let (_, view, _, _) = parse(source);
    assert_eq!(
        view.functions.iter().filter(|f| f.name == "choose").count(),
        3
    );
    let list = view.at(position(source, "teleport choose"), false).unwrap();
    let functions = list
        .items
        .iter()
        .filter(|i| i.kind == Some(CompletionItemKind::FUNCTION) && i.label.starts_with("choose("))
        .collect::<Vec<_>>();
    let names = functions
        .iter()
        .map(|i| i.label.as_str())
        .collect::<Vec<_>>();
    assert!(
        !names.iter().any(|n| n.contains("java.lang.Number")),
        "{names:?}"
    );
    assert!(
        names.iter().any(|n| n.contains("java.lang.String")),
        "{names:?}"
    );
    assert!(
        names.iter().any(|n| n.contains("java.lang.Boolean")),
        "{names:?}"
    );
    for item in functions {
        assert!(
            matches!(&item.text_edit, Some(CompletionTextEdit::Edit(e)) if e.new_text == "choose()")
        );
    }
}

#[test]
fn unicode_inside_a_capture_is_not_trimmed_into_the_candidate() {
    let source = "# 日本語\non load:\n    kill \u{a0}proj\n";
    let (early, mut view, _, _) = parse(source);
    drop(early);
    let parsed =
        syntax_pattern_parser::syntax::parse("\u{a0}projectile force", catalog().plural_rules())
            .unwrap();
    Arc::get_mut(&mut view.index)
        .unwrap()
        .patterns
        .push(Pattern {
            documentation: None,
            kind: SyntaxKind::Expression,
            label: "\u{a0}projectile force".into(),
            detail: "passive Catalog Entity expression".into(),
            identity: "fixture:unicode-expression:0".into(),
            graph: continuation::Graph::new(&parsed.elements).unwrap(),
        });
    let p = position(source, "kill \u{a0}proj");
    let start = source.rfind("proj").unwrap();
    assert!(
        view.expected_at(
            &view.context[&p.line],
            TextRange::new(start, start + 4),
            true
        )
        .is_none()
    );
    assert!(
        labels(&view.at(p, false).unwrap())
            .iter()
            .any(|label| label.contains("projectile force"))
    );
    let input = " \t\u{a0}now\u{a0}\u{1f}";
    assert_eq!(
        skript_parser::java_trim_range(input).slice(input),
        Some("\u{a0}now\u{a0}")
    );
}

#[test]
fn candidate_definition_does_not_borrow_a_different_literal_interpretations_type() {
    let source = "function accept_flag(value: boolean):\n    send \"ready\" to console\non load:\n    accept_flag(tr)\n";
    let (_, view, _, _) = parse(source);
    let p = position(source, "accept_flag(tr");
    let line = &view.context[&p.line];
    let start = source.rfind("tr)").unwrap();
    let edit = TextRange::new(start, start + 2);
    let wanted = view.expected_at(line, edit, true).unwrap();
    assert_eq!(wanted, expected("java.lang.Boolean", false));
    // Passive proposal fixture: fixed Number registration with the same spelling
    // as a native Boolean literal. Its own declared output is still Number.
    let mut template = view
        .index
        .templates
        .iter()
        .find(|t| t.label.contains("projectile force"))
        .unwrap()
        .clone();
    template.parts = vec![Part::Text("true".into())];
    let item = CompletionItem {
        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
            range: view.lines.range(source, edit).unwrap(),
            new_text: "true".into(),
        })),
        ..CompletionItem::default()
    };
    assert!(view.incompatible_template(
        &template,
        Some(&wanted),
        line,
        edit,
        &item,
        false,
        true,
        &mut continuation::Budget::new()
    ));
    // This is independent of native accepting that text as another candidate.
    let (_, _, result, _) = parse(&source.replace("accept_flag(tr)", "accept_flag(true)"));
    assert!(
        result.syntax.diagnostics.is_empty(),
        "{:?}",
        result.syntax.diagnostics
    );
    assert!(
        result.effects.diagnostics.is_empty(),
        "{:?}",
        result.effects.diagnostics
    );
}

#[test]
fn declared_function_return_does_not_borrow_a_possible_literal_return_type() {
    let source = "function choose_number() :: number:\n    return 1\nfunction accept_flag(value: boolean):\n    send \"ready\" to console\non load:\n    accept_flag(choose)\n";
    let (_, view, _, _) = parse(source);
    let list = view
        .at(position(source, "accept_flag(choose"), false)
        .unwrap();
    assert!(
        !labels(&list)
            .iter()
            .any(|n| n.starts_with("choose_number("))
    );
}

#[test]
fn identical_insertions_filter_each_registration_by_its_own_output_type() {
    for with_slots in [false, true] {
        let source = "on load:\n    kill probe\n";
        let (early, mut view, _, _) = parse(source);
        drop(early);
        let p = position(source, "kill probe");
        let start = source.rfind("probe").unwrap();
        let wanted = view
            .expected_at(
                &view.context[&p.line],
                TextRange::new(start, start + 5),
                true,
            )
            .unwrap();
        assert!(
            wanted
                .iter()
                .any(|t| t.class_name.as_str() == "org.bukkit.entity.Entity")
        );
        let rules = catalog();
        let base = rules
            .expressions()
            .find(|e| {
                e.common
                    .patterns
                    .iter()
                    .any(|p| p.source.contains("projectile force"))
            })
            .unwrap();
        let index = Arc::get_mut(&mut view.index).unwrap();
        let template = index
            .templates
            .iter()
            .find(|t| t.label.contains("projectile force"))
            .unwrap()
            .clone();
        for (name, class, state) in [
            (
                "number",
                "java.lang.Number",
                syntaxes::ReturnTypeState::Static,
            ),
            (
                "entity",
                "org.bukkit.entity.Entity",
                syntaxes::ReturnTypeState::Static,
            ),
            (
                "unknown",
                "java.lang.Object",
                syntaxes::ReturnTypeState::Dynamic,
            ),
        ] {
            let identity = format!("fixture:{name}:0");
            let mut definition = base.clone();
            definition.return_type = Some(ClassName(class.into()));
            definition.return_type_state = state;
            definition.return_type_multiplicity_state = syntaxes::ResolutionState::Resolved;
            definition.return_type_multiplicity = Some(Multiplicity::Single);
            index
                .typing
                .as_mut()
                .unwrap()
                .insert(&identity, &definition);
            let mut proposal = template.clone();
            proposal.label = "probe".into();
            proposal.leading = "probe".into();
            proposal.folded = "probe".into();
            proposal.parts = if with_slots {
                vec![Part::Text("probe ".into()), Part::Slot("object".into())]
            } else {
                vec![Part::Text("probe".into())]
            };
            proposal.identity = identity;
            proposal.detail = format!("passive registered {name} output");
            index.templates.push(proposal);
        }

        for snippets in [false, true] {
            let list = view.at(p, snippets).unwrap();
            let proposals = list
                .items
                .iter()
                .filter(|i| {
                    i.data
                        .as_ref()
                        .and_then(|d| d["syntaxIdentity"].as_str())
                        .is_some_and(|id| id.starts_with("fixture:"))
                })
                .collect::<Vec<_>>();
            let ids = proposals
                .iter()
                .map(|i| i.data.as_ref().unwrap()["syntaxIdentity"].as_str().unwrap())
                .collect::<HashSet<_>>();
            assert_eq!(
                ids,
                HashSet::from(["fixture:entity:0", "fixture:unknown:0"]),
                "slots={with_slots}, snippets={snippets}"
            );
            let insertions = proposals
                .iter()
                .map(|i| match &i.text_edit {
                    Some(CompletionTextEdit::Edit(e)) => e.new_text.as_str(),
                    _ => panic!("text edit"),
                })
                .collect::<HashSet<_>>();
            assert_eq!(insertions.len(), 1);
        }
    }
}

#[test]
fn prefix_using_a_right_suffix_does_not_confuse_an_inner_slot_with_the_proposals_output() {
    let source = "on load:\n    consume na of {_x}\n";
    let (early, mut view, _, _) = parse(source);
    drop(early);
    let rules = catalog();
    let index = Arc::get_mut(&mut view.index).unwrap();
    index.patterns.clear();
    for (pattern, kind) in [
        ("consume %strings%", SyntaxKind::Effect),
        ("%numbers% of %objects%", SyntaxKind::Expression),
    ] {
        let parsed = syntax_pattern_parser::syntax::parse(pattern, rules.plural_rules()).unwrap();
        index.patterns.push(Pattern {
            documentation: None,
            kind,
            label: pattern.into(),
            detail: "passive parent extent fixture".into(),
            identity: pattern.into(),
            graph: continuation::Graph::new(&parsed.elements).unwrap(),
        });
    }
    let mut proposal = index
        .templates
        .iter()
        .find(|t| t.label.contains("projectile force"))
        .unwrap()
        .clone();
    let mut definition = rules
        .expressions()
        .find(|e| {
            e.common
                .patterns
                .iter()
                .any(|p| p.source.contains("projectile force"))
        })
        .unwrap()
        .clone();
    definition.return_type = Some(ClassName("java.lang.String".into()));
    proposal.identity = "fixture:outer-string:0".into();
    index
        .typing
        .as_mut()
        .unwrap()
        .insert(&proposal.identity, &definition);
    proposal.parts = vec![Part::Text("name of ".into()), Part::Slot("object".into())];
    proposal.leading = "name of".into();
    proposal.folded = "name of".into();
    index.templates.clear();
    index.templates.push(proposal);
    let p = position(source, "consume na");
    let start = source.rfind("na of").unwrap();
    assert_eq!(
        view.expected_at(
            &view.context[&p.line],
            TextRange::new(start, start + 2),
            true
        ),
        Some(expected("java.lang.Number", true))
    );
    for snippets in [false, true] {
        let list = view.at(p, snippets).unwrap();
        let proposal = list
            .items
            .iter()
            .find(|i| {
                i.data
                    .as_ref()
                    .is_some_and(|d| d["syntaxIdentity"] == "fixture:outer-string:0")
            })
            .unwrap();
        assert!(
            matches!(&proposal.text_edit, Some(CompletionTextEdit::Edit(e)) if e.new_text == "name")
        );
    }
}

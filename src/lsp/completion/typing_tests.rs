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
    for (middle, text_expected, number_expected) in [
        ("", true, true),
        ("    set {_arg_number} to 1\n", true, false),
        ("    set {_arg_text} to 1\n", false, true),
    ] {
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
        assert_eq!(
            names.contains(&"{_arg_number}"),
            number_expected,
            "{names:?}"
        );
        assert!(names.contains(&"{_arg_unknown}"), "{names:?}");
        // Prepared views do not pretend the declaration is a current flow type.
        let PreparedQuery::Ready(list) = early.at(p, false).unwrap() else {
            panic!("early candidates")
        };
        assert!(declaration_labels(&list).contains(&"{_arg_number}"));
    }
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
    // This suffix-preserving edit inserts only "max". Its expression extent
    // is unknown even if a displayed full literal has a certified output class.
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
fn adopted_variable_type_is_not_borrowed_from_other_parent_patterns() {
    let source = "function check_types(arg_text: text, arg_number: number):\n    set {_arg_number} to 1\n    {_arg_} matches \"x\"\n";
    let (early, mut view, _, _) = parse(source);
    drop(early);
    let p = position(source, "{_arg_");
    assert!(!declaration_labels(&view.at(p, false).unwrap()).contains(&"{_arg_number}"));
    let original_patterns = view.index.patterns.len();
    for competing in [
        "{_arg_number} matches %strings%",
        "<.+> matches %strings%",
        "{_%objects%} matches %strings%",
        "%objects%} matches %strings%",
        "{%objects%%objects%} matches %strings%",
    ] {
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
        assert!(
            !declaration_labels(&view.at(p, false).unwrap()).contains(&"{_arg_number}"),
            "{competing}"
        );
        let removed = Arc::get_mut(&mut view.index)
            .unwrap()
            .patterns
            .pop()
            .unwrap();
        assert_eq!(removed.identity, "fixture:passive:0");
        assert_eq!(view.index.patterns.len(), original_patterns);
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
            "function typed(arg_text: text, arg_number: number, arg_unknown: object, arg_%%Number: number):\n{middle}    set {{_arg_number}} to 1\n    {{_arg_}} matches \"x\"\n"
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
        assert!(
            names.contains(&"{_arg_%%%%Number}"),
            "nullable escaped parameter must remain unknown: {names:?}"
        );
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
fn adopted_audience_capture_keeps_uncertain_and_stale_facts_but_filters_present_number() {
    for (middle, known, kept) in [
        ("", Some(("java.lang.Number", true)), true),
        (
            "    set {_arg_number} to 1\n",
            Some(("java.lang.Long", false)),
            false,
        ),
        (
            "    set {_arg_number} to 1\n    execute console command \"noop\"\n",
            None,
            true,
        ),
    ] {
        let source = format!(
            "function typed(arg_number: number):\n{middle}    send \"ready\" to {{_arg_}}\n"
        );
        let (_, mut view, result, _) = parse(&source);
        assert!(
            result.syntax.diagnostics.is_empty(),
            "{:?}",
            result.syntax.diagnostics
        );
        let start = source.find("{_arg_}").unwrap();
        let edit = TextRange::new(start, start + "{_arg_}".len());
        assert!(
            view.accepted_slots.contains_key(&(edit.start, edit.end)),
            "actual adopted capture required"
        );
        let semantic = view.semantic_types.as_ref().unwrap();
        let incoming = semantic.completion_input(&source, edit, "arg_number");
        match known {
            Some((class, maybe_unset)) => {
                let (ty, _, _) = incoming.expect("incoming facts");
                assert!(
                    matches!(ty,crate::lsp::semantic::InferredType::Values{classes,maybe_unset:actual,..} if classes.len()==1 && classes.contains(class) && *actual==maybe_unset),
                    "{ty:?}"
                );
            }
            None => assert!(
                incoming.is_none(),
                "opaque effect must withdraw the completion point and prior Long fact"
            ),
        }
        let p = position(&source, "{_arg_");
        assert_eq!(
            declaration_labels(&view.at(p, false).unwrap()).contains(&"{_arg_number}"),
            kept,
            "{source}"
        );
        if !kept {
            let edited = source.replace("ready", "other");
            assert_eq!(edited.len(), source.len());
            assert!(
                semantic
                    .completion_input(&edited, edit, "arg_number")
                    .is_none(),
                "changed source cannot reuse a proven Long fact"
            );
            view.source = Arc::from(edited);
            assert!(
                declaration_labels(&view.at(p, false).unwrap()).contains(&"{_arg_number}"),
                "source edit must withdraw the previous exclusion"
            );
        }
    }
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

#[test]
fn adopted_capture_resolves_overlapping_object_and_audience_slots() {
    let source = "on load:\n    send \"test\" to player\n";
    let (early, view, result, _) = parse(source);
    assert!(
        result.syntax.diagnostics.is_empty(),
        "{:?}",
        result.syntax.diagnostics
    );
    let start = source.rfind("player").unwrap();
    let p = LineIndex::new(source).position(source, start + 6).unwrap();
    let edit = TextRange::new(start, start + "player".len());
    let adopted_expected = view
        .expected_at(&view.context[&p.line], edit, true)
        .unwrap();
    assert_eq!(
        adopted_expected,
        expected("net.kyori.adventure.audience.Audience", true)
    );
    // Query the actual reported word end. Middle-word suffix preservation may
    // independently suppress a new call; do not attribute that to type filtering.
    for name in ["product", "permutations"] {
        let candidate = view
            .index
            .functions
            .iter()
            .find(|f| f.name == name)
            .unwrap();
        assert!(
            view.incompatible_function(candidate, Some(&adopted_expected)),
            "{name}"
        );
    }
    let candidate = view
        .index
        .functions
        .iter()
        .find(|f| f.name == "player")
        .unwrap();
    assert!(!view.incompatible_function(candidate, Some(&adopted_expected)));
    let pi = view
        .index
        .templates
        .iter()
        .find(|t| t.kind == SyntaxKind::Expression && t.folded == "pi")
        .unwrap();
    let (item, whole) = view
        .template_item(pi, "pi", start, start + 6, &view.context[&p.line], false)
        .unwrap();
    assert!(view.incompatible_template(
        pi,
        Some(&adopted_expected),
        &view.context[&p.line],
        edit,
        &item,
        whole,
        true,
        &mut continuation::Budget::new()
    ));
    let list = view.at(p, false).unwrap();
    let names = labels(&list);
    assert!(names.iter().any(|n| n.starts_with("player(")), "{names:?}");
    for name in ["product(", "permutations("] {
        assert!(!names.iter().any(|n| n.starts_with(name)), "{names:?}");
    }
    assert!(
        !list.items.iter().any(|i| i.label == "pi"
            && i.data
                .as_ref()
                .is_some_and(|d| d["syntaxKind"] == "Expression")),
        "{names:?}"
    );
    // No adopted AST is needed when the provider closes the child positions
    // of a proposal. Another opaque leaf does not own this proposal node.
    assert_eq!(
        early
            .document
            .expected_at(&early.document.context[&p.line], edit, false),
        Some(expected("net.kyori.adventure.audience.Audience", true))
    );
}

#[test]
fn provider_guaranteed_literal_class_filters_without_claiming_cardinality() {
    let source = "on load:\n    kill pi\n";
    let (early, view, _, _) = parse(source);
    let p = position(source, "kill pi");
    for list in [
        view.at(p, false).unwrap(),
        match early.at(p, false).unwrap() {
            PreparedQuery::Ready(l) => l,
            _ => panic!("prepared"),
        },
    ] {
        assert!(
            !list.items.iter().any(|i| i.label == "pi"
                && i.data
                    .as_ref()
                    .is_some_and(|d| d["syntaxKind"] == "Expression")),
            "{:?}",
            labels(&list)
        );
    }
    let source = "on load:\n    broadcast pi\n";
    let (_, view, _, _) = parse(source);
    let list = view.at(position(source, "broadcast pi"), false).unwrap();
    assert!(labels(&list).contains(&"pi"), "{:?}", labels(&list));
}

#[test]
fn native_function_result_assignment_reaches_hover_and_variable_type_filter() {
    use crate::lsp::semantic::InferredType;
    for alias in ["string", "text"] {
        let source = format!(
            "function test() :: {alias}:\n    return \"test\"\nfunction inspect(arg_number: number):\n    set {{_aaa}} to test()\n    {{_aaa}} matches \"x\"\n"
        );
        let (_, view, result, _) = parse(&source);
        assert!(
            result.syntax.diagnostics.is_empty(),
            "{:?}",
            result.syntax.diagnostics
        );
        let p = position(&source, "{_aaa");
        let (_, ty) = view.semantic_types.as_ref().unwrap().type_at(p).unwrap();
        assert!(
            matches!(ty, InferredType::Values { classes, multiple: false, maybe_unset: true, .. } if classes == &std::collections::BTreeSet::from(["java.lang.String".into()])),
            "{alias}: {ty:?}"
        );
        let list = view.at(p, false).unwrap();
        assert!(declaration_labels(&list).contains(&"{_aaa}"));
        // Recipients are observed before message conversion callbacks. This
        // checks the known String at the adopted Audience lookup.
        let source = source.replace("{_aaa} matches \"x\"", "send \"test\" to {_a}");
        let (early, view, _, _) = parse(&source);
        let start = source.rfind("{_a}").unwrap();
        let edit = TextRange::new(start, start + "{_a}".len());
        let input = view
            .semantic_types
            .as_ref()
            .unwrap()
            .completion_input(&source, edit, "aaa");
        assert!(
            input.is_some(),
            "missing current operand for {alias}; current variable: {:?}",
            view.semantic_types
                .as_ref()
                .unwrap()
                .type_at(position(&source, "{_a"))
        );
        assert!(
            matches!(input.unwrap().0, InferredType::Values { classes, .. } if classes.contains("java.lang.String")),
            "{:?}",
            input.map(|(ty, requested, _)| (ty, requested))
        );
        assert!(
            view.accepted_slots.contains_key(&(edit.start, edit.end)),
            "no adopted capture: {:?}",
            view.accepted_slots
        );
        // The snapshot converts String -> World, and World implements Audience.
        // Retention here is compatibility, not a missing assignment type proof.
        assert!(catalog().can_convert("java.lang.String", "net.kyori.adventure.audience.Audience"));
        let list = view.at(position(&source, "{_a"), false).unwrap();
        assert!(
            declaration_labels(&list).contains(&"{_aaa}"),
            "{:?}",
            declaration_labels(&list)
        );
        let PreparedQuery::Ready(list) = early.at(position(&source, "{_a"), false).unwrap() else {
            panic!("prepared")
        };
        assert!(
            declaration_labels(&list).contains(&"{_arg_number}"),
            "early facts are not guessed"
        );
    }
}

#[test]
fn native_result_typing_keeps_call_effect_barriers_and_unknown_results() {
    use crate::lsp::semantic::InferredType;
    for (result_type, middle, known) in [
        ("text", "", true),
        ("object", "", false),
        ("text", "    execute console command \"noop\"\n", false),
        ("text", "    set {_aaa} to 1\n", false),
        (
            "text",
            "    if chance of 50%:\n        set {_aaa} to 1\n",
            false,
        ),
    ] {
        let source = format!(
            "function result() :: {result_type}:\n    return \"value\"\nfunction inspect(arg_number: number):\n    set {{_aaa}} to result()\n{middle}    {{_aaa}} matches \"x\"\n"
        );
        let (_, view, _, _) = parse(&source);
        let p = position(&source, "{_aaa");
        let (_, ty) = view.semantic_types.as_ref().unwrap().type_at(p).unwrap();
        assert_eq!(
            matches!(ty, InferredType::Values { classes, .. } if classes.len() == 1 && classes.contains("java.lang.String")),
            known,
            "{result_type} {middle}: {ty:?}"
        );
        // A native call can run arbitrary function code: no old parameter fact
        // is retained merely because its result has a known immutable type.
        let source = source.replace("{_aaa} matches \"x\"", "{_arg_} matches \"x\"");
        let (_, view, _, _) = parse(&source);
        assert!(
            declaration_labels(&view.at(position(&source, "{_arg_"), false).unwrap())
                .contains(&"{_arg_number}")
        );
    }
}

#[test]
fn adopted_context_does_not_survive_rewritten_native_facts_or_an_edit() {
    let source = "on load:\n    send \"test\" to player\n";
    let (early, view, mut result, _) = parse(source);
    let start = source.rfind("player").unwrap();
    let edit = TextRange::new(start, start + 6);
    let line = &view.context[&1];
    assert!(view.expected_at(line, edit, true).is_some());
    assert!(
        Document::from_result(
            &result,
            Arc::from(source.replace("test", "edit")),
            Arc::clone(&view.index)
        )
        .is_err()
    );
    let effect = result
        .ast
        .nodes
        .iter_mut()
        .find(|n| n.kind == parser_wasm::host::SyntaxKind::Effect)
        .unwrap();
    effect.summary.as_mut().unwrap().pattern_index = Some(99);
    let rewritten =
        Document::from_result(&result, Arc::from(source), Arc::clone(&view.index)).unwrap();
    // The altered AST no longer certifies this capture. Current source grammar
    // can independently recover the same expected slot without adopting it.
    assert!(
        !rewritten
            .accepted_slots
            .contains_key(&(edit.start, edit.end))
    );
    let fallback = early
        .document
        .expected_at(&early.document.context[&1], edit, false);
    assert!(fallback.is_some());
    assert_eq!(
        rewritten.expected_at(&rewritten.context[&1], edit, true),
        fallback
    );
}

#[test]
fn revoked_provider_proof_reaches_old_prepared_and_final_documents() {
    use parser_wasm::host::{HostConfig, ParserHost};
    let source = "on load:\n    kill p\n";
    let (mut early, mut view, result, _) = parse(source);
    let rules = catalog();
    let mut host = ParserHost::new(
        crate::core_library_component(),
        HostConfig {
            syntax_catalog: Some(Arc::clone(&rules)),
            runtime_profile: parser_wasm::RuntimeProfile {
                skript_version: Some("2.15.4".into()),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    let mut index = CatalogIndex::with_documentation_format(
        &rules,
        host.raw_tree_options(),
        lsp_types::MarkupKind::PlainText,
    );
    index.certify_types(&host);
    let index = Arc::new(index);
    early.document.index = Arc::clone(&index);
    view.index = Arc::clone(&index);
    let p = position(source, "kill p");
    let is_pi = |list: &CompletionList| list.items.iter().any(|i| i.label == "pi");
    assert!(!is_pi(&view.at(p, false).unwrap()));
    let PreparedQuery::Ready(list) = early.at(p, false).unwrap() else {
        panic!("prepared")
    };
    assert!(!is_pi(&list));
    // Even an unsuccessful provider-set update revokes old certificates. A
    // real disable uses the same gate (tested with a recoverable host fault).
    assert!(host.load_addon(&[]).is_err());
    assert!(is_pi(&view.at(p, false).unwrap()));
    let PreparedQuery::Ready(list) = early.at(p, false).unwrap() else {
        panic!("prepared")
    };
    assert!(is_pi(&list));
    let next = Document::from_result(&result, Arc::from(source), index).unwrap();
    assert!(is_pi(&next.at(p, false).unwrap()));
}

#[test]
fn native_parser_accepts_string_to_world_audience_conversion_in_the_adopted_slot() {
    let source = "function recipient() :: string:\n    return \"world\"\non load:\n    send \"test\" to recipient()\n";
    let (_, view, result, _) = parse(source);
    assert!(
        result.syntax.diagnostics.is_empty(),
        "{:?}",
        result.syntax.diagnostics
    );
    let start = source.rfind("recipient()").unwrap();
    let expected = view
        .accepted_slots
        .get(&(start, start + "recipient()".len()))
        .expect("the native parser adopted the complete function call capture");
    assert_eq!(expected.len(), 1);
    assert_eq!(
        expected[0].class_name.as_str(),
        "net.kyori.adventure.audience.Audience"
    );
    let rules = catalog();
    assert!(rules.can_convert("java.lang.String", "org.bukkit.World"));
    assert!(rules.is_class_assignable("org.bukkit.World", "net.kyori.adventure.audience.Audience"));
    let function = view
        .functions
        .iter()
        .find(|f| f.name == "recipient")
        .unwrap();
    assert_eq!(
        function
            .definition
            .as_ref()
            .unwrap()
            .return_type
            .as_ref()
            .unwrap()
            .as_str(),
        "java.lang.String"
    );
    assert!(!view.incompatible_function(function, Some(expected)));
}

#[test]
fn partially_typed_recipient_filters_its_own_definition_early_and_final() {
    for prefix in ["p", "pl", "pla", "play", "player"] {
        let source = format!(
            "on load:\n    make player invisible\n    send \"a\" to {prefix}\nfunction player_number() :: number:\n    return 1\nfunction player_entity() :: player:\n    return player\nfunction player_unknown() :: object:\n    return player\n"
        );
        let (early, final_view, _, _) = parse(&source);
        let start = source.find(" to ").unwrap() + 4;
        let p = LineIndex::new(&source)
            .position(&source, start + prefix.len())
            .unwrap();
        let edit = TextRange::new(start, start + prefix.len());
        for (view, references) in [(&early.document, false), (&final_view, true)] {
            assert_eq!(
                view.expected_at(&view.context[&p.line], edit, references),
                Some(expected("net.kyori.adventure.audience.Audience", true)),
                "{prefix}"
            );
            let list = view.at_with_references(p, false, references).unwrap();
            let names = labels(&list);
            assert!(!names.contains(&"pi"), "{prefix}: {names:?}");
            assert!(
                !names.iter().any(|n| n.starts_with("product(")
                    || n.starts_with("permutations(")
                    || n.starts_with("player_number(")),
                "{prefix}: {names:?}"
            );
            if references {
                assert!(
                    names.iter().any(|n| n.starts_with("player_entity(")),
                    "{names:?}"
                );
                assert!(
                    names.iter().any(|n| n.starts_with("player_unknown(")),
                    "{names:?}"
                );
            }
        }
    }
}

#[test]
fn expression_return_detail_reaches_preview_without_claiming_cardinality() {
    let source = "on load:\n    send pi\n";
    let (early, view, _, _) = parse(source);
    let p = position(source, "send pi");
    for list in [
        view.at(p, false).unwrap(),
        match early.at(p, false).unwrap() {
            PreparedQuery::Ready(list) => list,
            _ => panic!("prepared"),
        },
    ] {
        let pi = list.items.iter().find(|i| i.label == "pi").unwrap();
        let detail = pi.detail.as_ref().unwrap();
        assert!(detail.contains("return: double"), "{detail}");
        assert!(detail.contains("cardinality: unknown"), "{detail}");
        assert!(
            serde_json::to_string(&pi.documentation)
                .unwrap()
                .contains("Return type: double")
        );
    }
}

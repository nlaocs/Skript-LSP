use super::*;
use crate::{ConditionNodeKind, FunctionArgument, TypeCaptureState};
use std::hint::black_box;
use std::time::Instant;
use syntax_pattern_parser::syntax::{PatternTypeAlternative, PatternTypeExpr, Span};

fn node(children: Vec<ExpressionNode>) -> ExpressionNode {
    let source = MappedSource::identity("x");
    ExpressionNode {
        effects: Some(ExpressionEffects::new("shared effects")),
        kind: ExpressionNodeKind::Custom {
            parser_id: "test.leaf".into(),
        },
        function: None,
        span: MatchSpan {
            local_range: TextRange::new(0, 1),
            mapped: source.map_range(TextRange::new(0, 1)).unwrap(),
        },
        return_type: Some(ClassName("java.lang.Object".into())),
        possible_return_types: vec![ClassName("java.lang.Object".into())],
        possible_return_types_state: PossibleReturnTypesState::Complete,
        multiplicity: Some(Multiplicity::Single),
        captures: Vec::new(),
        tags: Vec::new(),
        mark: 0,
        children,
        routed_captures: Vec::new(),
        public_data: vec![ExpressionPublicData {
            schema_id: "test.capture".into(),
            schema_version: 1,
            json: "{}".into(),
        }],
        metadata: BTreeMap::from([("level".into(), "leaf".into())]),
    }
}

fn expression_type() -> PatternTypeExpr {
    PatternTypeExpr {
        alternatives: vec![PatternTypeAlternative {
            name: "object".into(),
            plural: true,
        }],
        nullable: false,
        allow_literals: true,
        allow_expressions: true,
        time: 0,
    }
}

fn typed(index: usize, resolved: bool) -> PatternCapture {
    PatternCapture::TypeExpression {
        capture_index: index,
        state: TypeCaptureState::Explicit,
        pattern_span: Span::new(0, 9),
        expression: expression_type(),
        value: "x".into(),
        span: node(Vec::new()).span,
        alternative_index: Some(0),
        resolution_id: resolved.then(|| "resolved".into()),
    }
}

fn regex(index: usize) -> PatternCapture {
    PatternCapture::Regex {
        capture_index: index,
        pattern_span: Span::new(0, 3),
        value: "x".into(),
        span: node(Vec::new()).span,
        groups: Vec::new(),
    }
}

fn routed(index: usize, value: Option<ParsedCaptureValue>) -> ParsedCapture {
    let mut result = ParsedCaptureResult::failure("test.result", node(Vec::new()).span, "failed");
    if let Some(value) = value {
        result.status = ParsedCaptureStatus::Partial;
        result.value = Some(value);
    }
    result.attachments.push(ParsedCaptureAttachment {
        owner_component_id: "test.owner".into(),
        schema_id: "test.attachment".into(),
        schema_version: 2,
        encoding: "bytes".into(),
        bytes: vec![0, 255],
    });
    ParsedCapture {
        capture_index: index,
        binding: RegisteredCaptureBinding {
            capture_index: index,
            parser_id: "test.binding".into(),
            required: false,
            options: BTreeMap::from([("option".into(), "preserved".into())]),
        },
        result,
    }
}

// Frozen pre-view implementation: equivalence and timing must not call the new iterator.
fn legacy_captures(node: &ExpressionNode) -> Vec<ParsedCapture> {
    let mut children = node.children.iter();
    if node.captures.is_empty() {
        return children
            .cloned()
            .enumerate()
            .map(|(index, node)| expression_parsed_capture(index, node))
            .collect();
    }
    let mut captures = Vec::new();
    for capture in &node.captures {
        let index = capture.capture_index();
        match capture {
            PatternCapture::TypeExpression {
                resolution_id: Some(_),
                ..
            } => {
                if let Some(node) = children.next() {
                    captures.push(expression_parsed_capture(index, node.clone()));
                }
            }
            PatternCapture::Regex { .. } => {
                if let Some(capture) = node
                    .routed_captures
                    .iter()
                    .find(|c| c.capture_index == index)
                {
                    captures.push(capture.clone());
                }
            }
            PatternCapture::TypeExpression {
                resolution_id: None,
                ..
            } => {}
        }
    }
    captures
}

fn assert_equivalent(node: &ExpressionNode) {
    let old = legacy_captures(node);
    let views = node.parsed_capture_refs().collect::<Vec<_>>();
    assert_eq!(
        views
            .iter()
            .copied()
            .map(ParsedCaptureRef::to_owned)
            .collect::<Vec<_>>(),
        old
    );
    assert_eq!(node.parsed_captures(), old);
    for (view, owned) in views.into_iter().zip(&old) {
        assert_eq!(view.capture_index(), owned.capture_index);
        assert_eq!(view.binding_parser_id(), owned.binding.parser_id);
        assert_eq!(view.span(), &owned.result.span);
        assert_eq!(view.summary().as_deref(), owned.result.summary.as_ref());
    }
}

#[test]
fn capture_views_keep_pattern_order_skips_and_routed_failures() {
    let mut root = node(vec![node(Vec::new()), node(Vec::new()), node(Vec::new())]);
    root.captures = vec![
        regex(9),
        typed(4, true),
        typed(7, false),
        regex(2),
        regex(8),
        typed(0, true),
        regex(2),
    ];
    root.routed_captures = vec![
        routed(2, Some(ParsedCaptureValue::Raw("first".into()))),
        routed(9, None),
        routed(2, Some(ParsedCaptureValue::Raw("duplicate ignored".into()))),
    ];
    assert_equivalent(&root);
    assert_eq!(
        root.parsed_capture_refs()
            .map(ParsedCaptureRef::capture_index)
            .collect::<Vec<_>>(),
        [9, 4, 2, 0, 2]
    );
    let views = root.parsed_capture_refs().collect::<Vec<_>>();
    let ParsedCaptureRef::Expression { node: child, .. } = views[1] else {
        panic!()
    };
    assert!(std::ptr::eq(child, &root.children[0]));
    let ParsedCaptureRef::Routed(failed) = views[0] else {
        panic!()
    };
    assert!(std::ptr::eq(failed, &root.routed_captures[1]));
    assert_eq!(failed.result.status, ParsedCaptureStatus::Failed);
    assert!(views[0].value().is_none());
    // Preserve behavior for an exhausted child list and for implicit children.
    root.children.clear();
    assert_equivalent(&root);
    root.captures.clear();
    assert_equivalent(&root);
    assert_eq!(root.parsed_capture_refs().count(), 0);
}

#[test]
fn capture_views_preserve_grouped_summary_defaults_and_function_values() {
    let mut leaf = node(Vec::new());
    leaf.kind = ExpressionNodeKind::Registered {
        definition_id: "test.definition".into(),
        registration_id: "test.registration".into(),
        element_class: ClassName("test.Element".into()),
        pattern_index: 3,
    };
    let leaf_data = leaf.public_data.clone();
    let mut group = node(vec![leaf]);
    group.kind = ExpressionNodeKind::Grouped;
    group.public_data.clear();
    group.return_type = Some(ClassName("test.GroupType".into()));
    group.metadata.insert("level".into(), "outer".into());
    let mut default = node(Vec::new());
    default.span.local_range = TextRange::new(1, 1);
    default.span.mapped = MappedSource::identity("x")
        .map_range(TextRange::new(1, 1))
        .unwrap();
    default.kind = ExpressionNodeKind::Default {
        info: Box::new(crate::DefaultExpressionInfo {
            capture_index: 4,
            pattern_span: Span::new(0, 9),
            expression: expression_type(),
            expression_source: "%objects%".into(),
            requested_type: ExpressionExpectedType {
                class_name: ClassName("java.lang.Object".into()),
                plural: true,
            },
            type_definition_id: "type.definition".into(),
            type_registration_id: "type.registration".into(),
            provider_id: "test.default".into(),
            component_id: "test.component".into(),
            reason: "omitted".into(),
            event_classes: Vec::new(),
            section_scope_ids: vec![7],
            anchor: default.span.mapped.clone(),
            catalog_references: Vec::new(),
            is_literal: false,
            time: 0,
        }),
    };
    let mut function = node(vec![node(Vec::new())]);
    function.kind = ExpressionNodeKind::Function {
        parser_id: "test.function".into(),
    };
    function.function = Some(FunctionCall {
        name: "f".into(),
        definition_id: "function.definition".into(),
        registration_id: "function.registration".into(),
        arguments: vec![FunctionArgument {
            parameter_name: "value".into(),
            supplied_name: Some("value".into()),
            child_start: 0,
            child_count: 1,
            omitted: false,
        }],
    });
    let root = node(vec![group, default, function]);
    assert_equivalent(&root);
    let views = root.parsed_capture_refs().collect::<Vec<_>>();
    let summary = views[0].summary().unwrap();
    assert_eq!(summary.kind, "registered-expression");
    assert_eq!(summary.public_data, leaf_data);
    assert_eq!(
        summary.return_type.as_ref().unwrap().as_str(),
        "test.GroupType"
    );
    assert_eq!(summary.metadata["level"], "outer");
    assert_eq!(
        views[1]
            .summary()
            .unwrap()
            .default_expression
            .as_ref()
            .unwrap()
            .anchor,
        root.children[1].span.mapped
    );
    let Some(ParsedCaptureValueRef::Expression(call)) = views[2].value() else {
        panic!()
    };
    assert!(std::ptr::eq(call, &root.children[2]));
    assert_eq!(call.function, root.children[2].function);
    assert_equivalent(call);
}

fn condition(expression: ExpressionNode) -> ConditionNode {
    ConditionNode {
        kind: ConditionNodeKind::Grouped,
        span: expression.span.clone(),
        captures: Vec::new(),
        tags: Vec::new(),
        mark: 0,
        marks: Vec::new(),
        expressions: vec![expression],
        children: Vec::new(),
        handler: None,
        metadata: BTreeMap::new(),
    }
}

#[test]
fn routed_capture_views_borrow_condition_subtrees_and_summaries() {
    let mut root = node(Vec::new());
    root.captures = vec![regex(6)];
    root.routed_captures = vec![condition_parsed_capture(
        6,
        condition(node(vec![node(Vec::new())])),
    )];
    assert_equivalent(&root);
    let view = root.parsed_capture_refs().next().unwrap();
    assert!(matches!(view.summary(), Some(Cow::Borrowed(_))));
    let Some(ParsedCaptureValueRef::Condition(child)) = view.value() else {
        panic!()
    };
    let Some(ParsedCaptureValue::Condition(stored)) = &root.routed_captures[0].result.value else {
        panic!()
    };
    assert!(std::ptr::eq(child, stored));
    assert_equivalent(&child.expressions[0]);
}

fn chain(depth: usize) -> ExpressionNode {
    let mut tree = node(Vec::new());
    for _ in 1..depth {
        tree = node(vec![tree]);
    }
    tree
}

fn visit_value(value: ParsedCaptureValueRef<'_>, borrowed: bool) -> usize {
    match value {
        ParsedCaptureValueRef::Expression(child) => walk(child, borrowed),
        ParsedCaptureValueRef::Condition(child) => child
            .expressions
            .iter()
            .map(|child| walk(child, borrowed))
            .sum(),
        _ => 0,
    }
}

fn walk(node: &ExpressionNode, borrowed: bool) -> usize {
    let descendants: usize = if borrowed {
        node.parsed_capture_refs()
            .filter_map(ParsedCaptureRef::value)
            .map(|value| visit_value(value, true))
            .sum()
    } else {
        legacy_captures(node)
            .iter()
            .filter_map(|capture| capture.result.value.as_ref())
            .map(|value| visit_value(value.into(), false))
            .sum()
    };
    descendants + node.span.local_range.end + node.metadata.len()
}

#[test]
#[ignore = "manual traversal microbenchmark; run alone with --nocapture --test-threads=1"]
fn measure_capture_traversal() {
    let mut routed_tree = chain(32);
    routed_tree.captures = vec![regex(0)];
    routed_tree.children.clear();
    routed_tree.routed_captures = vec![condition_parsed_capture(0, condition(chain(32)))];
    for (shape, tree) in [
        ("chain-8", chain(8)),
        ("chain-32", chain(32)),
        ("chain-128", chain(128)),
        (
            "wide-128",
            node((0..128).map(|_| node(Vec::new())).collect()),
        ),
        ("routed-condition-32", routed_tree),
    ] {
        assert_eq!(walk(&tree, false), walk(&tree, true));
        for _ in 0..5 {
            black_box(walk(black_box(&tree), false));
            black_box(walk(black_box(&tree), true));
        }
        for sample in 0..15 {
            for borrowed in if sample % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let repetitions = 50;
                let start = Instant::now();
                let mut checksum = 0;
                for _ in 0..repetitions {
                    checksum += black_box(walk(black_box(&tree), borrowed));
                }
                println!(
                    "capture-view,{shape},{sample},{},{repetitions},{},{}",
                    if borrowed { "borrowed" } else { "legacy-owned" },
                    start.elapsed().as_nanos(),
                    black_box(checksum)
                );
            }
        }
    }
}

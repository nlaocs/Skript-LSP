use super::*;
use crate::{ExpressionNodeKind, TypeCaptureState};
use syntax_pattern_parser::syntax::{PatternTypeAlternative, PatternTypeExpr, Span};
use syntaxes::{ClassName, Multiplicity, PossibleReturnTypesState};

fn expression() -> ExpressionNode {
    let source = MappedSource::identity("x");
    ExpressionNode {
        effects: None,
        kind: ExpressionNodeKind::Literal {
            parser_id: "test.literal".into(),
        },
        function: None,
        span: MatchSpan {
            local_range: TextRange::new(0, 1),
            mapped: source.map_range(TextRange::new(0, 1)).unwrap(),
        },
        return_type: Some(ClassName("java.lang.String".into())),
        possible_return_types: vec![ClassName("java.lang.String".into())],
        possible_return_types_state: PossibleReturnTypesState::Complete,
        multiplicity: Some(Multiplicity::Single),
        captures: Vec::new(),
        tags: Vec::new(),
        mark: 0,
        children: Vec::new(),
        routed_captures: Vec::new(),
        public_data: Vec::new(),
        metadata: BTreeMap::new(),
    }
}

fn typed(index: usize, id: Option<&str>, state: TypeCaptureState) -> PatternCapture {
    PatternCapture::TypeExpression {
        capture_index: index,
        state,
        omission: None,
        pattern_span: Span::new(0, 8),
        expression: PatternTypeExpr {
            alternatives: vec![PatternTypeAlternative {
                name: "string".into(),
                plural: false,
            }],
            nullable: state == TypeCaptureState::Null,
            allow_literals: true,
            allow_expressions: true,
            time: 0,
        },
        value: "x".into(),
        span: expression().span,
        alternative_index: Some(0),
        resolution_id: id.map(str::to_owned),
    }
}

fn regex(index: usize) -> PatternCapture {
    PatternCapture::Regex {
        capture_index: index,
        pattern_span: Span::new(0, 3),
        value: "x".into(),
        span: expression().span,
        groups: Vec::new(),
    }
}

fn node(
    captures: Vec<PatternCapture>,
    expressions: Vec<ExpressionNode>,
    expression_bindings: Vec<ConditionExpressionBinding>,
) -> ConditionNode {
    ConditionNode {
        kind: ConditionNodeKind::Grouped,
        span: expression().span,
        captures,
        expressions,
        expression_bindings,
        tags: Vec::new(),
        mark: 0,
        marks: Vec::new(),
        children: Vec::new(),
        handler: None,
        metadata: BTreeMap::new(),
    }
}

#[test]
fn missing_resolutions_regex_and_null_slots_never_shift_later_bindings() {
    let leaf = expression();
    let captures = vec![
        regex(0),
        typed(3, Some("missing"), TypeCaptureState::Explicit),
        typed(5, None, TypeCaptureState::Null),
        typed(7, Some("valid"), TypeCaptureState::Explicit),
        typed(9, Some("valid"), TypeCaptureState::Default),
    ];
    let (expressions, bindings) =
        resolved_condition_expressions(&captures, |id| (id == "valid").then_some(&leaf));
    assert_eq!(
        bindings,
        [
            ConditionExpressionBinding {
                origin: ConditionExpressionBindingOrigin::TypeResolution,
                capture_index: 7,
                expression_index: 0
            },
            ConditionExpressionBinding {
                origin: ConditionExpressionBindingOrigin::TypeResolution,
                capture_index: 9,
                expression_index: 1
            }
        ]
    );
    let node = node(captures, expressions, bindings);
    let views = node.parsed_capture_refs().collect::<Vec<_>>();
    assert_eq!(
        views
            .iter()
            .map(|view| view.capture_index())
            .collect::<Vec<_>>(),
        [7, 9]
    );
    for (index, view) in views.into_iter().enumerate() {
        let ParsedCaptureRef::Expression { node: borrowed, .. } = view else {
            panic!()
        };
        assert!(std::ptr::eq(borrowed, &node.expressions[index]));
        assert_eq!(view.to_owned().capture_index, [7, 9][index]);
        assert_eq!(view.binding_parser_id(), crate::HOST_EXPRESSION_PARSER_ID);
    }
}

#[test]
fn malformed_or_absent_associations_make_no_claim_despite_equal_spans() {
    let base = node(
        vec![
            regex(0),
            typed(4, Some("valid"), TypeCaptureState::Explicit),
        ],
        vec![expression(), expression()],
        vec![ConditionExpressionBinding {
                origin: ConditionExpressionBindingOrigin::TypeResolution,
            capture_index: 4,
            expression_index: 0,
        }],
    );
    assert_eq!(base.parsed_capture_refs().count(), 1);
    assert_eq!(base.expression_capture_index(1), None);
    for binding in [
        ConditionExpressionBinding {
                origin: ConditionExpressionBindingOrigin::TypeResolution,
            capture_index: 4,
            expression_index: 1,
        },
        ConditionExpressionBinding {
                origin: ConditionExpressionBindingOrigin::TypeResolution,
            capture_index: 0,
            expression_index: 0,
        },
        ConditionExpressionBinding {
                origin: ConditionExpressionBindingOrigin::TypeResolution,
            capture_index: 99,
            expression_index: 0,
        },
        ConditionExpressionBinding {
                origin: ConditionExpressionBindingOrigin::TypeResolution,
            capture_index: 4,
            expression_index: 99,
        },
    ] {
        let mut invalid = base.clone();
        invalid.expression_bindings.push(binding);
        assert_eq!(invalid.parsed_capture_refs().count(), 0, "{binding:?}");
        assert_eq!(
            invalid.expressions.len(),
            2,
            "unbound children remain intact"
        );
    }
    let mut duplicate_capture = base.clone();
    duplicate_capture
        .captures
        .push(typed(4, Some("other"), TypeCaptureState::Explicit));
    assert_eq!(duplicate_capture.parsed_capture_refs().count(), 0);
    let no_resolution = node(
        vec![typed(4, None, TypeCaptureState::Null)],
        base.expressions,
        base.expression_bindings,
    );
    assert_eq!(no_resolution.parsed_capture_refs().count(), 0);
}

#[test]
fn recursive_source_mapping_preserves_explicit_associations_and_groups() {
    let inner = node(
        vec![typed(7, Some("valid"), TypeCaptureState::Explicit)],
        vec![expression()],
        vec![ConditionExpressionBinding {
                origin: ConditionExpressionBindingOrigin::TypeResolution,
            capture_index: 7,
            expression_index: 0,
        }],
    );
    let grouped = grouped_candidate(ConditionCandidate { node: inner }, expression().span).node;
    assert!(grouped.expression_bindings.is_empty());
    let mut outer = expression();
    outer
        .routed_captures
        .push(crate::condition_parsed_capture(2, grouped));
    outer
        .try_map_spans(&mut |span| -> Result<_, ()> {
            let mut mapped = span.clone();
            mapped.local_range.start += 10;
            mapped.local_range.end += 10;
            mapped.mapped.virtual_range.start += 10;
            mapped.mapped.virtual_range.end += 10;
            Ok(mapped)
        })
        .unwrap();
    let Some(crate::ParsedCaptureValue::Condition(grouped)) =
        &outer.routed_captures[0].result.value
    else {
        panic!()
    };
    let inner = &grouped.children[0];
    assert_eq!(inner.expression_capture_index(0), Some(7));
    assert_eq!(
        inner.expressions[0].span.local_range,
        TextRange::new(10, 11)
    );
    assert_eq!(inner.captures[0].capture_index(), 7);
    assert_eq!(
        inner.span.local_range,
        inner.expressions[0].span.local_range
    );
}

#[test]
fn native_regex_origin_is_explicit_and_cannot_relabel_typed_or_duplicate_slots() {
    let base = node(
        vec![regex(0), typed(4, Some("valid"), TypeCaptureState::Explicit)],
        vec![expression(), expression()],
        vec![
            ConditionExpressionBinding {
                capture_index: 0, expression_index: 0,
                origin: ConditionExpressionBindingOrigin::NativeRegex,
            },
            ConditionExpressionBinding {
                capture_index: 4, expression_index: 1,
                origin: ConditionExpressionBindingOrigin::TypeResolution,
            },
        ],
    );
    assert_eq!(base.parsed_capture_refs().map(|v|v.capture_index()).collect::<Vec<_>>(), [0,4]);
    for index in [0,1] {
        let mut wrong = base.clone();
        wrong.expression_bindings[index].origin = if index==0 {
            ConditionExpressionBindingOrigin::TypeResolution
        } else { ConditionExpressionBindingOrigin::NativeRegex };
        assert_eq!(wrong.expression_capture_index(index), None);
        assert_eq!(wrong.parsed_capture_refs().count(), 1);
    }
    let mut duplicate = base.clone();
    duplicate.expression_bindings.push(base.expression_bindings[0]);
    assert_eq!(duplicate.expression_capture_index(0), None);
    assert_eq!(duplicate.parsed_capture_refs().count(), 1);
}

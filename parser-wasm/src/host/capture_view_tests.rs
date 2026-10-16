use super::*;
use skript_parser::{
    CandidateMatch, EffectCandidate, EventCandidate, ExpressionExpectedType, FunctionArgument,
    FunctionCall, ParsedCaptureAttachment, ParsedCaptureResult, ParsedCaptureValue, PatternMatch,
    SectionCandidate, TypeCaptureState,
};
use std::path::Path;
use syntax_pattern_parser::syntax::{PatternElement, Span};

pub(super) fn fixture() -> (MappedSource, ExpressionNode, Vec<ParserParsedCapture>) {
    let catalog = ssg::load(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/type-parser-versions/skript-2.16.0"),
    )
    .unwrap()
    .into_catalog();
    let input = "dummy direct registry expression";
    let source = MappedSource::identity(input);
    let leaf = skript_parser::parse_expression(
        &catalog,
        ExpressionParseRequest {
            source: &source,
            range: ParserTextRange::new(0, input.len()),
            expected_types: Vec::new(),
            context: ExpressionParseContext::default(),
        },
        &mut skript_parser::NoopExpressionEnvironment,
        ExpressionParserConfig::default(),
    )
    .unwrap()
    .selected
    .unwrap()
    .node;
    let parsed = syntax_pattern_parser::syntax::parse("%objects%", catalog.plural_rules()).unwrap();
    let PatternElement::TypeExpr(expression) = &parsed.elements[0].value else {
        panic!()
    };
    let mut default = leaf.clone();
    default.span.local_range = ParserTextRange::empty(input.len());
    default.span.mapped = source.map_range(default.span.local_range).unwrap();
    default.kind = ExpressionNodeKind::Default {
        info: Box::new(skript_parser::DefaultExpressionInfo {
            capture_index: 2,
            pattern_span: Span::new(0, 9),
            expression: expression.clone(),
            expression_source: "%objects%".into(),
            requested_type: ExpressionExpectedType {
                class_name: ClassName("java.lang.Object".into()),
                plural: true,
            },
            type_definition_id: "type.test".into(),
            type_registration_id: "type.test.0".into(),
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
    let mut group = leaf.clone();
    group.kind = ExpressionNodeKind::Grouped;
    group.children = vec![leaf.clone()];
    let mut function = leaf.clone();
    function.kind = ExpressionNodeKind::Function {
        parser_id: "test.function".into(),
    };
    function.children = vec![group];
    function.function = Some(FunctionCall {
        name: "f".into(),
        definition_id: "test.function".into(),
        registration_id: "test.function.0".into(),
        arguments: vec![FunctionArgument {
            parameter_name: "value".into(),
            supplied_name: Some("value".into()),
            child_start: 0,
            child_count: 1,
            omitted: false,
        }],
    });
    let mut root = leaf.clone();
    root.children = vec![function, default];
    root.captures = [7, 2]
        .into_iter()
        .map(|capture_index| PatternCapture::TypeExpression {
            capture_index,
            state: TypeCaptureState::Explicit,
            pattern_span: Span::new(0, 9),
            expression: expression.clone(),
            value: input.into(),
            span: leaf.span.clone(),
            alternative_index: Some(0),
            resolution_id: Some("resolved".into()),
        })
        .collect();
    let matched = |kind| CandidateMatch {
        extension_state: None,
        kind,
        definition_id: "test.definition".into(),
        registration_id: "test.registration".into(),
        priority: 0,
        registration_order: 0,
        literal_anchor: true,
        pattern_index: 0,
        pattern: input.into(),
        matched: PatternMatch {
            span: leaf.span.clone(),
            captures: Vec::new(),
            tags: Vec::new(),
            mark: 0,
            marks: Vec::new(),
            recovered_failures: Vec::new(),
        },
    };
    let nested = root.parsed_captures();
    let condition = ConditionNode {
        kind: ConditionNodeKind::Grouped,
        span: leaf.span.clone(),
        captures: Vec::new(),
        tags: Vec::new(),
        mark: 0,
        marks: Vec::new(),
        expressions: vec![root.clone()],
        children: Vec::new(),
        handler: None,
        metadata: BTreeMap::new(),
    };
    let effect = EffectCandidate {
        raw_node_id: ParserRawNodeId::new(1),
        matched: matched(MatchSyntaxKind::Effect),
        parsed_captures: nested.clone(),
        handler: None,
        metadata: BTreeMap::new(),
    };
    let event = EventCandidate {
        matched: matched(MatchSyntaxKind::Event),
        span: leaf.span.clone(),
        element_class: None,
        reference_events: Vec::new(),
        cancellable: Some(true),
        priority_supported: None,
        handler: None,
        metadata: BTreeMap::new(),
    };
    let section = SectionCandidate {
        raw_node_id: ParserRawNodeId::new(2),
        matched: matched(MatchSyntaxKind::Section),
        element_class: None,
        addon_name: None,
        addon_version: None,
        owner_component_id: None,
        parsed_captures: nested,
        loop_section: false,
        effect_section: false,
        section_expression: false,
        body_mode: skript_parser::SectionBodyMode::default(),
        body: Vec::new(),
        handler: None,
        metadata: BTreeMap::new(),
        body_context: ExpressionParseContext::default(),
    };
    let captures = [
        Some(ParsedCaptureValue::Raw("raw text".into())),
        None,
        Some(ParsedCaptureValue::Condition(condition)),
        Some(ParsedCaptureValue::Effect(Box::new(effect))),
        Some(ParsedCaptureValue::Event(Box::new(event))),
        Some(ParsedCaptureValue::Section(Box::new(section))),
    ]
    .into_iter()
    .zip([9, 3, 8, 1, 4, 6])
    .map(|(value, capture_index)| ParserParsedCapture {
        capture_index,
        binding: RegisteredCaptureBinding {
            capture_index,
            parser_id: format!("binding.{capture_index}"),
            required: false,
            options: BTreeMap::new(),
        },
        result: ParsedCaptureResult {
            parser_id: format!("result.{capture_index}"),
            status: if value.is_some() {
                ParserParsedCaptureStatus::Partial
            } else {
                ParserParsedCaptureStatus::Failed
            },
            span: leaf.span.clone(),
            summary: None,
            value,
            diagnostics: Vec::new(),
            attachments: vec![ParsedCaptureAttachment {
                owner_component_id: "test.owner".into(),
                schema_id: "test.bytes".into(),
                schema_version: 2,
                encoding: "raw".into(),
                bytes: vec![0, 255],
            }],
        },
    })
    .collect();
    (source, root, captures)
}

#[test]
fn capture_view_parse_result_matches_owned_capture_projection() {
    let (source, root, captures) = fixture();
    let request = ParseRequest {
        request_id: 1,
        parser_id: skript_parser::HOST_EXPRESSION_PARSER_ID.into(),
        input: source.virtual_source().into(),
        expected_types: Vec::new(),
        span: mapped_span_to_wit(root.span.mapped.clone()),
        options: Vec::new(),
    };
    let actual = expression_parse_result(&request, &root, None);
    let mut legacy = ParseResultArena::new(&request, None);
    let children = root
        .parsed_captures()
        .iter()
        .filter_map(|capture| legacy.push_capture(capture))
        .collect::<Vec<_>>();
    assert_eq!(actual.nodes.last().unwrap().children, children);
    assert_eq!(
        format!("{:?}", &actual.nodes[..actual.nodes.len() - 1]),
        format!("{:?}", legacy.nodes)
    );
    assert_eq!(actual.nodes.len(), 5);
    // The final projection still omits FunctionCall argument binding, as before.
    assert_eq!(
        root.children[0].function.as_ref().unwrap().arguments[0].child_count,
        1
    );
    let mut borrowed = ParseResultArena::new(&request, None);
    let mut legacy = ParseResultArena::new(&request, None);
    for capture in &captures {
        let id = borrowed.push_capture_ref(capture.into());
        assert_eq!(id, legacy.push_capture(capture));
        assert_eq!(
            format!("{:?}", borrowed.nodes),
            format!("{:?}", legacy.nodes)
        );
        if capture.result.value.is_none() {
            assert!(id.is_none());
        } else {
            let projected = &borrowed.nodes[id.unwrap() as usize];
            assert_eq!(projected.parser_id, capture.result.parser_id);
            assert_eq!(projected.attachments.len(), 1);
            assert_eq!(projected.attachments[0].bytes, [0, 255]);
        }
    }
}

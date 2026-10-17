use super::*;
use std::path::Path;

fn host() -> ParserHost {
    let snapshot = ssg::load(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../syntax-pattern-parser/tests/data/corpus/multi-addon-2.15.4"),
    )
    .unwrap();
    ParserHost::new(
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../artifacts/core-library.wasm"
        )),
        HostConfig {
            syntax_catalog: Some(Arc::new(snapshot.into_catalog())),
            ..HostConfig::default()
        },
    )
    .unwrap()
}

fn context(revision: u64) -> InvocationContext {
    InvocationContext {
        invocation_id: revision,
        subscription_id: String::new(),
        document_id: "file:///function.sk".into(),
        document_revision: revision,
        expansion: None,
        syntax_context: 0,
    }
}

fn request(input: &str) -> ParseRequest {
    ParseRequest {
        request_id: 1,
        parser_id: skript_parser::HOST_EXPRESSION_PARSER_ID.into(),
        input: input.into(),
        expected_types: Vec::new(),
        span: mapped_span_to_wit(
            MappedSource::identity(input)
                .map_range(ParserTextRange::new(0, input.len()))
                .unwrap(),
        ),
        options: Vec::new(),
    }
}

fn assert_call(native: &ExpressionNode, projected: Option<&WitFunctionCall>) {
    assert_eq!(
        native.function,
        projected.map(|call| function_call_from_wit(call, native.children.len()).unwrap())
    );
}

fn assert_ast(native: &ExpressionNode, id: u64, tree: &AstTree) {
    let node = tree.nodes.iter().find(|node| node.id == id).unwrap();
    assert_call(native, node.function.as_ref());
    assert_eq!(node.children.len(), native.children.len());
    assert!(same_mapped_span(
        &node.span,
        &mapped_span_to_wit(native.span.mapped.clone())
    ));
    assert_eq!(
        node.summary.as_ref().unwrap().return_type.as_deref(),
        native.return_type.as_ref().map(ClassName::as_str)
    );
    for (child, id) in native.children.iter().zip(&node.children) {
        assert_ast(child, *id, tree);
    }
}

fn assert_result(native: &ExpressionNode, id: u64, result: &ParseResult, request: &ParseRequest) {
    let node = result.nodes.iter().find(|node| node.node_id == id).unwrap();
    assert_call(native, node.function.as_ref());
    assert_eq!(node.children.len(), native.children.len());
    assert!(same_mapped_span(
        &node.span,
        &nested_span_to_request(&native.span, request)
    ));
    assert_eq!(
        node.text,
        native
            .span
            .mapped
            .virtual_range
            .slice(&request.input)
            .unwrap()
    );
    assert_eq!(
        node.summary.as_ref().unwrap().return_type.as_deref(),
        native.return_type.as_ref().map(ClassName::as_str)
    );
    for (child, id) in native.children.iter().zip(&node.children) {
        assert_result(child, *id, result, request);
    }
}

#[test]
fn native_function_bindings_reach_both_arenas_in_declaration_order() {
    let mut host = host();
    for (index, input) in [
        "sin(abs(-1))",
        "log(base: 2, n: abs(-8))",
        "log(8)",
        "sum(1, 2)",
        "clamp((1,2), 3, 4)",
        "(log(base: 2, n: 8))",
        "concat(\"日本語\", \"x\")",
    ]
    .into_iter()
    .enumerate()
    {
        let revision = index as u64 + 1;
        let transaction = host
            .begin_parse("file:///workspace", "file:///function.sk", revision)
            .unwrap();
        let source = MappedSource::identity(input);
        let parsed = host
            .parse_expression_in_parse(
                &transaction,
                context(revision),
                ExpressionParseRequest {
                    source: &source,
                    range: ParserTextRange::new(0, input.len()),
                    expected_types: Vec::new(),
                    context: ExpressionParseContext::default(),
                },
                ExpressionParserConfig::default(),
            )
            .unwrap();
        let native = parsed
            .matches
            .selected
            .unwrap_or_else(|| panic!("{input} must parse"))
            .node;
        let tree = ast_tree::expression_for_test(&source, &native);
        assert_ast(&native, tree.roots[0], &tree);
        let expanded = host
            .expand_ast_in_parse(
                &transaction,
                AstMacroRequest {
                    context: context(revision),
                    source: source.clone(),
                    tree,
                },
            )
            .unwrap();
        assert_ast(&native, expanded.tree.roots[0], &expanded.tree);
        let request = request(input);
        let result =
            expression_parse_result(&request, &native, host.config.syntax_catalog.as_deref());
        validate_parse_result(&request, &result).unwrap();
        assert_result(&native, result.roots[0], &result, &request);
        if input.starts_with("log(base:") {
            let call = result.nodes[result.roots[0] as usize]
                .function
                .as_ref()
                .unwrap();
            assert_eq!(
                call.arguments
                    .iter()
                    .map(|argument| argument.parameter_name.as_str())
                    .collect::<Vec<_>>(),
                ["n", "base"]
            );
            assert_eq!(call.arguments[0].supplied_name.as_deref(), Some("n"));
            assert_eq!(call.arguments[1].supplied_name.as_deref(), Some("base"));
            let children = &result.nodes[result.roots[0] as usize].children;
            assert_eq!(result.nodes[children[0] as usize].text, "abs(-8)");
            assert_eq!(result.nodes[children[1] as usize].text, "2");
        }
        transaction.cancel().unwrap();
    }
}

#[test]
fn additional_parse_preserves_nested_bindings_and_rebased_spans() {
    let mut host = host();
    let transaction = host
        .begin_parse("file:///workspace", "file:///function.sk", 1)
        .unwrap();
    let mut request = request("log(base: 2, n: abs(-8))");
    request.span.virtual_range.start += 40;
    request.span.virtual_range.end += 40;
    request.span.origins[0].original_range.start += 100;
    request.span.origins[0].original_range.end += 100;
    let mut results = host
        .execute_parse_requests(&transaction, &context(1), None, vec![request.clone()])
        .unwrap();
    let result = results.pop().unwrap();
    let native = &result.expression_roots[&result.wire.roots[0]];
    let root = &result.wire.nodes[result.wire.roots[0] as usize];
    assert!(root.function.is_some());
    assert_eq!(root.span.virtual_range.start, 40);
    assert_eq!(root.span.origins[0].original_range.start, 100);
    assert_eq!(
        native.function.as_ref().unwrap().arguments[0].parameter_name,
        "n"
    );
    for node in &result.wire.nodes {
        assert!(node.span.virtual_range.start >= 40);
        if node.kind == "function" {
            function_call_from_wit(node.function.as_ref().unwrap(), node.children.len()).unwrap();
        }
    }
    transaction.cancel().unwrap();
}

#[test]
fn parse_result_function_validation_preserves_unresolved_statuses() {
    let (source, root, _) = capture_view_tests::fixture();
    let request = request(source.virtual_source());
    let valid = expression_parse_result(&request, &root.children[0], None);
    validate_parse_result(&request, &valid).unwrap();
    let id = valid.roots[0] as usize;
    let mut unresolved = valid.clone();
    unresolved.nodes[id].function = None;
    assert!(validate_parse_result(&request, &unresolved).is_err());
    for status in [WitParseResultStatus::Partial, WitParseResultStatus::Failed] {
        unresolved.nodes[id].status = status;
        validate_parse_result(&request, &unresolved).unwrap();
    }
    let mut non_function = valid.clone();
    non_function.nodes[id].kind = "custom".into();
    assert!(validate_parse_result(&request, &non_function).is_err());
    for mutation in 0..7 {
        let mut invalid = valid.clone();
        let call = invalid.nodes[id].function.as_mut().unwrap();
        match mutation {
            0 => call.name.clear(),
            1 => call.arguments[0].parameter_name.clear(),
            2 => call.arguments.push(call.arguments[0].clone()),
            3 => call.arguments[0].child_start = 1,
            4 => call.arguments[0].child_count = 2,
            5 => call.arguments[0].omitted = true,
            6 => {
                call.arguments[0].child_start = u64::MAX;
                call.arguments[0].child_count = 1;
            }
            _ => unreachable!(),
        }
        assert!(
            validate_parse_result(&request, &invalid).is_err(),
            "mutation {mutation}"
        );
    }
    let mut broad_request = request.clone();
    broad_request.span.virtual_range.end += 10;
    let mut outside = valid;
    let child = outside.nodes[id].children[0] as usize;
    outside.nodes[child].span.virtual_range.end += 1;
    validate_parse_result(&broad_request, &outside).unwrap();
}

#[test]
fn function_binding_bytes_are_counted_in_both_output_quotas() {
    let (source, root, _) = capture_view_tests::fixture();
    let tree = ast_tree::expression_for_test(&source, &root.children[0]);
    let mut node = tree.nodes[tree.roots[0] as usize].clone();
    let function_bytes = function_call_size(node.function.as_ref().unwrap());
    let with = ast_node_size(&node);
    node.function = None;
    assert_eq!(with - ast_node_size(&node), function_bytes);
    let result =
        expression_parse_result(&request(source.virtual_source()), &root.children[0], None);
    let mut node = result.nodes[result.roots[0] as usize].clone();
    let with = parse_result_node_size(&node);
    node.function = None;
    assert_eq!(with - parse_result_node_size(&node), function_bytes);
}

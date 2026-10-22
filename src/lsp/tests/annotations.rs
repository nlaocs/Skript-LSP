use super::*;

fn no_responses(client: &Client, barrier: i32, ids: &[i32]) {
    client
        .connection
        .sender
        .send(
            lsp_server::Request::new(barrier.into(), "fixture/barrier".into(), Value::Null).into(),
        )
        .unwrap();
    loop {
        if let Message::Response(response) =
            client.connection.receiver.recv_timeout(TIMEOUT).unwrap()
        {
            if response.id == barrier.into() {
                break;
            }
            assert!(
                !ids.iter().any(|id| response.id == (*id).into()),
                "unexpected or duplicate response {response:?}"
            );
        }
    }
}

fn request(client: &Client, id: i32, method: &str, character: u32) {
    let mut params = json!({"textDocument":{"uri":fixture_uri("example.sk")},"position":{"line":0,"character":character}});
    if method == "textDocument/references" {
        params["context"] = json!({"includeDeclaration":true});
    }
    if method == "textDocument/inlayHint" {
        params["range"] =
            json!({"start":{"line":0,"character":0},"end":{"line":0,"character":character}});
    }
    client
        .connection
        .sender
        .send(lsp_server::Request::new(id.into(), method.into(), params).into())
        .unwrap();
}

#[test]
fn completion_documentation_format_is_independent_of_hover_preference() {
    for (formats, expected) in [
        (json!(["markdown", "plaintext"]), MarkupKind::Markdown),
        (json!(["plaintext", "markdown"]), MarkupKind::PlainText),
        (json!([]), MarkupKind::PlainText),
    ] {
        let params = serde_json::from_value(json!({"rootUri":fixture_project_uri(),"capabilities":{"textDocument":{"hover":{"contentFormat":["plaintext"]},"completion":{"completionItem":{"documentationFormat":formats}}}},"initializationOptions":{"snapshotPath":"snapshot"}})).unwrap();
        let config = ProjectConfig::from_params(params).unwrap();
        assert_eq!(config.completion_format, expected);
        assert_eq!(config.hover_format, MarkupKind::PlainText);
    }
}

#[test]
fn semantic_requests_wait_and_cancel_once_when_source_changes() {
    let client = Client::start();
    client.open(1, "hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    request(&client, 40, "textDocument/definition", 0);
    request(&client, 41, "textDocument/references", 0);
    request(&client, 42, "textDocument/inlayHint", 4);
    no_responses(&client, 43, &[40, 41, 42]);
    client.notify("$/cancelRequest", json!({"id":41}));
    assert_eq!(
        token_response(&client, 41)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::RequestCanceled as i32
    );
    client.change(2, "fresh");
    let mut expected = vec![RequestId::from(40), RequestId::from(42)];
    while !expected.is_empty() {
        let message = client.connection.receiver.recv_timeout(TIMEOUT).unwrap();
        if let Message::Response(response) = message {
            let index = expected
                .iter()
                .position(|id| *id == response.id)
                .expect("unexpected or duplicate semantic cancellation response");
            expected.remove(index);
            assert_eq!(
                response.response_result.unwrap_err().code,
                ErrorCode::ContentModified as i32
            );
        }
    }
    client.proceed.send(()).unwrap();
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 2);
    loop {
        let message = client.connection.receiver.recv_timeout(TIMEOUT).unwrap();
        if let Message::Response(response) = &message {
            assert!(
                ![40, 41, 42]
                    .iter()
                    .any(|id| response.id == RequestId::from(*id)),
                "duplicate semantic response after obsolete worker finished"
            );
        }
        if let Message::Notification(notification) = message {
            if notification.method == "textDocument/publishDiagnostics" {
                assert_eq!(
                    notification.params["version"], 2,
                    "obsolete result published"
                );
                break;
            }
        }
    }
    no_responses(&client, 44, &[40, 41, 42]);
    request(&client, 45, "textDocument/definition", 0);
    assert_eq!(
        token_response(&client, 45).response_result.unwrap(),
        json!([])
    );
    client.finish();
}

#[test]
fn semantic_requests_validate_surrogates_and_reversed_ranges_before_waiting() {
    let client = Client::start();
    client.open(1, "hold🙂");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    for (id, method) in [
        (40, "textDocument/definition"),
        (41, "textDocument/references"),
        (42, "textDocument/inlayHint"),
    ] {
        request(&client, id, method, 5);
        assert_eq!(
            token_response(&client, id)
                .response_result
                .unwrap_err()
                .code,
            ErrorCode::InvalidParams as i32
        );
    }
    client.connection.sender.send(lsp_server::Request::new(43.into(),"textDocument/inlayHint".into(),json!({"textDocument":{"uri":fixture_uri("example.sk")},"range":{"start":{"line":0,"character":4},"end":{"line":0,"character":2}}})).into()).unwrap();
    assert_eq!(
        token_response(&client, 43)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::InvalidParams as i32
    );
    client.proceed.send(()).unwrap();
    client.diagnostics(1);
    client.finish();
}

#[test]
fn semantic_pending_requests_are_bounded_and_shutdown_cancels_them() {
    let client = Client::start();
    client.open(1, "hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    for id in 100..165 {
        request(&client, id, "textDocument/definition", 0);
    }
    assert_eq!(
        token_response(&client, 164)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::RequestFailed as i32
    );
    client
        .connection
        .sender
        .send(lsp_server::Request::new(99.into(), "shutdown".into(), Value::Null).into())
        .unwrap();
    for id in 100..164 {
        assert_eq!(
            token_response(&client, id)
                .response_result
                .unwrap_err()
                .code,
            ErrorCode::RequestCanceled as i32
        );
    }
    assert_eq!(
        token_response(&client, 99).response_result.unwrap(),
        Value::Null
    );
    client.proceed.send(()).unwrap();
    client.notify("exit", Value::Null);
    assert!(client.server.join().unwrap().is_ok());
}

#[test]
fn unopened_semantic_requests_have_empty_results() {
    let client = Client::start();
    for (id, method) in [
        (40, "textDocument/definition"),
        (41, "textDocument/references"),
        (42, "textDocument/inlayHint"),
    ] {
        request(&client, id, method, 0);
        assert_eq!(
            token_response(&client, id).response_result.unwrap(),
            json!([])
        );
    }
    client.finish();
}

#[test]
fn rename_requires_versioned_edits_and_refuses_pending_current_analysis() {
    for (capabilities, expected) in [
        (json!({}), ErrorCode::RequestFailed),
        (
            json!({"workspace":{"workspaceEdit":{"documentChanges":true}}}),
            ErrorCode::ContentModified,
        ),
    ] {
        let client = Client::start_with_capabilities(capabilities);
        client.open(1, "hold");
        assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
        client.connection.sender.send(lsp_server::Request::new(50.into(), "textDocument/rename".into(),
            json!({"textDocument":{"uri":fixture_uri("example.sk")},"position":{"line":0,"character":0},"newName":"renamed"})).into()).unwrap();
        assert_eq!(
            token_response(&client, 50)
                .response_result
                .unwrap_err()
                .code,
            expected as i32
        );
        client.proceed.send(()).unwrap();
        client.diagnostics(1);
        no_responses(&client, 51, &[50]);
        client.finish();
    }
}

#[test]
fn rename_negotiates_document_changes_without_versionless_fallback() {
    for (edit, expected) in [
        (json!({}), false),
        (json!({"documentChanges":false}), false),
        (json!({"documentChanges":true}), true),
    ] {
        let params = serde_json::from_value(json!({
            "rootUri":fixture_project_uri(),
            "capabilities":{"workspace":{"workspaceEdit":edit}},
            "initializationOptions":{"snapshotPath":"snapshot"}
        }))
        .unwrap();
        assert_eq!(
            ProjectConfig::from_params(params)
                .unwrap()
                .rename_document_changes,
            expected
        );
    }
}

#[test]
fn call_hierarchy_prepare_uses_semantic_wait_cancellation_and_edit_invalidation() {
    let client = Client::start();
    client.open(1, "hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    request(&client, 70, "textDocument/prepareCallHierarchy", 0);
    no_responses(&client, 71, &[70]);
    client.notify("$/cancelRequest", json!({"id":70}));
    assert_eq!(
        token_response(&client, 70)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::RequestCanceled as i32
    );
    request(&client, 72, "textDocument/prepareCallHierarchy", 0);
    client.change(2, "fresh");
    assert_eq!(
        token_response(&client, 72)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::ContentModified as i32
    );
    client.proceed.send(()).unwrap();
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 2);
    client.diagnostics(2);
    request(&client, 73, "textDocument/prepareCallHierarchy", 0);
    assert_eq!(
        token_response(&client, 73).response_result.unwrap(),
        Value::Null
    );
    no_responses(&client, 74, &[70, 72]);
    client.finish();
}

#[test]
fn document_symbols_wait_cancel_and_invalidate_with_the_document_generation() {
    let client = Client::start();
    client.open(1, "hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    request(&client, 80, "textDocument/documentSymbol", 0);
    no_responses(&client, 81, &[80]);
    client.notify("$/cancelRequest", json!({"id":80}));
    assert_eq!(
        token_response(&client, 80)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::RequestCanceled as i32
    );
    request(&client, 82, "textDocument/documentSymbol", 0);
    client.change(2, "fresh");
    assert_eq!(
        token_response(&client, 82)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::ContentModified as i32
    );
    client.proceed.send(()).unwrap();
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 2);
    client.diagnostics(2);
    request(&client, 83, "textDocument/documentSymbol", 0);
    assert_eq!(
        token_response(&client, 83).response_result.unwrap(),
        json!([])
    );
    no_responses(&client, 84, &[80, 82]);
    client.finish();
}

#[test]
fn document_symbols_negotiate_hierarchy_without_assuming_client_support() {
    for (support, expected) in [
        (json!({}), false),
        (json!({"hierarchicalDocumentSymbolSupport":false}), false),
        (json!({"hierarchicalDocumentSymbolSupport":true}), true),
    ] {
        let params=serde_json::from_value(json!({"rootUri":fixture_project_uri(),"capabilities":{"textDocument":{"documentSymbol":support}},"initializationOptions":{"snapshotPath":"snapshot"}})).unwrap();
        assert_eq!(
            ProjectConfig::from_params(params)
                .unwrap()
                .hierarchical_symbols,
            expected
        );
    }
}

#[test]
fn document_symbols_legacy_clients_receive_supported_kinds_in_flat_and_nested_results() {
    let mut symbols = json!([{"kind":24,"children":[{"kind":12},{"kind":23}]}]);
    crate::lsp::annotations::legacy_symbol_kinds(&mut symbols);
    assert_eq!(
        symbols,
        json!([{"kind":3,"children":[{"kind":12},{"kind":3}]}])
    );
}

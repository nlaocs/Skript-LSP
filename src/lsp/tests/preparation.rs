use super::*;

fn client() -> Client {
    Client::start_with_capabilities(json!({"experimental":{"skriptCompletionRefresh":true}}))
}
fn request(client: &Client, id: i32, method: &str, line: u32, character: u32) {
    client
        .connection
        .sender
        .send(
            lsp_server::Request::new(
                id.into(),
                method.into(),
                json!({"textDocument":{"uri":URI},"position":{"line":line,"character":character}}),
            )
            .into(),
        )
        .unwrap();
}
fn no_responses(client: &Client, barrier: i32, ids: &[i32]) {
    client
        .connection
        .sender
        .send(
            lsp_server::Request::new(barrier.into(), "fixture/barrier".into(), Value::Null).into(),
        )
        .unwrap();
    loop {
        let message = client.connection.receiver.recv_timeout(TIMEOUT).unwrap();
        if let Message::Response(response) = message {
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
fn refresh(client: &Client, phase: &str) -> Value {
    let Message::Notification(message) = client.until(
        |message| matches!(message,Message::Notification(n)if n.method=="skript/completionRefresh"),
    ) else {
        unreachable!()
    };
    assert_eq!(message.params["phase"], phase);
    message.params
}

#[test]
fn prepared_completion_precedes_body_but_reference_signature_and_tokens_wait() {
    let client = client();
    client.open(1, "hold:\n    hold\n    {_va");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    request(&client, 40, "textDocument/completion", 1, 8);
    let result = token_response(&client, 40).response_result.unwrap();
    let ticket = result["items"][0]["data"]["completionTicket"].clone();
    assert_eq!(result["items"][0]["data"]["completionPhase"], "prepared");
    assert_eq!(result["items"][0]["label"], "hold pattern");
    assert_eq!(result["isIncomplete"], true);
    request(&client, 41, "textDocument/completion", 2, 8);
    request(&client, 42, "textDocument/signatureHelp", 1, 8);
    assert!(client.hover(43).is_null());
    no_responses(&client, 44, &[40, 41, 42]);
    client.proceed.send(()).unwrap();
    client.diagnostics(1);
    token_response(&client, 41);
    token_response(&client, 42);
    let notification = refresh(&client, "final");
    assert_eq!(notification["ticket"], ticket);
    assert_eq!(notification["version"], 1);
    request(&client, 45, "textDocument/completion", 1, 8);
    let final_list = token_response(&client, 45).response_result.unwrap();
    assert!(
        final_list["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["data"]["completionPhase"].is_null())
    );
    no_responses(&client, 46, &[40, 41, 42, 45]);
    client.finish();
}

#[test]
fn rejected_prepared_owner_refreshes_once_and_normal_requests_wait_for_final() {
    let client = client();
    client.open(1, "hold-revoke:\n    hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), -1);
    request(&client, 40, "textDocument/completion", 1, 8);
    let result = token_response(&client, 40).response_result.unwrap();
    let ticket = result["items"][0]["data"]["completionTicket"].clone();
    client.proceed.send(()).unwrap();
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    let notification = refresh(&client, "revoked");
    assert_eq!(notification["ticket"], ticket);
    request(&client, 41, "textDocument/completion", 1, 8);
    no_responses(&client, 42, &[40, 41]);
    client.proceed.send(()).unwrap();
    client.diagnostics(1);
    token_response(&client, 41);
    no_responses(&client, 43, &[40, 41]);
    client.finish();
}

#[test]
fn rapid_context_edits_drop_old_preparation_and_only_analyze_latest_revision() {
    let client = client();
    client.open(1, "hold:\n    hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    request(&client, 40, "textDocument/completion", 1, 8);
    let old = token_response(&client, 40).response_result.unwrap();
    client.change(2, "hold:\n    {_va");
    request(&client, 41, "textDocument/completion", 1, 8);
    client.change(3, "hold-raw:\n    hold");
    assert_eq!(
        token_response(&client, 41)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::ContentModified as i32
    );
    request(&client, 42, "textDocument/completion", 1, 8);
    no_responses(&client, 43, &[40, 42]);
    client.proceed.send(()).unwrap();
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 3);
    let current = token_response(&client, 42).response_result.unwrap();
    assert!(current["items"].as_array().unwrap().is_empty());
    assert!(!old["items"].as_array().unwrap().is_empty());
    client.proceed.send(()).unwrap();
    client.diagnostics(3);
    no_responses(&client, 44, &[40, 41, 42]);
    assert!(!client.actions.lock().unwrap().contains(&Action::Started(2)));
    client.finish();
}

#[test]
fn close_reopen_same_version_has_new_preparation_ticket_and_ignores_old_finish() {
    let client = client();
    client.open(1, "hold:\n    hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    request(&client, 40, "textDocument/completion", 1, 8);
    let first = token_response(&client, 40).response_result.unwrap();
    client.notify("textDocument/didClose", json!({"textDocument":{"uri":URI}}));
    client.diagnostics(1);
    client.open(1, "hold:\n    hold");
    request(&client, 41, "textDocument/completion", 1, 8);
    no_responses(&client, 42, &[41]);
    client.proceed.send(()).unwrap();
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    let second = token_response(&client, 41).response_result.unwrap();
    assert_ne!(
        first["items"][0]["data"]["completionTicket"],
        second["items"][0]["data"]["completionTicket"]
    );
    client.proceed.send(()).unwrap();
    client.diagnostics(1);
    assert_eq!(
        refresh(&client, "final")["ticket"],
        second["items"][0]["data"]["completionTicket"]
    );
    no_responses(&client, 43, &[40, 41]);
    client.finish();
}

#[test]
fn prepared_terminal_outcomes_never_retain_current_proposals_or_pending_references() {
    for suffix in ["fatal", "cancel", "stale", "partial", "panic"] {
        let client = client();
        client.open(1, &format!("hold-{suffix}:\n    hold\n    {{_va"));
        assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
        request(&client, 40, "textDocument/completion", 1, 8);
        let proposal = token_response(&client, 40).response_result.unwrap();
        request(&client, 41, "textDocument/completion", 2, 8);
        no_responses(&client, 42, &[40, 41]);
        client.proceed.send(()).unwrap();
        if suffix == "panic" {
            let notice = refresh(&client, "failed");
            assert_eq!(
                notice["ticket"],
                proposal["items"][0]["data"]["completionTicket"]
            );
            assert_eq!(
                token_response(&client, 41)
                    .response_result
                    .unwrap_err()
                    .code,
                ErrorCode::InternalError as i32
            );
            client.diagnostics(1);
            assert!(client.server.join().unwrap().is_err());
        } else {
            if matches!(suffix, "fatal" | "partial") {
                client.diagnostics(1);
            }
            token_response(&client, 41);
            assert_eq!(
                refresh(&client, "final")["ticket"],
                proposal["items"][0]["data"]["completionTicket"]
            );
            request(&client, 43, "textDocument/completion", 1, 8);
            let result = token_response(&client, 43).response_result.unwrap();
            assert!(
                result["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|item| item["data"]["completionPhase"].is_null())
            );
            if suffix != "partial" {
                assert!(result["items"].as_array().unwrap().is_empty());
            }
            client.finish();
        }
    }
}

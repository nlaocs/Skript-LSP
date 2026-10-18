use super::*;
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::sync::Mutex;
use std::time::Duration;
use worker::{Analysis, Analyzer};

#[path = "tests/annotations.rs"]
mod annotations;
#[path = "tests/preparation.rs"]
mod preparation;

const URI: &str = "file:///project/example.sk";
const TIMEOUT: Duration = Duration::from_secs(30);

#[test]
fn hover_content_format_uses_first_supported_client_preference_and_defaults_to_plaintext() {
    for (hover, expected) in [
        (json!(null), MarkupKind::PlainText),
        (json!({}), MarkupKind::PlainText),
        (json!({"contentFormat":[]}), MarkupKind::PlainText),
        (
            json!({"contentFormat":["markdown","plaintext"]}),
            MarkupKind::Markdown,
        ),
        (
            json!({"contentFormat":["plaintext","markdown"]}),
            MarkupKind::PlainText,
        ),
    ] {
        let params = serde_json::from_value(json!({
            "rootUri":"file:///project", "capabilities":{"textDocument":{"hover":hover}},
            "initializationOptions":{"snapshotPath":"snapshot"}
        }))
        .unwrap();
        assert_eq!(
            ProjectConfig::from_params(params).unwrap().hover_format,
            expected
        );
    }
}

#[derive(Debug, PartialEq)]
enum Action {
    Started(i32),
    Finished(i32),
    Released(String, u64),
}

struct TestAnalyzer {
    started: Sender<i32>,
    proceed: Receiver<()>,
    released: Sender<(String, u64)>,
    actions: Arc<Mutex<Vec<Action>>>,
    completion_index: Option<Arc<completion::CatalogIndex>>,
    preparation_enabled: bool,
}
impl Analyzer for TestAnalyzer {
    fn completion_index(&self) -> Option<Arc<completion::CatalogIndex>> {
        self.completion_index.clone()
    }
    fn analyze_with_preparation(
        &mut self,
        job: &Job,
        publish: &mut dyn FnMut(worker::Preparation),
    ) -> Analysis {
        if self.preparation_enabled {
            let view = completion::PreparedDocument::test_fixture(Arc::clone(&job.source));
            let owner = view.owner_at(Position::new(1, 0)).cloned();
            publish(worker::Preparation::Ready(Box::new(view)));
            if job.source.starts_with("hold-revoke") {
                self.started.send(-job.version).unwrap();
                self.proceed.recv_timeout(TIMEOUT).unwrap();
                publish(worker::Preparation::Invalidated(owner.unwrap()));
            }
        }
        self.analyze(job)
    }
    fn analyze(&mut self, job: &Job) -> Analysis {
        self.actions
            .lock()
            .unwrap()
            .push(Action::Started(job.version));
        self.started.send(job.version).unwrap();
        if job.source.as_ref().starts_with("hold") {
            self.proceed.recv_timeout(TIMEOUT).unwrap();
        }
        let fixture_source = if self.preparation_enabled {
            job.source
                .lines()
                .next()
                .unwrap_or("")
                .trim_end_matches(':')
        } else {
            &job.source
        };
        let outcome = match fixture_source
            .strip_prefix("hold-")
            .unwrap_or(fixture_source)
        {
            "panic" => panic!("controlled worker failure"),
            "cancel" => Outcome::Cancelled,
            "stale" => Outcome::Stale,
            "fatal" => Outcome::Diagnostics(
                vec![diagnostics::fatal(
                    "lsp.analysis-failed",
                    "fatal fixture".into(),
                )],
                "fatal",
            ),
            "partial" => Outcome::Diagnostics(
                vec![diagnostics::fatal(
                    "fixture.extension-fault",
                    "partial fixture".into(),
                )],
                "partial",
            ),
            "" => Outcome::Diagnostics(Vec::new(), "complete"),
            _ => Outcome::Diagnostics(
                vec![diagnostics::fatal(
                    "fixture",
                    format!("version {}", job.version),
                )],
                "complete",
            ),
        };
        self.actions
            .lock()
            .unwrap()
            .push(Action::Finished(job.version));
        let hover = if !job.source.is_empty()
            && matches!(outcome, Outcome::Diagnostics(_, "complete" | "partial"))
        {
            Some(hover::Document::test_fixture(Arc::clone(&job.source)))
        } else {
            None
        };
        // Deliberately supply a stale-looking snapshot with fatal outcomes:
        // protocol policy must still clear tokens rather than trust this field.
        let semantic_tokens = matches!(
            outcome,
            Outcome::Diagnostics(_, "complete" | "partial" | "fatal")
        )
        .then(|| semantic_tokens::Document::test_fixture(&job.source));
        Analysis {
            outcome,
            hover,
            semantic: None,
            inlay: None,
            annotations_error: None,
            project_semantics: Vec::new(),
            semantic_tokens,
            semantic_error: None,
            completion: Some(completion::Document::test_fixture(Arc::clone(&job.source))),
            completion_error: None,
            analysis_metrics: Value::Null,
            parse_ms: 0.0,
            report_ms: 0.0,
        }
    }
    fn release(&mut self, uri: &str, revision: u64) -> Result<(), String> {
        self.actions
            .lock()
            .unwrap()
            .push(Action::Released(uri.to_owned(), revision));
        self.released.send((uri.to_owned(), revision)).unwrap();
        Ok(())
    }
}

struct Client {
    connection: Connection,
    started: Receiver<i32>,
    proceed: Sender<()>,
    released: Receiver<(String, u64)>,
    actions: Arc<Mutex<Vec<Action>>>,
    server: thread::JoinHandle<Result<(), String>>,
}
impl Client {
    fn start() -> Self {
        Self::start_with_capabilities(json!({}))
    }
    fn start_with_capabilities(capabilities: Value) -> Self {
        Self::start_with_index(capabilities, None)
    }
    fn start_with_index(
        capabilities: Value,
        completion_index: Option<Arc<completion::CatalogIndex>>,
    ) -> Self {
        let (connection, server_connection) = Connection::memory();
        let (started_sender, started) = unbounded();
        let (proceed, proceed_receiver) = unbounded();
        let (released_sender, released) = unbounded();
        let actions = Arc::new(Mutex::new(Vec::new()));
        let analyzer_actions = Arc::clone(&actions);
        let server = thread::spawn(move || {
            serve(
                server_connection,
                Box::new(move |config| {
                    Ok(Box::new(TestAnalyzer {
                        started: started_sender,
                        proceed: proceed_receiver,
                        released: released_sender,
                        actions: analyzer_actions,
                        completion_index,
                        preparation_enabled: config.completion_refresh,
                    }))
                }),
            )
        });
        let client = Self {
            connection,
            started,
            proceed,
            released,
            actions,
            server,
        };
        client.connection.sender.send(lsp_server::Request::new(1.into(), "initialize".into(), json!({"rootUri":"file:///project","capabilities":capabilities,"initializationOptions":{"snapshotPath":"snapshot"}})).into()).unwrap();
        let response = client.until(
            |message| matches!(message, Message::Response(response) if response.id == 1.into()),
        );
        let Message::Response(response) = response else {
            unreachable!()
        };
        let result = response.response_result.unwrap();
        assert_eq!(result["capabilities"]["positionEncoding"], "utf-16");
        assert_eq!(result["capabilities"]["textDocumentSync"]["change"], 1);
        assert_eq!(result["capabilities"]["hoverProvider"], true);
        client.notify("initialized", json!({}));
        client
    }
    fn notify(&self, method: &str, params: Value) {
        self.connection
            .sender
            .send(Notification::new(method.into(), params).into())
            .unwrap();
    }
    fn until(&self, predicate: impl Fn(&Message) -> bool) -> Message {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let message = self
                .connection
                .receiver
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("server response deadline");
            if predicate(&message) {
                return message;
            }
            assert!(
                !matches!(message, Message::Notification(notification) if notification.method == "textDocument/publishDiagnostics"),
                "unexpected diagnostic while waiting for protocol response"
            );
        }
    }
    fn sync(&self, id: i32) {
        self.connection
            .sender
            .send(lsp_server::Request::new(id.into(), "fixture/barrier".into(), Value::Null).into())
            .unwrap();
        self.until(
            |message| matches!(message, Message::Response(response) if response.id == id.into()),
        );
    }
    fn hover(&self, id: i32) -> Value {
        self.connection
            .sender
            .send(
                lsp_server::Request::new(
                    id.into(),
                    "textDocument/hover".into(),
                    json!({"textDocument":{"uri":URI},"position":{"line":0,"character":0}}),
                )
                .into(),
            )
            .unwrap();
        let Message::Response(response) = self.until(
            |message| matches!(message, Message::Response(response) if response.id == id.into()),
        ) else {
            unreachable!()
        };
        response.response_result.unwrap()
    }
    fn open(&self, version: i32, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":URI,"languageId":"skript","version":version,"text":text}}),
        );
    }
    fn change(&self, version: i32, text: &str) {
        self.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":URI,"version":version},"contentChanges":[{"text":text}]}),
        );
    }
    fn diagnostics(&self, version: i32) -> Value {
        let message = self.until(|message| matches!(message, Message::Notification(notification) if notification.method == "textDocument/publishDiagnostics"));
        let Message::Notification(notification) = message else {
            unreachable!()
        };
        assert_eq!(
            notification.params["version"], version,
            "obsolete result was published"
        );
        notification.params
    }
    fn finish(self) {
        self.connection
            .sender
            .send(lsp_server::Request::new(99.into(), "shutdown".into(), Value::Null).into())
            .unwrap();
        let message = self.until(
            |message| matches!(message, Message::Response(response) if response.id == 99.into()),
        );
        let Message::Response(response) = message else {
            unreachable!()
        };
        assert_eq!(response.response_result.unwrap(), Value::Null);
        self.notify("exit", Value::Null);
        assert!(self.server.join().unwrap().is_ok());
    }
}

fn semantic_capabilities() -> Value {
    json!({"textDocument":{"semanticTokens":{
        "requests":{"full":true},"tokenTypes":["skriptLiteral", "function"],
        "tokenModifiers":[],"formats":["relative"]
    }}})
}

fn token_request(client: &Client, id: i32) {
    client
        .connection
        .sender
        .send(
            lsp_server::Request::new(
                id.into(),
                "textDocument/semanticTokens/full".into(),
                json!({"textDocument":{"uri":URI}}),
            )
            .into(),
        )
        .unwrap();
}

fn token_response(client: &Client, id: i32) -> Response {
    let Message::Response(response) = client.until(
        |message| matches!(message, Message::Response(response) if response.id == id.into()),
    ) else {
        unreachable!()
    };
    response
}

fn completion_request(client: &Client, id: i32, column: u32) {
    client
        .connection
        .sender
        .send(
            lsp_server::Request::new(
                id.into(),
                "textDocument/completion".into(),
                json!({"textDocument":{"uri":URI},"position":{"line":0,"character":column}}),
            )
            .into(),
        )
        .unwrap();
}

fn signature_request(client: &Client, id: i32, column: u32) {
    client
        .connection
        .sender
        .send(
            lsp_server::Request::new(
                id.into(),
                "textDocument/signatureHelp".into(),
                json!({"textDocument":{"uri":URI},"position":{"line":0,"character":column}}),
            )
            .into(),
        )
        .unwrap();
}

#[test]
fn signature_and_suppressed_completion_respond_before_the_held_worker() {
    let client = Client::start_with_index(
        json!({"textDocument":{"signatureHelp":{"signatureInformation":{"activeParameterSupport":true,"parameterInformation":{"labelOffsetSupport":true}}}}}),
        Some(completion::CatalogIndex::test_signature_index()),
    );
    client.open(1, "hold player to ");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    signature_request(&client, 40, 15);
    let result = token_response(&client, 40).response_result.unwrap();
    assert_eq!(
        result["signatures"][0]["label"],
        "hold %objects% to %locations%"
    );
    assert_eq!(result["signatures"][0]["activeParameter"], 1);
    client.change(2, "hold \"literal");
    completion_request(&client, 41, 13);
    let result = token_response(&client, 41).response_result.unwrap();
    assert_eq!(result["items"], json!([]));
    assert_eq!(result["isIncomplete"], false);
    signature_request(&client, 42, 13);
    assert!(
        token_response(&client, 42)
            .response_result
            .unwrap()
            .is_null()
    );
    client.proceed.send(()).unwrap();
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 2);
    client.proceed.send(()).unwrap();
    client.diagnostics(2);
    client
        .connection
        .sender
        .send(lsp_server::Request::new(43.into(), "fixture/barrier".into(), Value::Null).into())
        .unwrap();
    loop {
        let message = client.connection.receiver.recv_timeout(TIMEOUT).unwrap();
        if let Message::Response(response) = message {
            assert_eq!(
                response.id,
                43.into(),
                "a completed editing request responded twice"
            );
            break;
        }
    }
    client.finish();
}

#[test]
fn signature_requests_cancel_and_invalidate_on_change_close_reopen_and_shutdown() {
    let client = Client::start();
    client.open(1, "hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    signature_request(&client, 40, 0);
    signature_request(&client, 41, 0);
    signature_request(&client, 42, 99);
    assert_eq!(
        token_response(&client, 42)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::InvalidParams as i32
    );
    client.notify("$/cancelRequest", json!({"id":40}));
    assert_eq!(
        token_response(&client, 40)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::RequestCanceled as i32
    );
    client.change(2, "hold");
    assert_eq!(
        token_response(&client, 41)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::ContentModified as i32
    );
    signature_request(&client, 43, 0);
    client.notify("textDocument/didClose", json!({"textDocument":{"uri":URI}}));
    assert_eq!(
        token_response(&client, 43)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::ContentModified as i32
    );
    client.diagnostics(2);
    signature_request(&client, 44, 0);
    assert!(
        token_response(&client, 44)
            .response_result
            .unwrap()
            .is_null()
    );
    client.open(0, "hold");
    signature_request(&client, 45, 0);
    client.sync(46);
    client
        .connection
        .sender
        .send(lsp_server::Request::new(99.into(), "shutdown".into(), Value::Null).into())
        .unwrap();
    assert_eq!(
        token_response(&client, 45)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::RequestCanceled as i32
    );
    assert!(
        token_response(&client, 99)
            .response_result
            .unwrap()
            .is_null()
    );
    client.proceed.send(()).unwrap();
    client.notify("exit", Value::Null);
    assert!(client.server.join().unwrap().is_ok());
}

#[test]
fn signature_fatal_cancel_stale_partial_and_worker_failure_are_terminal() {
    for suffix in ["fatal", "cancel", "stale", "partial", "panic"] {
        let client = Client::start();
        client.open(1, &format!("hold-{suffix}"));
        assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
        signature_request(&client, 40, 0);
        client.sync(41);
        client.proceed.send(()).unwrap();
        if matches!(suffix, "fatal" | "partial") {
            client.diagnostics(1);
        }
        let response = token_response(&client, 40);
        if suffix == "panic" {
            assert_eq!(
                response.response_result.unwrap_err().code,
                ErrorCode::InternalError as i32
            );
            assert!(client.server.join().unwrap().is_err());
        } else {
            assert!(response.response_result.unwrap().is_null());
            client.finish();
        }
    }
}

#[test]
fn signature_pending_limit_rejects_excess_requests_and_shutdown_finishes_the_rest() {
    let client = Client::start();
    client.open(1, "hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    for id in 100..165 {
        signature_request(&client, id, 0);
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
    assert!(
        token_response(&client, 99)
            .response_result
            .unwrap()
            .is_null()
    );
    client.proceed.send(()).unwrap();
    client.notify("exit", Value::Null);
    assert!(client.server.join().unwrap().is_ok());
}

#[test]
fn syntax_completion_does_not_wait_for_the_held_worker_or_respond_twice() {
    let client = Client::start_with_index(
        json!({}),
        Some(completion::CatalogIndex::test_syntax_index()),
    );
    client.open(1, "hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    completion_request(&client, 40, 4);
    let result = token_response(&client, 40).response_result.unwrap();
    assert_eq!(result["items"][0]["label"], "hold pattern");
    assert_eq!(result["isIncomplete"], true);
    // The response above must arrive before this deterministic worker barrier.
    client.proceed.send(()).unwrap();
    client.diagnostics(1);
    client.sync(41);
    client.finish();
}

#[test]
fn completion_waits_for_the_current_analysis_and_cancels_independently() {
    let client = Client::start();
    client.open(1, "hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    completion_request(&client, 40, 0);
    completion_request(&client, 41, 0);
    completion_request(&client, 42, 99);
    assert_eq!(
        token_response(&client, 42)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::InvalidParams as i32
    );
    assert!(client.hover(43).is_null());
    client.notify("$/cancelRequest", json!({"id":40}));
    assert_eq!(
        token_response(&client, 40)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::RequestCanceled as i32
    );
    client.proceed.send(()).unwrap();
    client.diagnostics(1);
    let result = token_response(&client, 41).response_result.unwrap();
    assert_eq!(result["items"][0]["label"], "hold");
    completion_request(&client, 44, 0);
    assert_eq!(token_response(&client, 44).response_result.unwrap(), result);
    client.finish();
}

#[test]
fn completion_edit_close_and_reopen_terminate_old_requests_without_reusing_a_cache() {
    let client = Client::start();
    client.open(1, "hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    completion_request(&client, 40, 0);
    client.sync(41);
    client.change(2, "hold");
    assert_eq!(
        token_response(&client, 40)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::ContentModified as i32
    );
    completion_request(&client, 42, 0);
    client.sync(43);
    client.notify("textDocument/didClose", json!({"textDocument":{"uri":URI}}));
    assert_eq!(
        token_response(&client, 42)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::ContentModified as i32
    );
    client.diagnostics(2);
    completion_request(&client, 44, 0);
    assert_eq!(
        token_response(&client, 44).response_result.unwrap()["items"],
        json!([])
    );
    client.open(0, "fresh");
    completion_request(&client, 45, 0);
    client.sync(46);
    client.proceed.send(()).unwrap();
    client.diagnostics(0);
    assert_eq!(
        token_response(&client, 45).response_result.unwrap()["items"][0]["label"],
        "fresh"
    );
    client.finish();
}

#[test]
fn completion_fatal_cancel_stale_partial_and_worker_failure_are_terminal() {
    for suffix in ["fatal", "cancel", "stale", "partial", "panic"] {
        let client = Client::start();
        client.open(1, &format!("hold-{suffix}"));
        assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
        completion_request(&client, 40, 0);
        client.sync(41);
        client.proceed.send(()).unwrap();
        if matches!(suffix, "fatal" | "partial") {
            client.diagnostics(1);
        }
        let response = token_response(&client, 40);
        if suffix == "panic" {
            assert_eq!(
                response.response_result.unwrap_err().code,
                ErrorCode::InternalError as i32
            );
            assert!(client.server.join().unwrap().is_err());
        } else {
            assert_eq!(
                response.response_result.unwrap()["items"]
                    .as_array()
                    .unwrap()
                    .len(),
                if suffix == "partial" { 1 } else { 0 }
            );
            client.finish();
        }
    }
}

#[test]
fn completion_pending_request_is_cancelled_before_shutdown_response() {
    let client = Client::start();
    client.open(1, "hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    completion_request(&client, 40, 0);
    client.sync(41);
    client
        .connection
        .sender
        .send(lsp_server::Request::new(99.into(), "shutdown".into(), Value::Null).into())
        .unwrap();
    assert_eq!(
        token_response(&client, 40)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::RequestCanceled as i32
    );
    assert_eq!(
        token_response(&client, 99).response_result.unwrap(),
        Value::Null
    );
    client.proceed.send(()).unwrap();
    client.notify("exit", Value::Null);
    assert!(client.server.join().unwrap().is_ok());
}

#[test]
fn semantic_requests_wait_without_blocking_hover_and_cancel_independently() {
    let client = Client::start_with_capabilities(semantic_capabilities());
    client.open(1, "hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    token_request(&client, 40);
    token_request(&client, 41);
    client.sync(42);
    assert!(client.hover(43).is_null());
    client.notify("$/cancelRequest", json!({"id":40}));
    assert_eq!(
        token_response(&client, 40)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::RequestCanceled as i32
    );
    client.proceed.send(()).unwrap();
    client.diagnostics(1);
    assert_eq!(
        token_response(&client, 41).response_result.unwrap()["data"],
        json!([0, 0, 4, 0, 0])
    );
    token_request(&client, 44);
    assert_eq!(
        token_response(&client, 44).response_result.unwrap()["data"],
        json!([0, 0, 4, 0, 0])
    );
    assert!(!client.hover(45).is_null());
    client.finish();
}

#[test]
fn semantic_requests_are_invalidated_by_edit_close_and_reopen() {
    let client = Client::start_with_capabilities(semantic_capabilities());
    client.open(10, "hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 10);
    token_request(&client, 40);
    client.sync(41);
    client.change(11, "hold");
    assert_eq!(
        token_response(&client, 40)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::ContentModified as i32
    );
    client.proceed.send(()).unwrap();
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 11);
    token_request(&client, 42);
    client.sync(43);
    client.notify("textDocument/didClose", json!({"textDocument":{"uri":URI}}));
    assert_eq!(
        token_response(&client, 42)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::ContentModified as i32
    );
    client.diagnostics(11);
    client.open(1, "new");
    token_request(&client, 44);
    client.proceed.send(()).unwrap();
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    client.diagnostics(1);
    assert_eq!(
        token_response(&client, 44).response_result.unwrap()["data"],
        json!([0, 0, 3, 0, 0])
    );
    client.finish();
}

#[test]
fn semantic_empty_fatal_cancelled_and_stale_analyses_finish_requests() {
    for source in ["", "fatal", "cancel", "stale"] {
        let client = Client::start_with_capabilities(semantic_capabilities());
        client.open(1, source);
        if matches!(source, "" | "fatal") {
            client.diagnostics(1);
        }
        token_request(&client, 40);
        assert_eq!(
            token_response(&client, 40).response_result.unwrap()["data"],
            json!([])
        );
        client.finish();
    }
}

#[test]
fn semantic_pending_requests_finish_on_fatal_cancel_stale_and_worker_failure() {
    for outcome in ["fatal", "cancel", "stale", "panic"] {
        let client = Client::start_with_capabilities(semantic_capabilities());
        client.open(1, &format!("hold-{outcome}"));
        assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
        token_request(&client, 40);
        client.sync(41);
        client.proceed.send(()).unwrap();
        if outcome == "fatal" {
            client.diagnostics(1);
        }
        let response = token_response(&client, 40);
        if outcome == "panic" {
            assert_eq!(
                response.response_result.unwrap_err().code,
                ErrorCode::InternalError as i32
            );
            client.diagnostics(1);
            assert!(client.server.join().unwrap().is_err());
        } else {
            assert_eq!(response.response_result.unwrap()["data"], json!([]));
            client.finish();
        }
    }
}

#[test]
fn semantic_pending_request_is_cancelled_before_shutdown_response() {
    let client = Client::start_with_capabilities(semantic_capabilities());
    client.open(1, "hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    token_request(&client, 40);
    client.sync(41);
    client
        .connection
        .sender
        .send(lsp_server::Request::new(99.into(), "shutdown".into(), Value::Null).into())
        .unwrap();
    assert_eq!(
        token_response(&client, 40)
            .response_result
            .unwrap_err()
            .code,
        ErrorCode::RequestCanceled as i32
    );
    assert_eq!(
        token_response(&client, 99).response_result.unwrap(),
        Value::Null
    );
    client.proceed.send(()).unwrap();
    client.notify("exit", Value::Null);
    assert!(client.server.join().unwrap().is_ok());
}

#[test]
fn old_completion_is_discarded_and_pending_edits_are_coalesced_without_blocking_protocol() {
    let client = Client::start();
    client.open(1, "hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    for version in 2..=8 {
        client.change(version, "");
    }
    // Response arrives while the parser is still blocked, proving I/O dispatch
    // is independent. All prior notifications have now been handled.
    client.sync(20);
    client.proceed.send(()).unwrap();
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 8);
    assert!(
        client.diagnostics(8)["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(client.started.try_recv().is_err());
    client.finish();
}

#[test]
fn close_and_version_reset_reopen_cannot_publish_the_previous_open_generation() {
    let client = Client::start();
    client.open(30, "hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 30);
    client.notify("textDocument/didClose", json!({"textDocument":{"uri":URI}}));
    assert!(
        client.diagnostics(30)["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    client.open(0, "");
    client.sync(21);
    assert!(
        client.released.try_recv().is_err(),
        "release must wait for the old parse to finish"
    );
    client.proceed.send(()).unwrap();
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 0);
    // Project open allocates a generation, then its current parse revision.
    assert_eq!(
        client.released.recv_timeout(TIMEOUT).unwrap(),
        (URI.to_owned(), 2)
    );
    assert_eq!(
        &client.actions.lock().unwrap()[..4],
        &[
            Action::Started(30),
            Action::Finished(30),
            Action::Released(URI.to_owned(), 2),
            Action::Started(0)
        ]
    );
    // No old nonempty diagnostic may appear before the new result.
    loop {
        let message = client.connection.receiver.recv_timeout(TIMEOUT).unwrap();
        if let Message::Notification(notification) = message {
            if notification.method == "textDocument/publishDiagnostics" {
                assert_eq!(notification.params["version"], 0);
                assert!(
                    notification.params["diagnostics"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
                break;
            }
        }
    }
    client.finish();
}

#[test]
fn latest_edit_precedes_pending_open_without_dropping_either_document() {
    let client = Client::start();
    client.open(1, "hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    client.notify("textDocument/didOpen", json!({"textDocument":{"uri":"file:///project/z.sk","languageId":"skript","version":55,"text":""}}));
    client.change(2, "");
    client.sync(22);
    client.proceed.send(()).unwrap();
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 2);
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 55);
    assert!(
        client.diagnostics(2)["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        client.diagnostics(55)["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    client.finish();
}

#[test]
fn cancellation_stale_fatal_and_partial_remain_distinct() {
    let client = Client::start();
    client.open(1, "cancel");
    for (version, text, status) in [
        (1, "cancel", "cancelled"),
        (2, "stale", "stale"),
        (3, "fatal", "fatal"),
        (4, "partial", "partial"),
    ] {
        if version > 1 {
            client.change(version, text);
        }
        client.until(|message| matches!(message, Message::Notification(notification) if notification.method == "window/logMessage" && notification.params["message"].as_str().is_some_and(|message| message.contains(&format!("\"status\":\"{status}\"")))));
        if version >= 3 {
            let diagnostics = client.diagnostics(version);
            assert_eq!(diagnostics["diagnostics"].as_array().unwrap().len(), 1);
            assert!(
                diagnostics["diagnostics"][0]["message"]
                    .as_str()
                    .unwrap()
                    .contains(status)
            );
        } else {
            assert!(client.connection.receiver.try_iter().all(|message| !matches!(message, Message::Notification(notification) if notification.method == "textDocument/publishDiagnostics")));
        }
    }
    client.finish();
}

#[test]
fn hover_cache_is_invalidated_before_a_blocked_reparse_and_old_results_cannot_restore_it() {
    let client = Client::start();
    client.open(1, "cached");
    client.diagnostics(1);
    assert!(
        client.hover(30)["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("cached")
    );
    client.change(2, "hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 2);
    // The parser cannot finish until proceed is sent. A response here proves
    // both immediate invalidation and that hover never waits for the worker.
    assert!(client.hover(31).is_null());
    client.change(3, "");
    assert!(client.hover(32).is_null());
    client.proceed.send(()).unwrap();
    client.diagnostics(3);
    assert!(client.hover(33).is_null());
    client.finish();
}

#[test]
fn hover_cache_does_not_survive_close_and_reopen_with_reset_version() {
    let client = Client::start();
    client.open(10, "cached");
    client.diagnostics(10);
    assert!(!client.hover(34).is_null());
    client.change(11, "hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 10);
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 11);
    client.notify("textDocument/didClose", json!({"textDocument":{"uri":URI}}));
    client.diagnostics(11);
    assert!(client.hover(35).is_null());
    client.open(0, "fresh");
    assert!(client.hover(36).is_null());
    client.proceed.send(()).unwrap();
    client.diagnostics(0);
    let content = client.hover(37)["contents"]["value"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(content.contains("fresh"));
    assert!(!content.contains("hold"));
    client.finish();
}

#[test]
fn initialization_failure_returns_an_error_without_claiming_capabilities() {
    let (client, connection) = Connection::memory();
    let server =
        thread::spawn(|| serve(connection, Box::new(|_| Err("snapshot unavailable".into()))));
    client.sender.send(lsp_server::Request::new(1.into(), "initialize".into(), json!({"rootUri":"file:///project","capabilities":{},"initializationOptions":{"snapshotPath":"missing"}})).into()).unwrap();
    let Message::Response(response) = client.receiver.recv_timeout(TIMEOUT).unwrap() else {
        panic!("expected initialize error")
    };
    assert!(
        response
            .response_result
            .unwrap_err()
            .message
            .contains("snapshot unavailable")
    );
    assert!(server.join().unwrap().is_err());
}

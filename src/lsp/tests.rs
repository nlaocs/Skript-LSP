use super::*;
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::sync::Mutex;
use std::time::Duration;
use worker::{Analysis, Analyzer};

const URI: &str = "file:///project/example.sk";
const TIMEOUT: Duration = Duration::from_secs(30);

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
}
impl Analyzer for TestAnalyzer {
    fn analyze(&mut self, job: &Job) -> Analysis {
        self.actions
            .lock()
            .unwrap()
            .push(Action::Started(job.version));
        self.started.send(job.version).unwrap();
        if job.source.as_ref() == "hold" {
            self.proceed.recv_timeout(TIMEOUT).unwrap();
        }
        let outcome = match job.source.as_ref() {
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
        Analysis {
            outcome,
            hover,
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
        let (connection, server_connection) = Connection::memory();
        let (started_sender, started) = unbounded();
        let (proceed, proceed_receiver) = unbounded();
        let (released_sender, released) = unbounded();
        let actions = Arc::new(Mutex::new(Vec::new()));
        let analyzer_actions = Arc::clone(&actions);
        let server = thread::spawn(move || {
            serve(
                server_connection,
                Box::new(move |_| {
                    Ok(Box::new(TestAnalyzer {
                        started: started_sender,
                        proceed: proceed_receiver,
                        released: released_sender,
                        actions: analyzer_actions,
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
        client.connection.sender.send(lsp_server::Request::new(1.into(), "initialize".into(), json!({"rootUri":"file:///project","capabilities":{},"initializationOptions":{"snapshotPath":"snapshot"}})).into()).unwrap();
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
    assert_eq!(
        client.released.recv_timeout(TIMEOUT).unwrap(),
        (URI.to_owned(), 1)
    );
    assert_eq!(
        &client.actions.lock().unwrap()[..4],
        &[
            Action::Started(30),
            Action::Finished(30),
            Action::Released(URI.to_owned(), 1),
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
fn queued_documents_are_fair_even_when_an_earlier_uri_is_edited_again() {
    let client = Client::start();
    client.open(1, "hold");
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 1);
    client.notify("textDocument/didOpen", json!({"textDocument":{"uri":"file:///project/z.sk","languageId":"skript","version":55,"text":""}}));
    client.change(2, "");
    client.sync(22);
    client.proceed.send(()).unwrap();
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 55);
    assert_eq!(client.started.recv_timeout(TIMEOUT).unwrap(), 2);
    assert!(
        client.diagnostics(55)["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        client.diagnostics(2)["diagnostics"]
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

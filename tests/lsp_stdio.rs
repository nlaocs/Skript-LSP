use lsp_server::{Message, Notification, Request, Response};
use lsp_types::{
    Diagnostic, DiagnosticSeverity, Hover, HoverContents, MarkupKind, NumberOrString, Position,
    PublishDiagnosticsParams, Range, Uri,
};
use serde_json::{Value, json};
use std::{
    io::BufReader,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tempfile::{NamedTempFile, TempDir};

// This is a protocol/process hang guard, not an analysis latency assertion.
const TIMEOUT: Duration = Duration::from_secs(120);
const HEALTHY: &str = "on load:\n    send 1 to console\n";

fn snapshot() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("utilities/skript-repl/tests/data/current-multi-addon-2.15.4")
}

fn text_macro() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts/text-macro-addon.wasm")
}

struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    messages: Receiver<Result<Message, String>>,
    reader: Option<JoinHandle<()>>,
    stderr: NamedTempFile,
    project: TempDir,
    uri: Uri,
}

impl Client {
    fn spawn() -> Self {
        let project = tempfile::tempdir().unwrap();
        let uri = url::Url::from_file_path(project.path().join("document.sk"))
            .unwrap()
            .as_str()
            .parse()
            .unwrap();
        let stderr = NamedTempFile::new().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_skript-lsp"))
            .arg("--stdio")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr.reopen().unwrap())
            .spawn()
            .expect("the built language server must start");
        let stdin = Some(child.stdin.take().unwrap());
        let stdout = child.stdout.take().unwrap();
        let (sender, messages) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut input = BufReader::new(stdout);
            loop {
                match Message::read(&mut input) {
                    Ok(Some(message)) => {
                        if sender.send(Ok(message)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        let _ = sender.send(Err(format!("invalid stdout LSP frame: {error}")));
                        break;
                    }
                }
            }
        });
        Self {
            child,
            stdin,
            messages,
            reader: Some(reader),
            stderr,
            project,
            uri,
        }
    }

    fn initialized(addons: &[PathBuf]) -> Self {
        Self::initialized_with_capabilities(addons, json!({}))
    }

    fn initialized_with_capabilities(addons: &[PathBuf], capabilities: Value) -> Self {
        let mut client = Self::spawn();
        client.initialize_with_capabilities(&snapshot(), addons, capabilities);
        let result = client.response(1).response_result.unwrap();
        assert_eq!(result["capabilities"]["positionEncoding"], "utf-16");
        assert_eq!(
            result["capabilities"]["textDocumentSync"]["openClose"],
            true
        );
        assert_eq!(result["capabilities"]["textDocumentSync"]["change"], 1);
        assert_eq!(result["capabilities"]["hoverProvider"], true);
        client.notify("initialized", json!({}));
        client
    }

    fn initialize(&mut self, snapshot: &Path, addons: &[PathBuf]) {
        self.initialize_with_capabilities(snapshot, addons, json!({}));
    }

    fn initialize_with_capabilities(
        &mut self,
        snapshot: &Path,
        addons: &[PathBuf],
        capabilities: Value,
    ) {
        let root = url::Url::from_directory_path(self.project.path()).unwrap();
        self.send(Request::new(
            1.into(),
            "initialize".into(),
            json!({
                "processId": null,
                "rootUri": root.as_str(),
                "capabilities": capabilities,
                "initializationOptions": {"snapshotPath": snapshot, "addonPaths": addons}
            }),
        ));
    }

    fn send(&mut self, message: impl Into<Message>) {
        message
            .into()
            .write(self.stdin.as_mut().expect("client stdin remains open"))
            .expect("write a framed LSP message");
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(Notification::new(method.into(), params));
    }

    fn receive(&self, deadline: Instant) -> Message {
        assert!(
            Instant::now() < deadline,
            "server did not produce the expected response; stderr: {}",
            self.stderr()
        );
        match self
            .messages
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(Ok(message)) => message,
            result => panic!(
                "LSP response missing or malformed: {result:?}\nstderr: {}",
                self.stderr()
            ),
        }
    }

    fn response(&self, id: i32) -> Response {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match self.receive(deadline) {
                Message::Response(response) => {
                    assert_eq!(response.id, id.into());
                    return response;
                }
                Message::Notification(notification)
                    if notification.method == "window/logMessage" => {}
                message => panic!("expected response {id}, got {message:?}"),
            }
        }
    }

    fn open(&mut self, version: i32, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument":{"uri":self.uri,"languageId":"skript","version":version,"text":text}}),
        );
    }

    fn change(&mut self, version: i32, text: &str) {
        self.notify(
            "textDocument/didChange",
            json!({"textDocument":{"uri":self.uri,"version":version},"contentChanges":[{"text":text}]}),
        );
    }

    fn close(&mut self) {
        self.notify(
            "textDocument/didClose",
            json!({"textDocument":{"uri":self.uri}}),
        );
    }

    fn diagnostics(&self, version: i32) -> PublishDiagnosticsParams {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match self.receive(deadline) {
                Message::Notification(notification)
                    if notification.method == "textDocument/publishDiagnostics" =>
                {
                    let params: PublishDiagnosticsParams =
                        serde_json::from_value(notification.params).unwrap();
                    assert_eq!(params.uri, self.uri);
                    assert_eq!(
                        params.version,
                        Some(version),
                        "stale or unversioned diagnostics"
                    );
                    return params;
                }
                Message::Notification(notification)
                    if notification.method == "window/logMessage" => {}
                message => panic!("expected diagnostics for version {version}, got {message:?}"),
            }
        }
    }

    fn hover(&mut self, id: i32, line: u32, character: u32) -> Option<Hover> {
        self.send(Request::new(
            id.into(),
            "textDocument/hover".into(),
            json!({
                "textDocument": {"uri": self.uri},
                "position": {"line": line, "character": character}
            }),
        ));
        serde_json::from_value(self.response(id).response_result.unwrap()).unwrap()
    }

    fn finish(mut self) {
        self.send(Request::new(2.into(), "shutdown".into(), Value::Null));
        assert_eq!(self.response(2).response_result.unwrap(), Value::Null);
        self.notify("exit", Value::Null);
        assert!(self.wait_for_exit().success(), "stderr: {}", self.stderr());
    }

    fn wait_for_exit(&mut self) -> ExitStatus {
        let deadline = Instant::now() + TIMEOUT;
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "server did not exit; stderr: {}",
                self.stderr()
            );
            thread::sleep(Duration::from_millis(10));
        };
        self.reader.take().unwrap().join().unwrap();
        // No startup banner, debug print, or unconsumed diagnostic may hide in stdout.
        for message in self.messages.try_iter() {
            match message {
                Ok(Message::Notification(notification))
                    if notification.method == "window/logMessage" => {}
                other => panic!("unexpected stdout after the final response: {other:?}"),
            }
        }
        status
    }

    fn stderr(&self) -> String {
        std::fs::read_to_string(self.stderr.path()).unwrap_or_default()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn range(start_line: u32, start_character: u32, end_line: u32, end_character: u32) -> Range {
    Range::new(
        Position::new(start_line, start_character),
        Position::new(end_line, end_character),
    )
}

fn diagnostic<'a>(params: &'a PublishDiagnosticsParams, code: &str) -> &'a Diagnostic {
    params
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == Some(NumberOrString::String(code.into())))
        .unwrap_or_else(|| panic!("missing diagnostic {code}: {params:#?}"))
}

fn assert_error(diagnostic: &Diagnostic) {
    assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
    assert_eq!(diagnostic.source.as_deref(), Some("skript-lsp"));
    assert!(!diagnostic.message.is_empty());
}

fn assert_plain_text_hover(
    hover: Option<Hover>,
    expected_range: Range,
    expected_lines: &[&str],
) -> String {
    let hover = hover.expect("the selected AST node must have hover information");
    assert_eq!(hover.range, Some(expected_range));
    let HoverContents::Markup(contents) = hover.contents else {
        panic!("hover must use MarkupContent");
    };
    assert_eq!(contents.kind, MarkupKind::PlainText);
    for expected in expected_lines {
        assert!(
            contents.value.lines().any(|line| line == *expected),
            "missing hover line {expected:?}: {}",
            contents.value
        );
    }
    contents.value
}

#[test]
fn full_text_changes_clear_diagnostics_and_close_allows_version_zero_reopen() {
    let mut client = Client::initialized(&[]);
    let invalid = "this root is not a registered structure";
    client.open(7, invalid);
    let report = client.diagnostics(7);
    let error = diagnostic(&report, "parser.structure.unclaimed");
    assert_error(error);
    assert_eq!(error.range, range(0, 0, 0, invalid.len() as u32));

    client.change(8, HEALTHY);
    assert!(client.diagnostics(8).diagnostics.is_empty());
    client.close();
    assert!(client.diagnostics(8).diagnostics.is_empty());

    client.open(0, invalid);
    assert_error(diagnostic(
        &client.diagnostics(0),
        "parser.structure.unclaimed",
    ));
    client.change(1, "");
    assert!(
        client.diagnostics(1).diagnostics.is_empty(),
        "empty documents are clean"
    );
    client.close();
    assert!(client.diagnostics(1).diagnostics.is_empty());
    client.finish();
}

#[test]
fn concrete_child_causes_replace_only_their_own_unclaimed_summaries() {
    let mut client = Client::initialized(&[]);
    let cases = [
        (
            "on load:\n    this effect does not exist\n",
            vec![(
                1,
                4,
                30,
                "no WASM Type parser is registered for this SSG Type",
            )],
        ),
        (
            "on load:\n    else:\n        send 1 to console\n",
            vec![(
                1,
                4,
                8,
                "'else' has to be placed just after another 'if' or 'else if' section",
            )],
        ),
        (
            "on load:\n    teleport a to location(b, 2, 3)\n",
            vec![
                (1, 13, 14, "Expected expression type: entity"),
                (1, 27, 28, "Unexpected trailing input"),
            ],
        ),
        (
            "on load:\n    teleport a to location(b, 2, 3)\n    teleport c to location(d, 2, 3)\n",
            vec![
                (1, 13, 14, "Expected expression type: entity"),
                (1, 27, 28, "Unexpected trailing input"),
                (2, 13, 14, "Expected expression type: entity"),
                (2, 27, 28, "Unexpected trailing input"),
            ],
        ),
    ];
    for (index, (source, expected)) in cases.into_iter().enumerate() {
        let version = index as i32 + 1;
        if index == 0 {
            client.open(version, source);
        } else {
            client.change(version, source);
        }
        let report = client.diagnostics(version);
        assert_eq!(
            report.diagnostics.len(),
            expected.len(),
            "only the concrete child failures should remain: {report:#?}"
        );
        for (diagnostic, (line, start, end, message)) in report.diagnostics.iter().zip(expected) {
            assert_error(diagnostic);
            assert_eq!(
                diagnostic.code,
                Some(NumberOrString::String("parser.syntax-incomplete".into()))
            );
            assert_eq!(diagnostic.range, range(line, start, line, end));
            assert_eq!(diagnostic.message, message);
        }
    }
    client.finish();
}

#[test]
fn raw_tree_errors_are_published_and_a_later_full_text_change_recovers() {
    let mut client = Client::initialized(&[]);
    client.open(1, "on load:\r\n \tsend 1 to console\r\n");
    let report = client.diagnostics(1);
    let error = diagnostic(&report, "parser.raw-tree.mixed-indentation");
    assert_error(error);
    assert_eq!(error.range, range(1, 0, 1, 2));
    assert!(error.message.contains("spaces and tabs"));

    client.change(2, HEALTHY);
    assert!(client.diagnostics(2).diagnostics.is_empty());
    client.finish();
}

#[test]
fn hook_diagnostics_use_utf16_crlf_blank_lines_and_the_original_eof() {
    let mut client = Client::initialized(&[text_macro()]);
    // ASCII at offset zero also keeps the fixture's related [0,1] span valid.
    let source = "on load:\r\n    # 日本😀\r\n\r\nx日本😀 reject";
    client.open(1, source);
    let report = client.diagnostics(1);
    let error = diagnostic(&report, "fixture.unclosed-delimiter");
    assert_error(error);
    // x + 日本 + a surrogate pair + space + reject = 12 UTF-16 code units.
    assert_eq!(error.range, range(3, 12, 3, 12));
    assert!(error.message.contains("closing delimiter"));
    let related = error
        .related_information
        .as_ref()
        .expect("opening span is retained");
    assert!(related.iter().any(|information| {
        information.location.uri == client.uri
            && information.location.range == range(0, 0, 0, 1)
            && information.message.contains("opening delimiter")
    }));

    client.change(2, HEALTHY);
    assert!(client.diagnostics(2).diagnostics.is_empty());
    client.finish();
}

#[test]
fn options_generated_failure_is_reported_at_the_original_call_site() {
    let mut client = Client::initialized(&[]);
    // The generated text is longer than the option use and remains an invalid Effect.
    let source = "options:\n    broken: this effect does not exist\non load:\n    {@broken}\n";
    client.open(1, source);
    let report = client.diagnostics(1);
    let error = report
        .diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.severity == Some(DiagnosticSeverity::ERROR)
                && diagnostic.range == range(3, 0, 4, 0)
        })
        .unwrap_or_else(|| panic!("generated failure lost its original call site: {report:#?}"));
    assert_error(error);
    assert!(
        matches!(&error.code, Some(NumberOrString::String(code)) if code.starts_with("parser."))
    );
    assert!(
        report
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.range.start.line != 1),
        "the option definition is not the failure call site: {report:#?}"
    );
    client.finish();
}

#[test]
fn text_macro_trap_and_disabled_component_remain_visible_as_partial_results() {
    let mut client = Client::initialized(&[text_macro()]);
    client.open(1, "on load:\n    # trap\n    send 1 to console\n");
    let report = client.diagnostics(1);
    let error = diagnostic(&report, "parser.extension-trap");
    assert_error(error);
    assert_eq!(error.data.as_ref().unwrap()["executionStatus"], "partial");
    assert!(error.message.contains("nlaocs.test.text-macro"));

    client.change(2, HEALTHY);
    let later = client.diagnostics(2);
    let error = diagnostic(&later, "parser.extension-unavailable");
    assert_error(error);
    assert_eq!(error.data.as_ref().unwrap()["executionStatus"], "partial");
    assert!(error.message.contains("nlaocs.test.text-macro"));
    client.finish();
}

#[test]
fn invalid_snapshot_returns_an_initialize_error_and_exits_with_stdin_open() {
    let mut client = Client::spawn();
    let missing_snapshot = client.project.path().join("missing-snapshot");
    client.initialize(&missing_snapshot, &[]);
    let error = client.response(1).response_result.unwrap_err();
    assert_eq!(error.code, lsp_server::ErrorCode::InternalError as i32);
    assert!(error.message.contains("parser initialization failed"));
    assert!(client.stdin.is_some(), "the client has not closed stdin");
    assert_eq!(client.wait_for_exit().code(), Some(1));
}

#[test]
fn else_without_preceding_if_keeps_its_rejection_reason_and_original_range() {
    let mut client = Client::initialized(&[]);
    client.open(1, "on load:\n    else:\n        send 1 to console\n");
    let report = client.diagnostics(1);
    // CoreLibrary rejects this Section without a dedicated semantic diagnostic,
    // so its HookRejected reason must survive the syntax-incomplete fallback.
    let error = diagnostic(&report, "parser.syntax-incomplete");
    assert_error(error);
    assert_eq!(
        error.message,
        "'else' has to be placed just after another 'if' or 'else if' section"
    );
    assert_eq!(error.range, range(1, 4, 1, 8));
    client.finish();
}

#[test]
fn long_percent_encoded_japanese_uri_can_close_and_reopen_at_version_zero() {
    let mut client = Client::initialized(&[]);
    // The filename has 123 characters; percent encoding expands its Japanese text.
    let filename = format!("{}.sk", "日".repeat(120));
    let uri = url::Url::from_file_path(client.project.path().join(filename)).unwrap();
    assert!(uri.as_str().is_ascii());
    assert!(
        uri.as_str().len() > 1024,
        "exercise the host document ID limit"
    );
    client.uri = uri.as_str().parse().unwrap();

    client.open(1, HEALTHY);
    assert!(client.diagnostics(1).diagnostics.is_empty());
    client.close();
    assert!(client.diagnostics(1).diagnostics.is_empty());
    client.open(0, HEALTHY);
    assert!(client.diagnostics(0).diagnostics.is_empty());
    client.finish();
}

#[test]
fn hover_distinguishes_registered_effect_function_argument_and_variables() {
    let mut client = Client::initialized(&[]);
    client.open(
        1,
        concat!(
            "on load:\n",
            "    send 1 to console\n",
            "    send sin(1) to console\n",
            "    send {_money} to console\n",
            "    send {balance} to console\n",
        ),
    );
    assert!(client.diagnostics(1).diagnostics.is_empty());

    let effect = assert_plain_text_hover(
        client.hover(10, 1, 5),
        range(1, 4, 1, 21),
        &[
            "Effect: Message",
            "Pattern: (message|send [message[s]]) %objects% [to %audiences%]",
            "Addon: Skript 2.15.4",
        ],
    );
    assert!(effect.contains("Sends a message to an audience"));
    assert_plain_text_hover(
        client.hover(11, 2, 10),
        range(2, 9, 2, 15),
        &[
            "Function: sin",
            "Signature: sin(n: number) -> number",
            "Return type: number",
            "Multiplicity: single",
        ],
    );
    // The argument has its own parsed type and range, rather than its parent's.
    assert_plain_text_hover(
        client.hover(12, 2, 13),
        range(2, 13, 2, 14),
        &["Return type: long", "Multiplicity: single"],
    );
    assert_plain_text_hover(
        client.hover(13, 3, 12),
        range(3, 9, 3, 17),
        &[
            "Variable",
            "Scope: local",
            "Name template: money",
            "Type (parse context, not inferred): object",
            "Multiplicity: single",
        ],
    );
    assert_plain_text_hover(
        client.hover(14, 4, 12),
        range(4, 9, 4, 18),
        &[
            "Variable",
            "Scope: global",
            "Name template: balance",
            "Type (parse context, not inferred): object",
        ],
    );
    client.finish();
}

#[test]
fn hover_uses_original_utf16_ranges_and_skips_whitespace_and_comments() {
    let mut client = Client::initialized(&[]);
    client.open(
        1,
        "on load:\r\n    send \"日本😀\" to console\r\n    # 日本😀 comment\r\n\r\n",
    );
    assert!(client.diagnostics(1).diagnostics.is_empty());

    // The emoji starts at UTF-16 column 12; the quoted string occupies [9,15).
    assert_plain_text_hover(
        client.hover(10, 1, 12),
        range(1, 9, 1, 15),
        &["Return type: string", "Multiplicity: single"],
    );
    for (id, line, character) in [(11, 1, 1), (12, 1, 8), (13, 1, 15), (14, 2, 6), (15, 3, 0)] {
        assert!(
            client.hover(id, line, character).is_none(),
            "whitespace, comments, and the literal's half-open end have no target"
        );
    }
    client.finish();
}

#[test]
fn hover_follows_document_revisions_and_keeps_healthy_nodes_in_incomplete_documents() {
    let mut client = Client::initialized(&[]);
    assert!(client.hover(10, 1, 5).is_none());
    client.send(Request::new(
        11.into(),
        "textDocument/hover".into(),
        json!({
            "textDocument": {"uri": client.uri},
            "position": {"line": 0, "character": "invalid"}
        }),
    ));
    assert_eq!(
        client.response(11).response_result.unwrap_err().code,
        lsp_server::ErrorCode::InvalidParams as i32
    );

    client.open(
        1,
        "on load:\n    send sin(1) to console\n    this effect does not exist\n",
    );
    let report = client.diagnostics(1);
    assert!(
        report.diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == Some(DiagnosticSeverity::ERROR)
                && diagnostic.range.start.line == 2
        }),
        "the document must remain incomplete: {report:#?}"
    );
    assert_plain_text_hover(
        client.hover(12, 1, 10),
        range(1, 9, 1, 15),
        &["Function: sin", "Return type: number"],
    );
    assert!(client.hover(13, 2, 5).is_none());

    client.change(2, "on load:\n    send {_money} to console\n");
    assert!(client.diagnostics(2).diagnostics.is_empty());
    let changed = assert_plain_text_hover(
        client.hover(14, 1, 10),
        range(1, 9, 1, 17),
        &["Variable", "Scope: local", "Name template: money"],
    );
    assert!(!changed.contains("Function: sin"));

    client.close();
    assert!(client.diagnostics(2).diagnostics.is_empty());
    assert!(client.hover(15, 1, 10).is_none());
    client.open(0, HEALTHY);
    assert!(client.diagnostics(0).diagnostics.is_empty());
    assert_plain_text_hover(
        client.hover(16, 1, 5),
        range(1, 4, 1, 21),
        &["Effect: Message"],
    );
    client.finish();
}

#[test]
fn hover_uses_current_document_function_signature_without_borrowing_builtin_docs() {
    let mut client = Client::initialized(&[]);
    for (version, parameter) in [(1, "message"), (2, "payload")] {
        let source = format!(
            "function greet({parameter}: string) :: string:\n    send \"hello\" to console\non load:\n    send greet(\"hello\") to console\n"
        );
        if version == 1 {
            client.open(version, &source);
        } else {
            client.change(version, &source);
        }
        let report = client.diagnostics(version);
        assert!(report.diagnostics.is_empty(), "{report:#?}");
        let text = assert_plain_text_hover(
            client.hover(10 + version, 3, 10),
            range(3, 9, 3, 23),
            &[
                "Function: greet",
                &format!("Signature: greet({parameter}: string) -> string"),
                "Return type: string",
            ],
        );
        assert!(!text.contains("Addon: Skript"));
        if version == 2 {
            assert!(!text.contains("message: string"));
        }
    }
    client.finish();
}

#[test]
fn hover_does_not_guess_a_node_inside_an_options_expansion() {
    let mut client = Client::initialized(&[]);
    client.open(
        1,
        "options:\n    value: sin(1)\non load:\n    send {@value} to console\n    send 1 to console\n",
    );
    assert!(client.diagnostics(1).diagnostics.is_empty());
    assert!(client.hover(10, 3, 12).is_none());
    // The options macro replaces the entire generated line. An untouched
    // sibling still has an exact origin and remains useful.
    assert!(client.hover(11, 3, 5).is_none());
    let hover = client.hover(12, 4, 5).expect("exact send token");
    let HoverContents::Markup(content) = hover.contents else {
        panic!("plain text hover")
    };
    assert!(content.value.contains("Effect: Message"));
    client.finish();
}

#[test]
fn hover_formats_real_ssg_docs_examples_and_implementation_classes_for_both_client_preferences() {
    for formats in [["markdown", "plaintext"], ["plaintext", "markdown"]] {
        let mut client = Client::initialized_with_capabilities(
            &[],
            json!({"textDocument":{"hover":{"contentFormat":formats}}}),
        );
        client.open(1, "on join:\n    send name of player to console\n    send display name of player to console\n    send player list name of player to console\n    send sin(1) to console\n");
        let report = client.diagnostics(1);
        assert!(report.diagnostics.is_empty(), "{report:#?}");
        for (id, line, class, block_count) in [
            (
                10,
                1,
                "org.skriptlang.skript.common.properties.elements.expressions.PropExprName",
                2,
            ),
            (
                11,
                2,
                "org.skriptlang.skript.common.properties.elements.expressions.PropExprCustomName",
                1,
            ),
            (
                12,
                3,
                "org.skriptlang.skript.bukkit.entity.player.elements.expressions.ExprPlayerListName",
                1,
            ),
        ] {
            let hover = client.hover(id, line, 11).unwrap();
            assert_eq!(hover.range.unwrap().start, Position::new(line, 9));
            let HoverContents::Markup(content) = hover.contents else {
                panic!("MarkupContent required")
            };
            assert_eq!(serde_json::to_value(content.kind).unwrap(), formats[0]);
            let text = content.value;
            assert!(text.contains(class), "{text}");
            assert!(text.contains("<red>"), "{text}");
            assert!(
                !text.contains("&lt;") && !text.contains("<strong>") && !text.contains("<em>"),
                "{text}"
            );
            assert_eq!(
                text.matches(if formats[0] == "markdown" {
                    "### Examples"
                } else {
                    "Examples:"
                })
                .count(),
                1,
                "{text}"
            );
            if formats[0] == "markdown" {
                assert_eq!(text.matches("```skript\n").count(), block_count, "{text}");
            }
            if line == 1 {
                assert!(text.contains("on join:\n\tplayer has permission \"name.red\"\n\tset the player's display name to \"<red>[admin] <gold>%name of player%\""), "{text}");
                assert!(
                    text.contains(
                        "set the name of the player's tool to \"Legendary Sword of Awesomeness\""
                    ),
                    "last example lost: {text}"
                );
                assert!(text.contains(if formats[0] == "markdown" {
                    "**Players:**"
                } else {
                    "Players:"
                }));
            }
        }
        let HoverContents::Markup(function) = client.hover(13, 4, 10).unwrap().contents else {
            panic!("function markup")
        };
        assert!(function.value.contains("sin(n: number) -> number"));
        assert!(
            !function.value.contains("Java class"),
            "function catalog has no implementation class: {}",
            function.value
        );
        client.finish();
    }
}

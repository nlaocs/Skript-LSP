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

#[test]
fn semantic_tokens_use_final_original_ranges_and_negotiated_classifications() {
    let mut client = Client::spawn();
    client.initialize_with_capabilities(
        &snapshot(),
        &[],
        json!({"textDocument":{"semanticTokens":{
            "requests":{"full":true}, "formats":["relative"], "tokenModifiers":[],
            "tokenTypes":["skriptEvent","skriptCondition","skriptEffect","skriptExpression",
                "skriptSection","skriptStructure","skriptLiteral","function","variable","skriptInterpolationDelimiter"]
        }}}),
    );
    let initialization = client.response(1).response_result.unwrap();
    let provider = &initialization["capabilities"]["semanticTokensProvider"];
    assert_eq!(provider["full"], true);
    assert!(provider.get("range").is_none());
    let types = provider["legend"]["tokenTypes"].as_array().unwrap();
    assert_eq!(types.len(), 10);
    client.notify("initialized", json!({}));
    let source = "on load:\r\n    send \"日本😀\" to console\r\n    send sin(1) to console\r\n    send {_money} to console\r\n    # no semantic color here\r\n    this effect does not exist\r\n    if 1 is 1:\r\n        send \"ok\" to console\r\n    send {_key%sin(1)%} to console\r\n    send {_key% sin(1) %} to console\r\n    send \"100%%\" to console\r\non join:\r\n    send player to console\r\n    send event-player to console\r\n    send name of player to console\r\n    send \"hello %player%\" to player\r\n";
    client.open(1, source);
    client.send(Request::new(
        10.into(),
        "textDocument/semanticTokens/full".into(),
        json!({"textDocument":{"uri":client.uri}}),
    ));
    let diagnostics = client.diagnostics(1);
    assert!(
        diagnostics
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.range.start.line == 5)
    );
    assert!(
        diagnostics.diagnostics.iter().any(|diagnostic| {
            diagnostic.range.start == Position::new(9, 9)
                && diagnostic.range.end == Position::new(9, 25)
                && diagnostic.code
                    == Some(NumberOrString::String("parser.syntax-incomplete".into()))
        }),
        "the current parser rejects spaces around the embedded expression"
    );
    let result = client.response(10).response_result.unwrap();
    let data = result["data"].as_array().unwrap();
    assert_eq!(data.len() % 5, 0);
    let mut decoded = Vec::new();
    let mut line = 0;
    let mut start = 0;
    let mut previous_end = (0, 0);
    for token in data.chunks_exact(5) {
        let delta = token[0].as_u64().unwrap() as usize;
        line += delta;
        start = if delta == 0 {
            start + token[1].as_u64().unwrap() as usize
        } else {
            token[1].as_u64().unwrap() as usize
        };
        let length = token[2].as_u64().unwrap() as usize;
        assert!(length > 0 && (line, start) >= previous_end);
        assert_eq!(token[4], 0);
        let text = source
            .lines()
            .nth(line)
            .unwrap()
            .encode_utf16()
            .collect::<Vec<_>>();
        let text = String::from_utf16(&text[start..start + length]).unwrap();
        let kind = types[token[3].as_u64().unwrap() as usize].as_str().unwrap();
        assert!(
            (line < 4 || line >= 6) && line != 9,
            "comments and unclaimed statements have no semantic tokens"
        );
        decoded.push((line, start, length, kind, text));
        previous_end = (line, start + length);
    }
    for expected in [
        (1, 4, 4, "skriptEffect", "send"),
        (1, 9, 6, "skriptLiteral", "\"日本😀\""),
        (2, 9, 3, "function", "sin"),
        (2, 13, 1, "skriptLiteral", "1"),
        (3, 9, 8, "variable", "{_money}"),
        (6, 4, 2, "skriptSection", "if"),
        (6, 9, 2, "skriptCondition", "is"),
        (8, 14, 1, "skriptInterpolationDelimiter", "%"),
        (8, 15, 3, "function", "sin"),
        (8, 21, 1, "skriptInterpolationDelimiter", "%"),
    ] {
        assert!(
            decoded.iter().any(|actual| actual.0 == expected.0
                && actual.1 == expected.1
                && actual.2 == expected.2
                && actual.3 == expected.3
                && actual.4 == expected.4),
            "missing {expected:?}: {decoded:#?}"
        );
    }
    assert!(
        !decoded
            .iter()
            .any(|token| token.0 == 10 && token.3 == "skriptInterpolationDelimiter"),
        "escaped %% is literal source"
    );
    for (line, text) in source.lines().enumerate().skip(12) {
        for (start, _) in text.match_indices("player") {
            assert!(
                decoded.iter().any(|token| token.0 == line
                    && token.1 <= start
                    && start + 6 <= token.1 + token.2
                    && token.3 == "skriptExpression"),
                "confirmed semantic capture at {line}:{start} must be colored: {decoded:?}"
            );
        }
    }
    client.change(2, "");
    assert!(client.diagnostics(2).diagnostics.is_empty());
    client.send(Request::new(
        11.into(),
        "textDocument/semanticTokens/full".into(),
        json!({"textDocument":{"uri":client.uri}}),
    ));
    assert_eq!(
        client.response(11).response_result.unwrap()["data"],
        json!([])
    );
    client.finish();
}

fn snapshot() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("utilities/skript-repl/tests/data/current-multi-addon-2.15.4")
}

#[test]
fn signature_help_uses_catalog_slots_current_functions_and_utf16_parameter_labels() {
    let mut client = Client::spawn();
    client.initialize_with_capabilities(&snapshot(), &[], json!({"textDocument":{"signatureHelp":{"signatureInformation":{"activeParameterSupport":true,"parameterInformation":{"labelOffsetSupport":true}}}}}));
    let initialized = client.response(1).response_result.unwrap();
    assert_eq!(
        initialized["capabilities"]["signatureHelpProvider"]["triggerCharacters"],
        json!(["(", ",", " "])
    );
    client.notify("initialized", json!({}));
    let source = "function greet(first: number, second: text) :: text:\r\n    return {_second}\r\non join:\r\n    teleport player to player's location\r\n    send greet(1, \"日本😀\") to console\r\n";
    client.open(1, source);
    client.diagnostics(1);
    client.send(Request::new(
        20.into(),
        "textDocument/signatureHelp".into(),
        json!({"textDocument":{"uri":client.uri},"position":{"line":3,"character":23}}),
    ));
    let pattern = client.response(20).response_result.unwrap();
    let info = &pattern["signatures"][0];
    let label = info["label"].as_str().unwrap();
    assert!(label.contains("teleport"));
    let active = info["activeParameter"].as_u64().expect("destination slot") as usize;
    let selected = &info["parameters"][active]["label"];
    let utf16 = label.encode_utf16().collect::<Vec<_>>();
    assert!(
        String::from_utf16(
            &utf16[selected[0].as_u64().unwrap() as usize..selected[1].as_u64().unwrap() as usize]
        )
        .unwrap()
        .contains("location")
    );
    client.send(Request::new(
        21.into(),
        "textDocument/signatureHelp".into(),
        json!({"textDocument":{"uri":client.uri},"position":{"line":4,"character":18}}),
    ));
    let function = client.response(21).response_result.unwrap();
    assert_eq!(function["signatures"][0]["activeParameter"], 1);
    assert!(
        function["signatures"][0]["label"]
            .as_str()
            .unwrap()
            .starts_with("greet(first:")
    );
    client.change(2, "on join:\n    send greet(1, \"x\") to console\n");
    client.send(Request::new(
        22.into(),
        "textDocument/signatureHelp".into(),
        json!({"textDocument":{"uri":client.uri},"position":{"line":1,"character":18}}),
    ));
    client.diagnostics(2);
    let revised = client.response(22).response_result.unwrap();
    let signatures = revised["signatures"]
        .as_array()
        .expect("registered statement guidance survives");
    assert!(!signatures.is_empty());
    assert!(
        signatures
            .iter()
            .all(|s| !s["label"].as_str().unwrap().starts_with("greet(")),
        "removed document Function must not survive"
    );
    assert!(
        signatures
            .iter()
            .any(|s| s["label"].as_str().unwrap().contains("send"))
    );
    client.finish();
}

#[test]
fn completion_uses_real_catalog_and_current_document_with_safe_original_edits() {
    let mut client = Client::initialized_with_capabilities(
        &[],
        json!({
            "textDocument":{"completion":{"completionItem":{"snippetSupport":true}}}
        }),
    );
    let source = "function greet(value: string) :: string:\r\n    send \"hello\" to console\r\non load:\r\n    send {_money} to console\r\n    send {_mo} to console\r\n    send si(1) to console\r\n    send gr\r\n    cancel \r\n    send name of player\r\n    send \"日本😀\" to console\r\non join:\r\n    send {_mo} to console\r\n";
    client.open(1, source);
    client.diagnostics(1);
    let variables = client.completion(30, 4, 13);
    let variable = variables["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["label"] == "{_money}")
        .expect("previous local reference in the same body");
    assert_eq!(variable["textEdit"]["newText"], "{_money");
    assert_eq!(variable["textEdit"]["range"], json!(range(4, 9, 4, 13)));
    assert!(
        !client.completion(31, 11, 13)["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["label"] == "{_money}"),
        "local history does not cross raw bodies"
    );
    let calls = client.completion(32, 5, 11);
    let sin = calls["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["label"]
                .as_str()
                .is_some_and(|label| label.starts_with("sin("))
        })
        .unwrap();
    assert_eq!(sin["textEdit"]["newText"], "sin");
    assert_eq!(sin["textEdit"]["range"], json!(range(5, 9, 5, 11)));
    assert_eq!(
        sin["insertTextFormat"], 1,
        "existing arguments are preserved"
    );
    let local_calls = client.completion(33, 6, 11);
    let greet = local_calls["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["label"]
                .as_str()
                .is_some_and(|label| label.starts_with("greet("))
        })
        .unwrap();
    assert_eq!(greet["textEdit"]["newText"], "greet(${1:value})");
    assert_eq!(greet["insertTextFormat"], 2);
    let cancel = client.completion(34, 7, 11);
    assert!(
        cancel["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["label"] == "event"
                && item["textEdit"]["newText"] == "event"
                && item["textEdit"]["range"] == json!(range(7, 11, 7, 11))),
        "fixed statement words survive trailing whitespace: {cancel}"
    );
    let names = client.completion(35, 8, 11);
    let name = names["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["textEdit"]["newText"] == "na")
        .expect("name template must not duplicate the right suffix");
    assert_eq!(name["textEdit"]["range"], json!(range(8, 9, 8, 11)));
    assert!(
        client.completion(36, 9, 11)["items"]
            .as_array()
            .unwrap()
            .is_empty(),
        "no string text suggestions"
    );
    client.send(Request::new(
        37.into(),
        "textDocument/completion".into(),
        json!({
            "textDocument":{"uri":client.uri},"position":{"line":9,"character":13}
        }),
    ));
    assert_eq!(
        client.response(37).response_result.unwrap_err().code,
        -32602,
        "surrogate interior is invalid"
    );
    client.change(2, "on load:\n    send gr\n");
    client.diagnostics(2);
    let revised = client.completion(38, 1, 11);
    assert!(
        !revised["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["label"]
                .as_str()
                .is_some_and(|label| label.starts_with("greet("))),
        "edits drop prior document functions"
    );
    client.change(3, "");
    client.diagnostics(3);
    assert!(
        !client.completion(39, 0, 0)["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    for (version, source, column, new_text) in [
        (4, "on join:\n    teleport player t", 21, "to"),
        (5, "on join:\n    teleport location(1, 2, 3) t", 32, "to"),
        (
            6,
            "on join:\n    teleport player to location(1, 2, 3)",
            21,
            "t",
        ),
    ] {
        client.change(version, source);
        // Request before awaiting diagnostics covers the edit/response ordering.
        // The held-worker unit test proves the immediate path without this race.
        let result = client.completion(40 + version, 1, column);
        let first = &result["items"][0];
        assert_eq!(first["label"], "to");
        assert_eq!(first["textEdit"]["newText"], new_text);
        assert_eq!(
            first["textEdit"]["range"],
            json!(range(1, column - 1, 1, column))
        );
        let documentation = first["documentation"].as_str().unwrap();
        assert!(
            documentation.starts_with("Syntax:\n")
                && documentation.contains("%")
                && documentation.contains("Registration: ")
        );
        assert!(
            result["items"].as_array().unwrap().len() <= 10,
            "context alternatives are bounded: {result}"
        );
        client.diagnostics(version);
    }
    client.finish();
}

#[test]
fn completion_without_snippet_support_never_inserts_placeholder_syntax() {
    let mut client = Client::spawn();
    client.initialize(&snapshot(), &[]);
    let capabilities = client.response(1).response_result.unwrap();
    assert_eq!(
        capabilities["capabilities"]["completionProvider"]["triggerCharacters"],
        json!(["{"])
    );
    assert_eq!(
        capabilities["capabilities"]["completionProvider"]["resolveProvider"],
        false
    );
    client.notify("initialized", json!({}));
    client.open(1, "on load:\n    send si\n    se\n    # si\n");
    client.diagnostics(1);
    let result = client.completion(40, 1, 11);
    let sin = result["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["label"]
                .as_str()
                .is_some_and(|label| label.starts_with("sin("))
        })
        .unwrap();
    assert_eq!(sin["textEdit"]["newText"], "sin(");
    let effects = client.completion(41, 2, 6);
    assert!(!effects["items"].as_array().unwrap().is_empty());
    for result in [&result, &effects] {
        for item in result["items"].as_array().unwrap() {
            assert_eq!(item["insertTextFormat"], 1);
            assert!(!item["textEdit"]["newText"].as_str().unwrap().contains("${"));
        }
    }
    assert!(
        client.completion(42, 3, 8)["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    client.finish();
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

    fn completion(&mut self, id: i32, line: u32, character: u32) -> Value {
        self.send(Request::new(
            id.into(),
            "textDocument/completion".into(),
            json!({
                "textDocument":{"uri":self.uri},"position":{"line":line,"character":character}
            }),
        ));
        self.response(id).response_result.unwrap()
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

#[test]
fn hover_prose_angle_placeholders_are_literal_without_added_escape_slashes() {
    for format in ["markdown", "plaintext"] {
        let mut client = Client::initialized_with_capabilities(
            &[],
            json!({"textDocument":{"hover":{"contentFormat":[format]}}}),
        );
        client.open(
            1,
            "on load:\n    log \"hello\"\n    send first 2 characters of \"hello\" to console\n",
        );
        assert!(client.diagnostics(1).diagnostics.is_empty());
        for (id, line, column, expected) in [(10, 1, 5, "<script>"), (11, 2, 11, "<x>")] {
            let HoverContents::Markup(content) = client.hover(id, line, column).unwrap().contents
            else {
                panic!("markup required")
            };
            assert_eq!(serde_json::to_value(content.kind).unwrap(), format);
            assert!(content.value.contains(expected), "{}", content.value);
            assert!(
                !content.value.contains("\\<") && !content.value.contains("\\>"),
                "added prose escapes: {}",
                content.value
            );
            if format == "markdown" {
                assert!(
                    content.value.contains(if line == 1 {
                        "`[<script>.sk]`"
                    } else {
                        "`<x>`"
                    }),
                    "{}",
                    content.value
                );
            }
        }
        client.finish();
    }
}

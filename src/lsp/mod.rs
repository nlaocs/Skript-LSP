//! A single-project, full-document LSP frontend for the shared parser service.

mod completion;
mod diagnostics;
mod hover;
mod hover_text;
mod positions;
mod public_data;
mod semantic_tokens;
mod worker;

use crossbeam_channel::{bounded, select};
use lsp_server::{Connection, ErrorCode, Message, Notification, RequestId, Response};
use lsp_types::{
    CompletionParams, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, HoverParams, InitializeParams, MarkupKind, Position,
    PublishDiagnosticsParams, SemanticTokens, SemanticTokensParams, Uri,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, io, path::PathBuf, sync::Arc, thread, time::Instant};
use worker::{Control, Event, Factory, Job, Outcome, Worker};

#[derive(Debug)]
struct ProjectConfig {
    project_uri: String,
    snapshot_path: PathBuf,
    addon_paths: Vec<PathBuf>,
    hover_format: MarkupKind,
    semantic_legend: Option<semantic_tokens::Legend>,
    completion_snippets: bool,
    signature_capabilities: completion::SignatureCapabilities,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InitializationOptions {
    snapshot_path: PathBuf,
    #[serde(default)]
    addon_paths: Vec<PathBuf>,
}

impl ProjectConfig {
    #[allow(deprecated)] // rootUri is still the explicit single-project client contract.
    fn from_params(params: InitializeParams) -> Result<Self, String> {
        let completion_snippets = params
            .capabilities
            .text_document
            .as_ref()
            .and_then(|d| d.completion.as_ref())
            .and_then(|c| c.completion_item.as_ref())
            .and_then(|c| c.snippet_support)
            .unwrap_or(false);
        let semantic_legend = semantic_tokens::Legend::negotiate(
            params
                .capabilities
                .text_document
                .as_ref()
                .and_then(|document| document.semantic_tokens.as_ref()),
        );
        let hover_format = params
            .capabilities
            .text_document
            .as_ref()
            .and_then(|document| document.hover.as_ref())
            .and_then(|hover| hover.content_format.as_ref())
            .and_then(|formats| {
                formats
                    .iter()
                    .find(|format| matches!(**format, MarkupKind::Markdown | MarkupKind::PlainText))
            })
            .cloned()
            .unwrap_or(MarkupKind::PlainText);
        let signature_capabilities = completion::SignatureCapabilities::negotiate(
            params
                .capabilities
                .text_document
                .as_ref()
                .and_then(|d| d.signature_help.as_ref()),
        );
        let root = match params.workspace_folders {
            Some(folders) if folders.len() > 1 => {
                return Err("only one project workspace is supported".into());
            }
            Some(mut folders) if folders.len() == 1 => folders.remove(0).uri,
            _ => params
                .root_uri
                .ok_or("rootUri or one workspaceFolder is required")?,
        };
        let root_url = url::Url::parse(root.as_str()).map_err(|error| error.to_string())?;
        let root_path = root_url
            .to_file_path()
            .map_err(|_| "the project root must be a file URI")?;
        let options: InitializationOptions = serde_json::from_value(
            params
                .initialization_options
                .ok_or("initializationOptions.snapshotPath is required")?,
        )
        .map_err(|error| error.to_string())?;
        if options.snapshot_path.as_os_str().is_empty() {
            return Err("snapshotPath must not be empty".into());
        }
        let absolute = |path: PathBuf| {
            if path.is_absolute() {
                path
            } else {
                root_path.join(path)
            }
        };
        Ok(Self {
            project_uri: root.to_string(),
            snapshot_path: absolute(options.snapshot_path),
            addon_paths: options.addon_paths.into_iter().map(absolute).collect(),
            hover_format,
            semantic_legend,
            completion_snippets,
            signature_capabilities,
        })
    }
}

/// Runs the language server over stdin/stdout. Only framed LSP messages use stdout.
/// Snapshot and addon paths are supplied by the client's initialization options.
pub fn run_stdio() -> Result<(), String> {
    // Own the writer separately so an initialization error is flushed without
    // waiting for a client that keeps stdin open after receiving that error.
    let (sender, outgoing) = bounded::<Message>(16);
    let (incoming, receiver) = bounded::<Message>(16);
    let reader = thread::Builder::new().name("skript-lsp-reader".into()).spawn(move || -> io::Result<()> {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        while let Some(message) = Message::read(&mut input)? {
            let exit = matches!(&message, Message::Notification(notification) if notification.method == "exit");
            if incoming.send(message).is_err() || exit { break; }
        }
        Ok(())
    }).map_err(|error| error.to_string())?;
    let writer = thread::Builder::new()
        .name("skript-lsp-writer".into())
        .spawn(move || -> io::Result<()> {
            let stdout = io::stdout();
            let mut output = stdout.lock();
            for message in outgoing {
                message.write(&mut output)?;
            }
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    let result = serve(Connection { sender, receiver }, worker::parser_factory());
    let written = writer
        .join()
        .map_err(|_| "LSP writer panicked".to_owned())?
        .map_err(|error| error.to_string());
    // On protocol/init failure the reader may still be blocked in stdin. Dropping
    // its handle does not block process exit or lose the already-flushed error.
    if reader.is_finished() {
        reader
            .join()
            .map_err(|_| "LSP reader panicked".to_owned())?
            .map_err(|error| error.to_string())?;
    }
    result.and(written)
}

fn send(connection: &Connection, message: impl Into<Message>) -> Result<(), String> {
    connection
        .sender
        .send(message.into())
        .map_err(|_| "LSP client disconnected".into())
}
fn log(connection: &Connection, message: impl Into<String>) -> Result<(), String> {
    send(
        connection,
        Notification::new(
            "window/logMessage".into(),
            json!({"type":4,"message":message.into()}),
        ),
    )
}
fn publish(
    connection: &Connection,
    uri: Uri,
    version: i32,
    diagnostics: Vec<lsp_types::Diagnostic>,
) -> Result<(), String> {
    send(
        connection,
        Notification::new(
            "textDocument/publishDiagnostics".into(),
            PublishDiagnosticsParams {
                uri,
                version: Some(version),
                diagnostics,
            },
        ),
    )
}

struct Document {
    generation: u64,
    revision: u64,
    version: i32,
    control: Control,
    hover: Option<hover::Document>,
    tokens: Option<SemanticTokens>,
    token_requests: Vec<RequestId>,
    source: Arc<str>,
    lines: positions::LineIndex,
    completion: Option<completion::Document>,
    syntax_completion: Option<completion::Document>,
    completion_ready: bool,
    completion_requests: Vec<(RequestId, Position, Instant)>,
    signature_requests: Vec<(RequestId, Position, Instant)>,
    signature_capabilities: completion::SignatureCapabilities,
}
impl Document {
    fn queue_signature(
        &mut self,
        connection: &Connection,
        id: RequestId,
        position: Position,
        received: Instant,
        server_has_capacity: bool,
    ) -> Result<(), String> {
        if !server_has_capacity || self.signature_requests.len() >= 64 {
            send(
                connection,
                Response::new_err(
                    id,
                    ErrorCode::RequestFailed as i32,
                    "too many pending signature requests".into(),
                ),
            )
        } else {
            self.signature_requests.push((id, position, received));
            Ok(())
        }
    }
    fn send_signature(
        &self,
        connection: &Connection,
        id: RequestId,
        position: Position,
        wait_ms: f64,
    ) -> Result<(), String> {
        let started = Instant::now();
        let result = match &self.completion {
            Some(document) => document.signature_at(position, true)?,
            None => completion::SignatureQuery::Ready(None),
        };
        let completion::SignatureQuery::Ready(help) = result else {
            return Err("analyzed signature still needs references".into());
        };
        let help = self.signature_capabilities.adapt(help);
        let count = help.as_ref().map_or(0, |h| h.signatures.len());
        send(connection, Response::new_ok(id, help))?;
        log(
            connection,
            format!(
                "skript-lsp signature timing {}",
                json!({"queryMs":started.elapsed().as_secs_f64()*1000.0,"waitMs":wait_ms,"source":"analysis","signatures":count})
            ),
        )
    }
    fn try_syntax_signature(
        &mut self,
        connection: &Connection,
        id: &RequestId,
        position: Position,
        index: Arc<completion::CatalogIndex>,
    ) -> Result<bool, String> {
        let started = Instant::now();
        if self.syntax_completion.is_none() {
            self.syntax_completion =
                completion::Document::from_source(Arc::clone(&self.source), index).ok();
        }
        let Some(document) = &self.syntax_completion else {
            return Ok(false);
        };
        let completion::SignatureQuery::Ready(help) = document.signature_at(position, false)?
        else {
            return Ok(false);
        };
        let help = self.signature_capabilities.adapt(help);
        let count = help.as_ref().map_or(0, |h| h.signatures.len());
        send(connection, Response::new_ok(id.clone(), help))?;
        log(
            connection,
            format!(
                "skript-lsp signature timing {}",
                json!({"queryMs":started.elapsed().as_secs_f64()*1000.0,"waitMs":0,"source":"syntax","signatures":count})
            ),
        )?;
        Ok(true)
    }
    fn finish_signature_requests(&mut self, connection: &Connection) -> Result<(), String> {
        for (id, position, queued) in std::mem::take(&mut self.signature_requests) {
            self.send_signature(
                connection,
                id,
                position,
                queued.elapsed().as_secs_f64() * 1000.0,
            )?;
        }
        Ok(())
    }
    fn completion_response(&self, id: RequestId, position: Position, snippets: bool) -> Response {
        let result = match &self.completion {
            Some(document) => document.at(position, snippets),
            None => Ok(lsp_types::CompletionList {
                is_incomplete: false,
                items: Vec::new(),
            }),
        };
        match result {
            Ok(list) => Response::new_ok(id, list),
            Err(error) => Response::new_err(id, ErrorCode::InvalidParams as i32, error),
        }
    }
    fn finish_completion_requests(
        &mut self,
        connection: &Connection,
        snippets: bool,
    ) -> Result<(), String> {
        for (id, position, queued) in std::mem::take(&mut self.completion_requests) {
            self.send_completion(
                connection,
                id,
                position,
                snippets,
                queued.elapsed().as_secs_f64() * 1000.0,
            )?;
        }
        Ok(())
    }
    fn send_completion(
        &self,
        connection: &Connection,
        id: RequestId,
        position: Position,
        snippets: bool,
        wait_ms: f64,
    ) -> Result<(), String> {
        let started = Instant::now();
        let response = self.completion_response(id, position, snippets);
        let result = response.response_result.as_ref().ok();
        let items = result
            .and_then(|r| r["items"].as_array())
            .map_or(0, Vec::len);
        let incomplete = result.is_some_and(|r| r["isIncomplete"] == true);
        let query_ms = started.elapsed().as_secs_f64() * 1000.0;
        send(connection, response)?;
        log(
            connection,
            format!(
                "skript-lsp completion timing {}",
                json!({"queryMs":query_ms,"waitMs":wait_ms,"source":"analysis","items":items,"isIncomplete":incomplete})
            ),
        )
    }
    fn try_syntax_completion(
        &mut self,
        connection: &Connection,
        id: &RequestId,
        position: Position,
        snippets: bool,
        index: Arc<completion::CatalogIndex>,
    ) -> Result<bool, String> {
        let started = Instant::now();
        if self.syntax_completion.is_none() {
            self.syntax_completion =
                completion::Document::from_source(Arc::clone(&self.source), index).ok();
        }
        let Some(document) = &self.syntax_completion else {
            return Ok(false);
        };
        let mut list = document.syntax_at(position, snippets)?;
        let suppressed = document.suppresses_completion(position);
        if suppressed {
            list.is_incomplete = false;
        }
        // Reference history and Function shadowing still require the current parse.
        let has_syntax = list.items.iter().any(|item| {
            item.data.as_ref().is_some_and(|data| {
                matches!(
                    data["syntaxKind"].as_str(),
                    Some("Event" | "Structure" | "Effect" | "Section" | "Condition")
                )
            })
        });
        if !has_syntax && !suppressed {
            return Ok(false);
        }
        let count = list.items.len();
        let incomplete = list.is_incomplete;
        let query_ms = started.elapsed().as_secs_f64() * 1000.0;
        send(connection, Response::new_ok(id.clone(), list))?;
        log(
            connection,
            format!(
                "skript-lsp completion timing {}",
                json!({"queryMs":query_ms,"waitMs":0,"source":"syntax","items":count,"isIncomplete":incomplete})
            ),
        )?;
        Ok(true)
    }
    fn finish_token_requests(&mut self, connection: &Connection) -> Result<(), String> {
        for id in self.token_requests.drain(..) {
            send(
                connection,
                Response::new_ok(id, self.tokens.clone().unwrap_or_default()),
            )?;
        }
        Ok(())
    }
    fn cancel_requests(&mut self, connection: &Connection, code: ErrorCode) -> Result<(), String> {
        for (id, _, _) in self.signature_requests.drain(..) {
            send(
                connection,
                Response::new_err(
                    id,
                    code as i32,
                    "signature analysis is no longer available for this request".into(),
                ),
            )?;
        }
        for (id, _, _) in self.completion_requests.drain(..) {
            send(
                connection,
                Response::new_err(
                    id,
                    code as i32,
                    "completion analysis is no longer available for this request".into(),
                ),
            )?;
        }
        for id in self.token_requests.drain(..) {
            send(
                connection,
                Response::new_err(
                    id,
                    code as i32,
                    "document analysis is no longer available for this request".into(),
                ),
            )?;
        }
        Ok(())
    }
}
struct Documents {
    open: HashMap<Uri, Document>,
    next_id: u64,
    completion_index: Option<Arc<completion::CatalogIndex>>,
    signature_capabilities: completion::SignatureCapabilities,
}
impl Documents {
    fn new(signature_capabilities: completion::SignatureCapabilities) -> Self {
        Self {
            open: HashMap::new(),
            next_id: 0,
            completion_index: None,
            signature_capabilities,
        }
    }
    fn id(&mut self) -> Result<u64, String> {
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or("document revision exhausted")?;
        Ok(self.next_id)
    }
    fn cancel_all(&self) {
        for document in self.open.values() {
            document.control.cancel();
        }
    }
    fn current(&self, job: &Job) -> bool {
        self.open.get(&job.uri).is_some_and(|document| {
            document.generation == job.generation
                && document.revision == job.revision
                && document.version == job.version
        })
    }
    fn open(&mut self, params: DidOpenTextDocumentParams, worker: &Worker) -> Result<(), String> {
        let item = params.text_document;
        if self.open.contains_key(&item.uri) {
            return Err("didOpen received for an already open document".into());
        }
        let revision = self.id()?;
        let control = Control::new();
        let source: Arc<str> = Arc::from(item.text);
        let job = Job {
            uri: item.uri.clone(),
            generation: revision,
            revision,
            version: item.version,
            source: Arc::clone(&source),
            control: control.clone(),
            queued_at: Instant::now(),
        };
        self.open.insert(
            item.uri,
            Document {
                generation: revision,
                revision,
                version: item.version,
                control,
                hover: None,
                tokens: None,
                token_requests: Vec::new(),
                lines: positions::LineIndex::new(&source),
                source,
                completion: None,
                syntax_completion: None,
                completion_ready: false,
                completion_requests: Vec::new(),
                signature_requests: Vec::new(),
                signature_capabilities: self.signature_capabilities,
            },
        );
        worker.submit(job);
        Ok(())
    }
    fn change(
        &mut self,
        params: DidChangeTextDocumentParams,
        worker: &Worker,
        connection: &Connection,
    ) -> Result<(), String> {
        let uri = params.text_document.uri;
        let document = self
            .open
            .get(&uri)
            .ok_or("didChange received for a closed document")?;
        if params.text_document.version <= document.version {
            return Err("didChange version must increase within an open document".into());
        }
        if params.content_changes.is_empty()
            || params
                .content_changes
                .iter()
                .any(|change| change.range.is_some() || change.range_length.is_some())
        {
            return Err("only nonempty full-document contentChanges are supported".into());
        }
        let source = params.content_changes.into_iter().last().unwrap().text;
        let revision = self.id()?;
        let document = self.open.get_mut(&uri).unwrap();
        document.cancel_requests(connection, ErrorCode::ContentModified)?;
        document.control.cancel();
        document.control = Control::new();
        document.revision = revision;
        document.version = params.text_document.version;
        document.hover = None;
        document.tokens = None;
        document.completion = None;
        document.syntax_completion = None;
        document.completion_ready = false;
        document.source = Arc::from(source);
        document.lines = positions::LineIndex::new(&document.source);
        worker.submit(Job {
            uri,
            generation: document.generation,
            revision,
            version: document.version,
            source: Arc::clone(&document.source),
            control: document.control.clone(),
            queued_at: Instant::now(),
        });
        Ok(())
    }
    fn close(
        &mut self,
        params: DidCloseTextDocumentParams,
        worker: &Worker,
        connection: &Connection,
    ) -> Result<(), String> {
        let uri = params.text_document.uri;
        if let Some(mut document) = self.open.remove(&uri) {
            document.cancel_requests(connection, ErrorCode::ContentModified)?;
            document.control.cancel();
            worker.release(uri.as_str(), document.revision);
            publish(connection, uri, document.version, Vec::new())?;
        }
        Ok(())
    }
}

fn serve(connection: Connection, factory: Factory) -> Result<(), String> {
    let (initialize_id, params) = connection
        .initialize_start()
        .map_err(|error| error.to_string())?;
    let config = serde_json::from_value(params)
        .map_err(|error| error.to_string())
        .and_then(ProjectConfig::from_params);
    let config = match config {
        Ok(config) => config,
        Err(error) => {
            send(
                &connection,
                Response::new_err(
                    initialize_id,
                    ErrorCode::InvalidParams as i32,
                    error.clone(),
                ),
            )?;
            return Err(error);
        }
    };
    let semantic_legend = config.semantic_legend.clone();
    let completion_snippets = config.completion_snippets;
    let signature_capabilities = config.signature_capabilities;
    let mut worker = Worker::start(config, factory)?;
    let mut documents = Documents::new(signature_capabilities);
    let mut initialize_id = Some(initialize_id);
    let mut initialized = false;
    let mut shutdown = false;
    let result = (|| {
        loop {
            let events = if shutdown {
                crossbeam_channel::never()
            } else {
                worker.events.clone()
            };
            select! {
                recv(events) -> event => match event.map_err(|_| "parser worker disconnected")? {
                    Event::Ready(Ok(index), startup_ms) => {
                        documents.completion_index = index;
                        let mut capabilities = json!({"positionEncoding":"utf-16","textDocumentSync":{"openClose":true,"change":1},"hoverProvider":true,"completionProvider":{"resolveProvider":false,"triggerCharacters":["{"]},"signatureHelpProvider":{"triggerCharacters":["(",","," "],"retriggerCharacters":[")"]}});
                        if let Some(legend) = &semantic_legend {
                            capabilities["semanticTokensProvider"] = json!({"legend":legend.wire(),"full":true});
                        }
                        send(&connection, Response::new_ok(initialize_id.take().unwrap(), json!({
                            "capabilities":capabilities,
                            "serverInfo":{"name":"skript-lsp","version":env!("CARGO_PKG_VERSION")}
                        })))?;
                        log(&connection, format!("skript-lsp startup {{\"startupMs\":{startup_ms}}}"))?;
                    }
                    Event::Ready(Err(error), _) => {
                        send(&connection, Response::new_err(initialize_id.take().unwrap(), ErrorCode::InternalError as i32, format!("parser initialization failed: {error}")))?;
                        return Err(error);
                    }
                    Event::Finished(job, analysis, metrics) => {
                        if shutdown { continue; }
                        log(&connection, format!("skript-lsp timing {metrics}"))?;
                        if documents.current(&job) {
                            if let Some(error) = analysis.semantic_error {
                                log(&connection, format!("semantic tokens unavailable: {error}"))?;
                            }
                            if let Some(error) = analysis.completion_error { log(&connection, format!("completion unavailable: {error}"))?; }
                            let document = documents.open.get_mut(&job.uri).unwrap();
                            document.tokens = Some(SemanticTokens::default());
                            document.completion_ready = true;
                            if !job.control.token.is_cancelled() {
                                if let Outcome::Diagnostics(diagnostics, status) = analysis.outcome {
                                    if status != "fatal" {
                                        document.hover = analysis.hover;
                                        document.completion = analysis.completion;
                                        if let (Some(snapshot), Some(legend)) = (analysis.semantic_tokens, &semantic_legend) {
                                            document.tokens = Some(snapshot.full(legend));
                                        }
                                    }
                                    publish(&connection, job.uri, job.version, diagnostics)?;
                                }
                            }
                            document.finish_token_requests(&connection)?;
                            document.finish_completion_requests(&connection, completion_snippets)?;
                            document.finish_signature_requests(&connection)?;
                        }
                    }
                    Event::Failed(error) => {
                        for document in documents.open.values_mut() {
                            document.cancel_requests(&connection, ErrorCode::InternalError)?;
                        }
                        if let Some(id) = initialize_id.take() {
                            send(&connection, Response::new_err(id, ErrorCode::InternalError as i32, error.clone()))?;
                        } else if !shutdown {
                            for (uri, document) in &documents.open { publish(&connection, uri.clone(), document.version, vec![diagnostics::fatal("lsp.worker-failed", error.clone())])?; }
                        }
                        return Err(error);
                    }
                },
                recv(connection.receiver) -> message => {
                    match message.map_err(|_| "LSP client disconnected before exit")? {
                        Message::Request(request) => {
                            if request.method == "shutdown" && initialize_id.is_none() && !shutdown {
                                shutdown = true;
                                for document in documents.open.values_mut() {
                                    document.cancel_requests(&connection, ErrorCode::RequestCanceled)?;
                                }
                                documents.cancel_all();
                                worker.stop();
                                send(&connection, Response::new_ok(request.id, Value::Null))?;
                            } else if request.method == "textDocument/semanticTokens/full" && semantic_legend.is_some() && initialize_id.is_none() && initialized && !shutdown {
                                match serde_json::from_value::<SemanticTokensParams>(request.params) {
                                    Ok(params) => {
                                        if let Some(document) = documents.open.get_mut(&params.text_document.uri) {
                                            if let Some(tokens) = &document.tokens {
                                                send(&connection, Response::new_ok(request.id, tokens))?;
                                            } else {
                                                document.token_requests.push(request.id);
                                            }
                                        } else {
                                            send(&connection, Response::new_ok(request.id, SemanticTokens::default()))?;
                                        }
                                    }
                                    Err(error) => send(&connection, Response::new_err(request.id, ErrorCode::InvalidParams as i32, error.to_string()))?,
                                }
                            } else if request.method == "textDocument/completion" && initialize_id.is_none() && initialized && !shutdown {
                                let received = Instant::now();
                                match serde_json::from_value::<CompletionParams>(request.params) {
                                    Ok(params) => {
                                        let target = params.text_document_position;
                                         let syntax_index = documents.completion_index.clone();
                                        if let Some(document) = documents.open.get_mut(&target.text_document.uri) {
                                            if document.lines.offset(&document.source, target.position).is_none() {
                                                send(&connection, Response::new_err(request.id, ErrorCode::InvalidParams as i32, "invalid completion UTF-16 position".into()))?;
                                            } else if document.completion_ready {
                                                document.send_completion(&connection, request.id, target.position, completion_snippets, 0.0)?;
                                            } else if let Some(index) = syntax_index {
                                                if !document.try_syntax_completion(&connection, &request.id, target.position, completion_snippets, index)? {
                                                    document.completion_requests.push((request.id, target.position, received));
                                                }
                                            } else { document.completion_requests.push((request.id, target.position, received)); }
                                        } else { send(&connection, Response::new_ok(request.id, lsp_types::CompletionList { is_incomplete: false, items: Vec::new() }))?; }
                                    }
                                    Err(error) => send(&connection, Response::new_err(request.id, ErrorCode::InvalidParams as i32, error.to_string()))?,
                                }
                            } else if request.method == "textDocument/signatureHelp" && initialize_id.is_none() && initialized && !shutdown {
                                let received = Instant::now();
                                let has_capacity = documents.open.values().map(|d| d.signature_requests.len()).sum::<usize>() < 256;
                                match serde_json::from_value::<lsp_types::SignatureHelpParams>(request.params) {
                                    Ok(params) => {
                                        let target = params.text_document_position_params;
                                        let syntax_index = documents.completion_index.clone();
                                        if let Some(document) = documents.open.get_mut(&target.text_document.uri) {
                                            if document.lines.offset(&document.source, target.position).is_none() {
                                                send(&connection, Response::new_err(request.id, ErrorCode::InvalidParams as i32, "invalid signature UTF-16 position".into()))?;
                                            } else if document.completion_ready {
                                                document.send_signature(&connection, request.id, target.position, 0.0)?;
                                            } else if let Some(index) = syntax_index {
                                                if !document.try_syntax_signature(&connection, &request.id, target.position, index)? {
                                                    document.queue_signature(&connection, request.id, target.position, received, has_capacity)?;
                                                }
                                            } else { document.queue_signature(&connection, request.id, target.position, received, has_capacity)?; }
                                        } else { send(&connection, Response::new_ok(request.id, Value::Null))?; }
                                    }
                                    Err(error) => send(&connection, Response::new_err(request.id, ErrorCode::InvalidParams as i32, error.to_string()))?,
                                }
                            } else if request.method == "textDocument/hover" && initialize_id.is_none() && initialized && !shutdown {
                                let response = match serde_json::from_value::<HoverParams>(request.params) {
                                    Ok(params) => {
                                        let target = params.text_document_position_params;
                                        let result = documents.open.get(&target.text_document.uri).and_then(|document| document.hover.as_ref()).and_then(|document| document.at(target.position));
                                        Response::new_ok(request.id, result)
                                    }
                                    Err(error) => Response::new_err(request.id, ErrorCode::InvalidParams as i32, error.to_string()),
                                };
                                send(&connection, response)?;
                            } else {
                                let code = if initialize_id.is_some() { ErrorCode::ServerNotInitialized } else if shutdown { ErrorCode::InvalidRequest } else { ErrorCode::MethodNotFound };
                                send(&connection, Response::new_err(request.id, code as i32, "request is not supported in the current server state".into()))?;
                            }
                        }
                        Message::Notification(notification) if notification.method == "exit" => return if shutdown { Ok(()) } else { Err("exit received before shutdown".into()) },
                        Message::Notification(notification) => {
                            if notification.method == "initialized" && initialize_id.is_none() { initialized = true; continue; }
                            if !initialized || shutdown { continue; }
                            let result = match notification.method.as_str() {
                                "textDocument/didOpen" => serde_json::from_value(notification.params).map_err(|error| error.to_string()).and_then(|params| documents.open(params, &worker)),
                                "textDocument/didChange" => serde_json::from_value(notification.params).map_err(|error| error.to_string()).and_then(|params| documents.change(params, &worker, &connection)),
                                "textDocument/didClose" => serde_json::from_value(notification.params).map_err(|error| error.to_string()).and_then(|params| documents.close(params, &worker, &connection)),
                                "$/cancelRequest" => {
                                    if let Ok(id) = serde_json::from_value::<RequestId>(notification.params["id"].clone()) {
                                        for document in documents.open.values_mut() {
                                            if let Some(index) = document.signature_requests.iter().position(|(pending, _, _)| *pending == id) {
                                                document.signature_requests.remove(index);
                                                send(&connection, Response::new_err(id, ErrorCode::RequestCanceled as i32, "signature request cancelled".into()))?;
                                                break;
                                            }
                                            if let Some(index) = document.completion_requests.iter().position(|(pending, _, _)| *pending == id) {
                                                document.completion_requests.remove(index);
                                                send(&connection, Response::new_err(id, ErrorCode::RequestCanceled as i32, "completion request cancelled".into()))?;
                                                break;
                                            }
                                            if let Some(index) = document.token_requests.iter().position(|pending| *pending == id) {
                                                document.token_requests.remove(index);
                                                send(&connection, Response::new_err(id, ErrorCode::RequestCanceled as i32, "semantic token request cancelled".into()))?;
                                                break;
                                            }
                                        }
                                    }
                                    Ok(())
                                }
                                _ => Ok(()),
                            };
                            if let Err(error) = result { log(&connection, error)?; }
                        }
                        Message::Response(_) => {}
                    }
                }
            }
        }
    })();
    documents.cancel_all();
    let stopped = worker.join();
    result.and(stopped)
}

#[cfg(test)]
mod tests;

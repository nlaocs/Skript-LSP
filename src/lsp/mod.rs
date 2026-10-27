//! A single-project, full-document LSP frontend for the shared parser service.

mod annotations;
mod call_hierarchy;
mod completion;
mod diagnostics;
mod hover;
mod hover_text;
mod inlay;
mod positions;
mod preparation;
mod project;
mod project_sessions;
mod public_data;
mod rename;
mod semantic;
mod semantic_tokens;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod storage_tests;
mod type_display;
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
    project_root: PathBuf,
    watch_files: bool,
    inlay_refresh: bool,
    semantic_refresh: bool,
    rename_document_changes: bool,
    hierarchical_symbols: bool,
    extended_symbol_kinds: bool,
    snapshot_path: PathBuf,
    addon_paths: Vec<PathBuf>,
    addon_data_sources: Vec<AddonDataConfig>,
    hover_format: MarkupKind,
    completion_format: MarkupKind,
    semantic_legend: Option<semantic_tokens::Legend>,
    completion_snippets: bool,
    completion_refresh: bool,
    signature_capabilities: completion::SignatureCapabilities,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InitializationOptions {
    snapshot_path: PathBuf,
    #[serde(default)]
    addon_paths: Vec<PathBuf>,
    #[serde(default)]
    addon_data_sources: Vec<AddonDataConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddonDataConfig {
    path: PathBuf,
    pin: parser_wasm::host::AddonDataPin,
}

impl ProjectConfig {
    #[allow(deprecated)] // rootUri is still the explicit single-project client contract.
    fn from_params(params: InitializeParams) -> Result<Self, String> {
        let extended_symbol_kinds = params
            .capabilities
            .text_document
            .as_ref()
            .and_then(|text| text.document_symbol.as_ref())
            .and_then(|symbols| symbols.symbol_kind.as_ref())
            .is_some();
        let hierarchical_symbols = params
            .capabilities
            .text_document
            .as_ref()
            .and_then(|text| text.document_symbol.as_ref())
            .and_then(|symbols| symbols.hierarchical_document_symbol_support)
            .unwrap_or(false);
        let rename_document_changes = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.workspace_edit.as_ref())
            .and_then(|edit| edit.document_changes)
            .unwrap_or(false);
        let inlay_refresh = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.inlay_hint.as_ref())
            .and_then(|capability| capability.refresh_support)
            .unwrap_or(false);
        let semantic_refresh = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.semantic_tokens.as_ref())
            .and_then(|capability| capability.refresh_support)
            .unwrap_or(false);
        let watch_files = params
            .capabilities
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.did_change_watched_files.as_ref())
            .and_then(|watch| watch.dynamic_registration)
            .unwrap_or(false);
        let completion_refresh = params
            .capabilities
            .experimental
            .as_ref()
            .is_some_and(|value| value["skriptCompletionRefresh"] == true);
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
        let completion_format = params
            .capabilities
            .text_document
            .as_ref()
            .and_then(|document| document.completion.as_ref())
            .and_then(|completion| completion.completion_item.as_ref())
            .and_then(|item| item.documentation_format.as_ref())
            .and_then(|formats| {
                formats
                    .iter()
                    .find(|format| matches!(**format, MarkupKind::Markdown | MarkupKind::PlainText))
            })
            .cloned()
            .unwrap_or(MarkupKind::PlainText);
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
            project_root: root_path.clone(),
            watch_files,
            inlay_refresh,
            semantic_refresh,
            rename_document_changes,
            hierarchical_symbols,
            extended_symbol_kinds,
            snapshot_path: absolute(options.snapshot_path),
            addon_paths: options.addon_paths.into_iter().map(absolute).collect(),
            addon_data_sources: options.addon_data_sources.into_iter().map(|mut source| {
                source.path = absolute(source.path); source
            }).collect(),
            hover_format,
            completion_format,
            semantic_legend,
            completion_snippets,
            completion_refresh,
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

struct CompletionTicket {
    ticket: String,
    position: Position,
    owner: Option<completion::OwnerAttempt>,
}
struct Document {
    source_epoch: u64,
    generation: u64,
    revision: u64,
    version: i32,
    control: Control,
    hover: Option<hover::Document>,
    semantic: Option<Arc<semantic::Document>>,
    inlay: Option<Arc<inlay::Document>>,
    annotation_requests: Vec<annotations::Pending>,
    tokens: Option<SemanticTokens>,
    token_requests: Vec<RequestId>,
    source: Arc<str>,
    lines: positions::LineIndex,
    completion: Option<completion::Document>,
    syntax_completion: Option<completion::Document>,
    prepared_completion: Option<Box<completion::PreparedDocument>>,
    prepared_state: Option<preparation::State>,
    prepared_ticket: Option<CompletionTicket>,
    next_completion_ticket: u64,
    completion_ready: bool,
    completion_requests: Vec<(RequestId, Position, Instant)>,
    signature_requests: Vec<(RequestId, Position, Instant)>,
    signature_capabilities: completion::SignatureCapabilities,
}
impl Document {
    fn try_prepared_completion(
        &mut self,
        connection: &Connection,
        id: &RequestId,
        position: Position,
        snippets: bool,
        wait_ms: f64,
    ) -> Result<bool, String> {
        let Some(document) = &self.prepared_completion else {
            return Ok(false);
        };
        let started = Instant::now();
        let completion::PreparedQuery::Ready(mut list) = document.at(position, snippets)? else {
            return Ok(false);
        };
        self.next_completion_ticket = self
            .next_completion_ticket
            .checked_add(1)
            .ok_or("completion ticket exhausted")?;
        let ticket = format!(
            "{}:{}:{}",
            self.generation, self.revision, self.next_completion_ticket
        );
        let owner = document.owner_at(position).cloned();
        for item in &mut list.items {
            if let Some(data) = item.data.as_mut().and_then(Value::as_object_mut) {
                data.insert("completionTicket".into(), json!(ticket));
            }
        }
        self.prepared_ticket = (!list.items.is_empty()).then_some(CompletionTicket {
            ticket,
            position,
            owner,
        });
        let count = list.items.len();
        send(connection, Response::new_ok(id.clone(), list))?;
        log(
            connection,
            format!(
                "skript-lsp completion timing {}",
                json!({"queryMs":started.elapsed().as_secs_f64()*1000.0,"waitMs":wait_ms,"source":"prepared","items":count,"isIncomplete":true})
            ),
        )?;
        Ok(true)
    }
    fn prepare_completion_requests(
        &mut self,
        connection: &Connection,
        snippets: bool,
    ) -> Result<(), String> {
        for (id, position, queued) in std::mem::take(&mut self.completion_requests) {
            if !self.try_prepared_completion(
                connection,
                &id,
                position,
                snippets,
                queued.elapsed().as_secs_f64() * 1000.0,
            )? {
                self.completion_requests.push((id, position, queued));
            }
        }
        Ok(())
    }
    fn refresh_completion(
        &mut self,
        connection: &Connection,
        uri: &Uri,
        phase: &str,
    ) -> Result<(), String> {
        if let Some(ticket) = self.prepared_ticket.take() {
            send(
                connection,
                Notification::new(
                    "skript/completionRefresh".into(),
                    json!({
                        "uri":uri,"version":self.version,"generation":self.generation,"revision":self.revision,
                        "ticket":ticket.ticket,"position":ticket.position,"phase":phase
                    }),
                ),
            )?;
        }
        Ok(())
    }

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
        self.cancel_annotations(connection, code)?;
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
    hierarchical_symbols: bool,
    extended_symbol_kinds: bool,
    hierarchy_session: String,
    project_root: Option<PathBuf>,
    project_epoch: u64,
    semantic_project: Vec<Arc<semantic::Document>>,
    hover_format: MarkupKind,
    open: HashMap<Uri, Document>,
    next_id: u64,
    completion_index: Option<Arc<completion::CatalogIndex>>,
    signature_capabilities: completion::SignatureCapabilities,
}
impl Documents {
    fn new(signature_capabilities: completion::SignatureCapabilities) -> Self {
        Self {
            hierarchical_symbols: false,
            extended_symbol_kinds: false,
            hierarchy_session: format!(
                "{}:{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ),
            project_root: None,
            project_epoch: 0,
            semantic_project: Vec::new(),
            hover_format: MarkupKind::PlainText,
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
        job.project
            .as_ref()
            .is_none_or(|snapshot| snapshot.epoch == self.project_epoch)
            && self.open.get(&job.uri).is_some_and(|document| {
                document.generation == job.generation
                    && document.revision == job.revision
                    && document.version == job.version
            })
    }
    fn open(
        &mut self,
        params: DidOpenTextDocumentParams,
        worker: &Worker,
        connection: &Connection,
    ) -> Result<(), String> {
        let item = params.text_document;
        if self.open.contains_key(&item.uri) {
            return Err("didOpen received for an already open document".into());
        }
        let revision = self.id()?;
        let control = Control::new();
        let source: Arc<str> = Arc::from(item.text);
        let focused = item.uri.clone();
        let job = Job {
            project: None,
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
                source_epoch: revision,
                generation: revision,
                revision,
                version: item.version,
                control,
                hover: None,
                semantic: None,
                inlay: None,
                annotation_requests: Vec::new(),
                tokens: None,
                token_requests: Vec::new(),
                lines: positions::LineIndex::new(&source),
                source,
                completion: None,
                syntax_completion: None,
                prepared_completion: None,
                prepared_state: None,
                prepared_ticket: None,
                next_completion_ticket: 0,
                completion_ready: false,
                completion_requests: Vec::new(),
                signature_requests: Vec::new(),
                signature_capabilities: self.signature_capabilities,
            },
        );
        if self.project_root.is_some() {
            self.restart_project(worker, connection, Some(&focused))?;
        } else {
            worker.submit(job);
        }
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
        document.source_epoch = revision;
        document.version = params.text_document.version;
        document.hover = None;
        document.semantic = None;
        document.inlay = None;
        document.tokens = None;
        document.completion = None;
        document.syntax_completion = None;
        document.prepared_completion = None;
        document.prepared_state = None;
        document.prepared_ticket = None;
        document.completion_ready = false;
        document.source = Arc::from(source);
        document.lines = positions::LineIndex::new(&document.source);
        if self.project_root.is_some() {
            return self.restart_project(worker, connection, Some(&uri));
        }
        worker.submit(Job {
            project: None,
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
            self.restart_project(worker, connection, None)?;
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
    let completion_refresh = config.completion_refresh;
    let signature_capabilities = config.signature_capabilities;
    let hover_format = config.hover_format.clone();
    let project_root = config.project_root.clone();
    let watch_files = config.watch_files;
    let inlay_refresh = config.inlay_refresh;
    let rename_document_changes = config.rename_document_changes;
    let hierarchical_symbols = config.hierarchical_symbols;
    let extended_symbol_kinds = config.extended_symbol_kinds;
    let semantic_refresh = config.semantic_refresh;
    let mut last_refresh_epoch = 0;
    let mut worker = Worker::start(config, factory)?;
    let mut documents = Documents::new(signature_capabilities);
    documents.hover_format = hover_format;
    documents.hierarchical_symbols = hierarchical_symbols;
    documents.extended_symbol_kinds = extended_symbol_kinds;
    documents.project_root = Some(project_root);
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
                        let mut capabilities = json!({"positionEncoding":"utf-16","textDocumentSync":{"openClose":true,"change":1},"hoverProvider":true,"definitionProvider":true,"referencesProvider":true,"callHierarchyProvider":true,"documentSymbolProvider":true,"inlayHintProvider":{"resolveProvider":false},"completionProvider":{"resolveProvider":false,"triggerCharacters":["{"]},"signatureHelpProvider":{"triggerCharacters":["(",","," "],"retriggerCharacters":[")"]}});
                        if completion_refresh { capabilities["experimental"] = json!({"skriptCompletionRefresh":true}); }
                        if rename_document_changes { capabilities["renameProvider"] = json!({"prepareProvider":true}); }
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
                    Event::Preparation(job, preparation, prepare_ms) => {
                        if shutdown { continue; }
                        let uri=preparation.source.uri.clone();
                        let native_revision=preparation.native_revision;
                        let phase=match &preparation.update {
                            worker::PreparationUpdate::Started=>"started",
                            worker::PreparationUpdate::Ready(_)=>"headers",
                            worker::PreparationUpdate::Structure(..)=>"structure",
                            worker::PreparationUpdate::Section(_)=>"section",
                            worker::PreparationUpdate::Invalidated(_)=>"invalidated",
                            worker::PreparationUpdate::Aborted=>"aborted",
                            worker::PreparationUpdate::Unavailable(_)=>"unavailable",
                        };
                        let unavailable=match &preparation.update { worker::PreparationUpdate::Unavailable(error)=>Some(error.clone()), _=>None };
                        if documents.prepare_completion(&job, preparation, &connection, completion_snippets)? {
                            if let Some(error)=unavailable { log(&connection,format!("prepared completion unavailable: {error}"))?; }
                            let document=&documents.open[&uri];
                            log(&connection, format!("skript-lsp preparation timing {}", json!({"uri":uri,"version":document.version,"revision":document.revision,"nativeRevision":native_revision,"phase":phase,"prepareMs":prepare_ms})))?;
                        }
                    }
                    Event::Finished(job, analysis, metrics) => {
                        if shutdown { continue; }
                        log(&connection, format!("skript-lsp timing {metrics}"))?;
                        if documents.current(&job) {
                            if let Some(error) = analysis.semantic_error {
                                log(&connection, format!("semantic tokens unavailable: {error}"))?;
                            }
                            if let Some(error) = analysis.completion_error { log(&connection, format!("completion unavailable: {error}"))?; }
                            if let Some(error) = analysis.annotations_error { log(&connection, format!("semantic annotations unavailable: {error}"))?; }
                            documents.semantic_project = analysis.project_semantics;
                            let hierarchy_stamp = documents.hierarchy_stamp();
                            let document = documents.open.get_mut(&job.uri).unwrap();
                            document.tokens = Some(SemanticTokens::default());
                            document.completion_ready = true;
                            document.prepared_completion = None;
                            document.prepared_state = None;
                            if !job.control.token.is_cancelled() {
                                if let Outcome::Diagnostics(diagnostics, status) = analysis.outcome {
                                    if status != "fatal" {
                                        document.hover = analysis.hover;
                                        document.semantic = analysis.semantic;
                                        document.inlay = analysis.inlay;
                                        document.completion = analysis.completion;
                                        if let (Some(snapshot), Some(legend)) = (analysis.semantic_tokens, &semantic_legend) {
                                            document.tokens = Some(snapshot.full(legend));
                                        }
                                    }
                                    publish(&connection, job.uri.clone(), job.version, diagnostics)?;
                                }
                            }
                            document.finish_annotations(&connection, &documents.semantic_project, &hierarchy_stamp)?;
                            document.finish_token_requests(&connection)?;
                            document.finish_completion_requests(&connection, completion_snippets)?;
                            document.finish_signature_requests(&connection)?;
                            document.refresh_completion(&connection, &job.uri, "final")?;
                            if documents.project_epoch > last_refresh_epoch {
                                last_refresh_epoch = documents.project_epoch;
                                for (supported, method, prefix) in [(inlay_refresh, "workspace/inlayHint/refresh", "skript-inlay-refresh"), (semantic_refresh, "workspace/semanticTokens/refresh", "skript-semantic-refresh")] {
                                    if supported { send(&connection, lsp_server::Request::new(RequestId::from(format!("{prefix}:{}", job.revision)), method.into(), Value::Null))?; }
                                }
                            }
                        }
                    }
                    Event::Failed(error) => {
                        for (uri, document) in &mut documents.open {
                            document.prepared_completion = None;
                            document.refresh_completion(&connection, uri, "failed")?;
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
                                            } else if document.try_prepared_completion(&connection, &request.id, target.position, completion_snippets, 0.0)? {
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
                            } else if matches!(request.method.as_str(), "textDocument/prepareRename" | "textDocument/rename") && initialize_id.is_none() && initialized && !shutdown {
                                documents.rename_request(&worker, &connection, request.id, &request.method, request.params, rename_document_changes)?;
                            } else if matches!(request.method.as_str(), "callHierarchy/incomingCalls" | "callHierarchy/outgoingCalls") && initialize_id.is_none() && initialized && !shutdown {
                                documents.hierarchy_request(&connection, request.id, &request.method, request.params)?;
                            } else if matches!(request.method.as_str(), "textDocument/definition" | "textDocument/references" | "textDocument/inlayHint" | "textDocument/prepareCallHierarchy" | "textDocument/documentSymbol") && initialize_id.is_none() && initialized && !shutdown {
                                documents.annotation_request(&connection, request.id, &request.method, request.params)?;
                            } else if request.method == "textDocument/hover" && initialize_id.is_none() && initialized && !shutdown {
                                let response = match serde_json::from_value::<HoverParams>(request.params) {
                                    Ok(params) => {
                                        let target = params.text_document_position_params;
                                        let result = documents.open.get(&target.text_document.uri).and_then(|document| document.semantic_hover(target.position, &documents.hover_format));
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
                            if notification.method == "initialized" && initialize_id.is_none() {
                                initialized = true;
                                if watch_files {
                                    send(&connection, lsp_server::Request::new(RequestId::from("skript-project-watch".to_owned()), "client/registerCapability".into(), json!({"registrations":[{"id":"skript-project-files","method":"workspace/didChangeWatchedFiles","registerOptions":{"watchers":[{"globPattern":"**/*.sk","kind":7}]}}]})))?;
                                }
                                continue;
                            }
                            if !initialized || shutdown { continue; }
                            let result = match notification.method.as_str() {
                                "textDocument/didOpen" => serde_json::from_value(notification.params).map_err(|error| error.to_string()).and_then(|params| documents.open(params, &worker, &connection)),
                                "textDocument/didChange" => serde_json::from_value(notification.params).map_err(|error| error.to_string()).and_then(|params| documents.change(params, &worker, &connection)),
                                "textDocument/didClose" => serde_json::from_value(notification.params).map_err(|error| error.to_string()).and_then(|params| documents.close(params, &worker, &connection)),
                                "workspace/didChangeWatchedFiles" => serde_json::from_value::<lsp_types::DidChangeWatchedFilesParams>(notification.params).map_err(|error| error.to_string()).and_then(|params| if params.changes.is_empty() { Ok(()) } else { documents.restart_project(&worker, &connection, None) }),
                                "$/cancelRequest" => {
                                    if let Ok(id) = serde_json::from_value::<RequestId>(notification.params["id"].clone()) {
                                        for document in documents.open.values_mut() {
                                            if document.cancel_annotation(&connection, &id)? { break; }
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
                        Message::Response(response) => {
                            if response.id == RequestId::from("skript-project-watch".to_owned()) && response.response_result.is_err() {
                                log(&connection, "project file watcher registration failed; closed-file changes require a subsequent open-document change or explicit workspace/didChangeWatchedFiles notification")?;
                            }
                        }
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

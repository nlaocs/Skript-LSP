use super::{ProjectConfig, diagnostics, hover};
use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use lsp_types::Diagnostic;
use parser_wasm::{
    DocumentCancellationToken, DocumentParseError, DocumentParseRequest, DocumentParserConfig,
    HostConfig, ParserHost,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, VecDeque},
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex},
    thread,
    time::Instant,
};

#[derive(Clone, Debug)]
pub(super) struct Control {
    pub token: DocumentCancellationToken,
    cancelled_at: Arc<Mutex<Option<Instant>>>,
}
impl Control {
    pub fn new() -> Self {
        Self {
            token: DocumentCancellationToken::new(),
            cancelled_at: Arc::new(Mutex::new(None)),
        }
    }
    pub fn cancel(&self) {
        self.cancelled_at
            .lock()
            .unwrap()
            .get_or_insert_with(Instant::now);
        self.token.cancel();
    }
    fn cancellation_ms(&self) -> Option<f64> {
        self.cancelled_at
            .lock()
            .unwrap()
            .map(|at| at.elapsed().as_secs_f64() * 1000.0)
    }
}
#[derive(Clone)]
pub(super) struct Job {
    pub uri: lsp_types::Uri,
    pub generation: u64,
    pub revision: u64,
    pub version: i32,
    pub source: Arc<str>,
    pub control: Control,
    pub queued_at: Instant,
}
pub(super) enum Outcome {
    Diagnostics(Vec<Diagnostic>, &'static str),
    Cancelled,
    Stale,
}
pub(super) struct Analysis {
    pub outcome: Outcome,
    pub hover: Option<hover::Document>,
    pub parse_ms: f64,
    pub report_ms: f64,
}
pub(super) trait Analyzer {
    fn analyze(&mut self, job: &Job) -> Analysis;
    fn release(&mut self, uri: &str, through_revision: u64) -> Result<(), String>;
}
pub(super) type Factory =
    Box<dyn FnOnce(ProjectConfig) -> Result<Box<dyn Analyzer>, String> + Send>;
struct ParserAnalyzer {
    project_uri: String,
    host: ParserHost,
    catalog: Arc<syntaxes::Catalog>,
}
pub(super) fn parser_factory() -> Factory {
    Box::new(|config| {
        let snapshot =
            ssg::load_for_parsing(&config.snapshot_path).map_err(|error| error.to_string())?;
        let catalog = Arc::new(snapshot.into_catalog());
        let mut host = crate::new_parser_host(HostConfig {
            syntax_catalog: Some(Arc::clone(&catalog)),
            ..HostConfig::default()
        })
        .map_err(|error| error.to_string())?;
        for path in config.addon_paths {
            let bytes = std::fs::read(&path)
                .map_err(|error| format!("cannot read addon {}: {error}", path.display()))?;
            host.load_addon(&bytes).map_err(|error| error.to_string())?;
        }
        // Validate the project's StateStore identity during initialization,
        // before any document can queue an unreleaseable parse.
        host.begin_parse(&config.project_uri, "lsp-project-validation", 0)
            .and_then(|parse| parse.cancel().map_err(Into::into))
            .map_err(|error| error.to_string())?;
        host.release_document(&config.project_uri, "lsp-project-validation", 0)
            .map_err(|error| error.to_string())?;
        Ok(Box::new(ParserAnalyzer {
            project_uri: config.project_uri,
            host,
            catalog,
        }))
    })
}
impl Analyzer for ParserAnalyzer {
    fn analyze(&mut self, job: &Job) -> Analysis {
        let started = Instant::now();
        let result = self.host.parse_document(
            DocumentParseRequest::new(
                &self.project_uri,
                document_key(job.uri.as_str()),
                job.revision,
                Arc::clone(&job.source),
            ),
            DocumentParserConfig {
                cancellation: job.control.token.clone(),
                ..DocumentParserConfig::default()
            },
        );
        let parse_ms = started.elapsed().as_secs_f64() * 1000.0;
        let report_started = Instant::now();
        let mut hover = None;
        let outcome = match result {
            Ok(result) => {
                let status =
                    if result.execution_status == parser_wasm::DocumentExecutionStatus::Partial {
                        "partial"
                    } else {
                        "complete"
                    };
                // Conversion and recursive destruction stay on the worker's large stack.
                match diagnostics::from_result(&job.uri, &result) {
                    Ok(mut diagnostics) => match hover::Document::from_result(
                        result,
                        Arc::clone(&job.source),
                        Arc::clone(&self.catalog),
                    ) {
                        Ok(document) => {
                            hover = Some(document);
                            Outcome::Diagnostics(diagnostics, status)
                        }
                        Err(error) => {
                            diagnostics.push(diagnostics::fatal("lsp.source-mapping", error));
                            Outcome::Diagnostics(diagnostics, "fatal")
                        }
                    },
                    Err(error) => Outcome::Diagnostics(
                        vec![diagnostics::fatal("lsp.source-mapping", error)],
                        "fatal",
                    ),
                }
            }
            Err(DocumentParseError::Cancelled { .. }) => Outcome::Cancelled,
            Err(DocumentParseError::StaleRevision { .. }) => Outcome::Stale,
            Err(error) => Outcome::Diagnostics(
                vec![diagnostics::fatal("lsp.analysis-failed", error.to_string())],
                "fatal",
            ),
        };
        Analysis {
            outcome,
            hover,
            parse_ms,
            report_ms: report_started.elapsed().as_secs_f64() * 1000.0,
        }
    }
    fn release(&mut self, uri: &str, revision: u64) -> Result<(), String> {
        self.host
            .release_document(&self.project_uri, &document_key(uri), revision)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

fn document_key(uri: &str) -> String {
    // URI percent encoding can exceed host identifier quotas for ordinary
    // Unicode filenames. The host accepts opaque IDs; protocol URIs stay intact.
    format!("lsp-document:{:x}", Sha256::digest(uri.as_bytes()))
}
pub(super) enum Event {
    Ready(Result<(), String>, f64),
    Finished(Job, Analysis, Value),
    Failed(String),
}
#[derive(Default)]
struct Pending {
    release_through: Option<u64>,
    job: Option<Job>,
}
#[derive(Default)]
struct Queue {
    documents: BTreeMap<String, Pending>,
    order: VecDeque<String>,
    stopped: bool,
}
impl Queue {
    fn pending(&mut self, uri: &str) -> &mut Pending {
        if !self.documents.contains_key(uri) {
            self.order.push_back(uri.to_owned());
        }
        self.documents.entry(uri.to_owned()).or_default()
    }
    fn pop(&mut self) -> Option<(String, Pending)> {
        let uri = self.order.pop_front()?;
        let pending = self
            .documents
            .remove(&uri)
            .expect("queued URI has pending work");
        Some((uri, pending))
    }
}
pub(super) struct Worker {
    queue: Arc<Mutex<Queue>>,
    wake: Sender<()>,
    pub events: Receiver<Event>,
    handle: Option<thread::JoinHandle<()>>,
}
impl Worker {
    pub fn start(config: ProjectConfig, factory: Factory) -> Result<Self, String> {
        let queue = Arc::new(Mutex::new(Queue::default()));
        let (wake, wake_receiver) = bounded(1);
        let (sender, events) = unbounded();
        let thread_queue = Arc::clone(&queue);
        let handle = thread::Builder::new().name("skript-lsp-parser".into()).stack_size(32 * 1024 * 1024).spawn(move || {
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                let started = Instant::now();
                let analyzer = factory(config);
                let startup_ms = started.elapsed().as_secs_f64() * 1000.0;
                let mut analyzer = match analyzer {
                    Ok(analyzer) => { let _ = sender.send(Event::Ready(Ok(()), startup_ms)); analyzer }
                    Err(error) => { let _ = sender.send(Event::Ready(Err(error), startup_ms)); return; }
                };
                while wake_receiver.recv().is_ok() {
                    loop {
                        let pending = {
                            let mut queue = thread_queue.lock().unwrap();
                            if queue.stopped { return; }
                            queue.pop()
                        };
                        let Some((uri, pending)) = pending else { break; };
                        if let Some(revision) = pending.release_through {
                            if let Err(error) = analyzer.release(&uri, revision) {
                                let _ = sender.send(Event::Failed(format!("document release failed: {error}")));
                                return;
                            }
                        }
                        let Some(job) = pending.job else { continue; };
                        if job.control.token.is_cancelled() { continue; }
                        let queue_ms = job.queued_at.elapsed().as_secs_f64() * 1000.0;
                        let started = Instant::now();
                        let analysis = analyzer.analyze(&job);
                        let analysis_ms = started.elapsed().as_secs_f64() * 1000.0;
                        let status = match &analysis.outcome { Outcome::Diagnostics(_, status) => *status, Outcome::Cancelled => "cancelled", Outcome::Stale => "stale" };
                        let metrics = json!({"uri":job.uri.as_str(), "revision":job.revision, "status":status, "queueMs":queue_ms, "parseMs":analysis.parse_ms, "reportMs":analysis.report_ms, "analysisMs":analysis_ms, "cancelToFinishMs":job.control.cancellation_ms()});
                        if sender.send(Event::Finished(job, analysis, metrics)).is_err() { return; }
                    }
                }
            }));
            if result.is_err() { let _ = sender.send(Event::Failed("parser worker panicked; no further analysis is safe".into())); }
        }).map_err(|error| error.to_string())?;
        Ok(Self {
            queue,
            wake,
            events,
            handle: Some(handle),
        })
    }
    pub fn submit(&self, job: Job) {
        let uri = job.uri.as_str().to_owned();
        self.queue.lock().unwrap().pending(&uri).job = Some(job);
        let _ = self.wake.try_send(());
    }
    pub fn release(&self, uri: &str, revision: u64) {
        let mut queue = self.queue.lock().unwrap();
        let pending = queue.pending(uri);
        pending.job = None;
        pending.release_through = Some(pending.release_through.unwrap_or(0).max(revision));
        drop(queue);
        let _ = self.wake.try_send(());
    }
    pub fn stop(&mut self) {
        {
            let mut queue = self.queue.lock().unwrap();
            queue.stopped = true;
            queue.documents.clear();
            queue.order.clear();
        }
        let _ = self.wake.try_send(());
    }
    pub fn join(&mut self) -> Result<(), String> {
        self.stop();
        self.handle.take().map_or(Ok(()), |handle| {
            handle.join().map_err(|_| "parser worker panicked".into())
        })
    }
}

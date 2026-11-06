#[cfg(test)]
mod equivalence_tests;

use super::{ProjectConfig, completion, diagnostics, hover, inlay, semantic, semantic_tokens};
use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use lsp_types::Diagnostic;
use parser_wasm::{
    DocumentCancellationToken, DocumentParseRequest, DocumentParserConfig, HostConfig, ParserHost,
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
    pub project: Option<Arc<super::project::ProjectSnapshot>>,
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
    pub semantic: Option<Arc<semantic::Document>>,
    pub inlay: Option<Arc<inlay::Document>>,
    pub annotations_error: Option<String>,
    pub project_semantics: Vec<Arc<semantic::Document>>,
    pub semantic_tokens: Option<semantic_tokens::Document>,
    pub semantic_error: Option<String>,
    pub completion: Option<completion::Document>,
    pub completion_error: Option<String>,
    pub analysis_metrics: Value,
    pub parse_ms: f64,
    pub report_ms: f64,
}
pub(super) trait Analyzer {
    fn completion_index(&self) -> Option<Arc<completion::CatalogIndex>> {
        None
    }
    fn analyze(&mut self, job: &Job) -> Analysis;
    fn analyze_with_preparation(
        &mut self,
        job: &Job,
        _publish: &mut dyn FnMut(Preparation),
    ) -> Analysis {
        self.analyze(job)
    }
    fn release(&mut self, uri: &str, through_revision: u64) -> Result<(), String>;
}
pub(super) type Factory =
    Box<dyn FnOnce(ProjectConfig) -> Result<Box<dyn Analyzer>, String> + Send>;
struct ParserAnalyzer {
    project_uri: String,
    host: ParserHost,
    catalog: Arc<syntaxes::Catalog>,
    hover_format: lsp_types::MarkupKind,
    semantic_enabled: bool,
    completions: Arc<completion::CatalogIndex>,
    prepare_completion: bool,
    project: super::project::ProjectIndex,
}
pub(super) fn parser_factory() -> Factory {
    Box::new(|config| {
        let snapshot =
            ssg::load_for_parsing(&config.snapshot_path).map_err(|error| error.to_string())?;
        let catalog = Arc::new(snapshot.into_catalog());
        if config.addon_data_sources.len()>64 {return Err("too many addon dataset sources".into());}
        let mut remaining=16 * 1024 * 1024;
        let mut addon_data_sources=Vec::new();
        for source in config.addon_data_sources {
            let captured=parser_wasm::host::AddonDataSource::from_file_with_budget(source.pin,source.path,remaining)?;
            remaining-=captured.byte_length();
            addon_data_sources.push(captured);
        }
        let mut host = crate::new_parser_host(HostConfig {
            syntax_catalog: Some(Arc::clone(&catalog)),
            addon_data_sources,
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
        let mut completions = completion::CatalogIndex::with_documentation_format(
            &catalog,
            host.raw_tree_options(),
            config.completion_format.clone(),
        );
        completions.certify_types(&host);
        let completions = Arc::new(completions);
        Ok(Box::new(ParserAnalyzer {
            project_uri: config.project_uri,
            host,
            catalog,
            hover_format: config.hover_format,
            semantic_enabled: config.semantic_legend.is_some(),
            completions,
            prepare_completion: config.completion_refresh,
            project: super::project::ProjectIndex::default(),
        }))
    })
}
impl Analyzer for ParserAnalyzer {
    fn completion_index(&self) -> Option<Arc<completion::CatalogIndex>> {
        Some(Arc::clone(&self.completions))
    }
    fn analyze(&mut self, job: &Job) -> Analysis {
        self.analyze_with_preparation(job, &mut |_| {})
    }
    fn analyze_with_preparation(
        &mut self,
        job: &Job,
        publish: &mut dyn FnMut(Preparation),
    ) -> Analysis {
        let started = Instant::now();
        let prepare = self.prepare_completion;
        let index = Arc::clone(&self.completions);
        let mut published = BTreeMap::new();
        let mut observer = |event: super::project::Preparation<'_>| {
            if !prepare || job.control.token.is_cancelled() {
                return;
            }
            let source = event.source;
            if job.project.as_ref().is_some_and(|snapshot| {
                !snapshot.open.iter().any(|open| {
                    open.uri == source.uri
                        && open.generation == source.generation
                        && open.source_epoch == source.source_epoch
                        && open.source == source.source
                })
            }) {
                return; // Closed sources have no protocol document or completion request.
            }
            let native_revision = event.native_revision;
            let update = match event.stage {
                super::project::PreparationStage::Started => {
                    published.insert(source.uri.clone(), (source.clone(), native_revision));
                    PreparationUpdate::Started
                }
                super::project::PreparationStage::Aborted => {
                    if published
                        .get(&source.uri)
                        .is_some_and(|(_, revision)| *revision == native_revision)
                    {
                        published.remove(&source.uri);
                    }
                    PreparationUpdate::Aborted
                }
                super::project::PreparationStage::Native(event) => match event {
                    skript_parser::StructurePreparationEvent::Headers {
                        source: mapped,
                        tree,
                        headers,
                    } => {
                        match completion::PreparedDocument::new(
                            Arc::clone(&source.source),
                            Arc::clone(&index),
                            mapped,
                            tree,
                            &headers,
                        ) {
                            Ok(document) => PreparationUpdate::Ready(Box::new(document)),
                            Err(error) => PreparationUpdate::Unavailable(error),
                        }
                    }
                    skript_parser::StructurePreparationEvent::StructureHeader {
                        source: mapped,
                        tree,
                        header,
                    } => {
                        let raw = header.candidate.raw_node_id;
                        match completion::PreparedDocument::new(
                            Arc::clone(&source.source),
                            Arc::clone(&index),
                            mapped,
                            tree,
                            &[header],
                        ) {
                            Ok(document) => PreparationUpdate::Structure(raw, Box::new(document)),
                            Err(error) => PreparationUpdate::Unavailable(error),
                        }
                    }
                    skript_parser::StructurePreparationEvent::SectionHeader {
                        source: mapped,
                        tree,
                        candidate,
                        attempt_id,
                        parent_attempt_id,
                    } => {
                        match completion::PreparedSection::new(
                            &source.source,
                            mapped,
                            tree,
                            candidate,
                            attempt_id,
                            parent_attempt_id,
                        ) {
                            Ok(Some(section)) => PreparationUpdate::Section(section),
                            Ok(None) => return,
                            Err(error) => PreparationUpdate::Unavailable(error),
                        }
                    }
                    skript_parser::StructurePreparationEvent::Invalidated {
                        raw_node_id,
                        registration_id,
                        pattern_index,
                        attempt_id,
                    } => PreparationUpdate::Invalidated(completion::OwnerAttempt {
                        root: raw_node_id,
                        registration: registration_id.into(),
                        pattern: pattern_index,
                        attempt_id,
                    }),
                },
            };
            publish(Preparation {
                source,
                native_revision,
                update,
            });
        };
        let config = DocumentParserConfig {
            cancellation: job.control.token.clone(),
            ..DocumentParserConfig::default()
        };
        let mut semantic = None;
        let mut project_semantics = Vec::new();
        let mut project_metrics = Value::Null;
        let mut project_incomplete = None;
        let mut parsed_document = true;
        let result = if let Some(snapshot) = &job.project {
            self.project.analyze(&mut self.host, Arc::clone(&self.catalog), &self.project_uri, snapshot, &job.uri, config, &mut observer).map(|analysis| {
                parsed_document = analysis.parsed_documents != 0;
                semantic = Some(analysis.current_semantic);
                project_semantics = analysis.documents;
                project_incomplete = analysis.incomplete_reason;
                project_metrics = json!({"root":snapshot.root,"currentDocument":job.uri,"complete":project_incomplete.is_none(),"incompleteReason":project_incomplete,"epoch":analysis.project_epoch,"nativeRevision":analysis.native_revision,"scannedDocuments":analysis.scanned_documents,"parsedDocuments":analysis.parsed_documents,"changedExportNames":analysis.changed_export_names});
                analysis.result
            })
        } else {
            let source = super::project::ProjectSource {
                uri: job.uri.clone(),
                generation: job.generation,
                source_epoch: job.revision,
                source: Arc::clone(&job.source),
            };
            observer(super::project::Preparation {
                source: source.clone(),
                native_revision: job.revision,
                stage: super::project::PreparationStage::Started,
            });
            self.host
                .parse_document_with_preparation(
                    DocumentParseRequest::new(
                        &self.project_uri,
                        document_key(job.uri.as_str()),
                        job.revision,
                        Arc::clone(&job.source),
                    ),
                    config,
                    &mut |event| {
                        observer(super::project::Preparation {
                            source: source.clone(),
                            native_revision: job.revision,
                            stage: super::project::PreparationStage::Native(event),
                        })
                    },
                )
                .map_err(super::project::ProjectError::from)
        };
        drop(observer);
        if !result.as_ref().is_ok_and(|result| {
            result.execution_status == parser_wasm::DocumentExecutionStatus::Complete
                && result.fault_regions.is_empty()
                && result.component_failures.is_empty()
        }) {
            for (_, (source, native_revision)) in published {
                publish(Preparation {
                    source,
                    native_revision,
                    update: PreparationUpdate::Aborted,
                });
            }
        }
        // A transferred result runs no native parse. Do not attribute the
        // producer's last hook-cache counters to this consumer job.
        let incremental = if parsed_document {
            self.host.incremental_metrics()
        } else {
            Default::default()
        };
        let analysis_metrics = json!({"project":project_metrics,"incremental":{"eligible":incremental.eligible,"hits":incremental.hits,"headerEligible":incremental.header_eligible,"headerHits":incremental.header_hits,"misses":incremental.misses,"dependencyMisses":incremental.dependency_misses,"stored":incremental.stored,"discarded":incremental.discarded}});
        let parse_ms = started.elapsed().as_secs_f64() * 1000.0;
        let report_started = Instant::now();
        let mut hover = None;
        let mut inlay = None;
        let mut annotations_error = None;
        let mut flow_diagnostics = Vec::new();
        let mut semantic_tokens = None;
        let mut semantic_error = None;
        let mut completion = None;
        let mut completion_error = None;
        let outcome = match result {
            Ok(mut result) => {
                result.ast = result.editor_ast();
                let semantic_result = semantic.take().map(Ok).unwrap_or_else(|| {
                    semantic::Document::from_result_with_host(
                        &result,
                        Arc::clone(&job.source),
                        job.uri.clone(),
                        Arc::clone(&self.catalog),
                        &mut self.host,Some(job.control.token.clone()),
                    )
                    .map(Arc::new)
                });
                match semantic_result {
                    Ok(document) => {
                        match document.diagnostics(&result) {
                            Ok(found) => flow_diagnostics = found,
                            Err(error) => annotations_error = Some(error),
                        }
                        if project_semantics.is_empty() {
                            project_semantics.push(Arc::clone(&document));
                        }
                        semantic = Some(document);
                    }
                    Err(error) => annotations_error = Some(error),
                }
                match inlay::Document::from_result(
                    &result,
                    Arc::clone(&job.source),
                    Arc::clone(&self.catalog),
                ) {
                    Ok(mut document) => {
                        if let Some(semantic) = &semantic {
                            if let Err(error) = document.add_variable_types(semantic) {
                                annotations_error = Some(error);
                            }
                        }
                        inlay = Some(Arc::new(document));
                    }
                    Err(error) => annotations_error = Some(error),
                }
                let status =
                    if result.execution_status == parser_wasm::DocumentExecutionStatus::Partial {
                        "partial"
                    } else {
                        "complete"
                    };
                if let Some(document)=&semantic {
                    match self.host.refine_editor_completions(&result,document.editor_type_facts(),Some(job.control.token.clone())) {
                        Ok(batches)=>result.editor_completions.extend(batches),
                        Err(error)=>completion_error=Some(format!("typed editing refinement failed: {error}")),
                    }
                }
                if semantic.as_ref().is_some_and(|document|!document.is_current()) {
                    // Refinement can fault/restart a provider after annotations
                    // were materialized. Never publish the revoked flow facts.
                    flow_diagnostics.clear();
                    semantic=None;
                    inlay=None;
                    annotations_error=Some("semantic transfer authority changed before publication".into());
                }
                // Conversion and recursive destruction stay on the worker's large stack.
                match completion::Document::from_result(
                    &result,
                    Arc::clone(&job.source),
                    Arc::clone(&self.completions),
                ) {
                    Ok(mut snapshot) => {
                        snapshot.semantic_types = semantic.clone();
                        completion = Some(snapshot);
                    }
                    Err(error) => completion_error = Some(error),
                }
                if self.semantic_enabled {
                    match semantic_tokens::Document::from_result(
                        &result,
                        &job.source,
                        &self.catalog,
                        &document_key(job.uri.as_str()),
                        // The provider snapshot revision is the parser's document
                        // generation. LSP textDocument.version is a separate counter
                        // and cannot authenticate this snapshot.
                        result.dynamic_syntax.document_revision,
                    ) {
                        Ok(snapshot) => semantic_tokens = Some(snapshot),
                        Err(error) => semantic_error = Some(error),
                    }
                }
                match diagnostics::from_result(&job.uri, &result) {
                    Ok(mut diagnostics) => {
                        diagnostics.append(&mut flow_diagnostics);
                        if let Some(reason) = &project_incomplete {
                            let mut notice = diagnostics::fatal(
                                "lsp.project-incomplete",
                                format!(
                                    "Project discovery is incomplete: {reason}. Analyzing this open document; project function calls and references are unavailable. Workspace: {}",
                                    job.project.as_ref().unwrap().root.display()
                                ),
                            );
                            notice.severity = Some(lsp_types::DiagnosticSeverity::WARNING);
                            diagnostics.push(notice);
                        }
                        match hover::Document::from_result(
                            result,
                            Arc::clone(&job.source),
                            Arc::clone(&self.catalog),
                            self.hover_format.clone(),
                        ) {
                            Ok(document) => {
                                hover = Some(document);
                                Outcome::Diagnostics(diagnostics, status)
                            }
                            Err(error) => {
                                semantic_tokens = None;
                                diagnostics.push(diagnostics::fatal("lsp.source-mapping", error));
                                Outcome::Diagnostics(diagnostics, "fatal")
                            }
                        }
                    }
                    Err(error) => {
                        semantic_tokens = None;
                        Outcome::Diagnostics(
                            vec![diagnostics::fatal("lsp.source-mapping", error)],
                            "fatal",
                        )
                    }
                }
            }
            Err(super::project::ProjectError::Cancelled) => Outcome::Cancelled,
            Err(error) => Outcome::Diagnostics(
                vec![diagnostics::fatal("lsp.analysis-failed", error.to_string())],
                "fatal",
            ),
        };
        Analysis {
            outcome,
            hover,
            semantic,
            inlay,
            annotations_error,
            project_semantics,
            semantic_tokens,
            semantic_error,
            completion,
            completion_error,
            analysis_metrics,
            parse_ms,
            report_ms: report_started.elapsed().as_secs_f64() * 1000.0,
        }
    }
    fn release(&mut self, uri: &str, revision: u64) -> Result<(), String> {
        if self.project.contains_uri(uri) {
            let uri = uri
                .parse()
                .map_err(|error| format!("invalid released URI: {error}"))?;
            return self
                .project
                .release_uri(&mut self.host, &self.project_uri, &uri)
                .map_err(|error| error.to_string());
        }
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
pub(super) struct Preparation {
    pub source: super::project::ProjectSource,
    pub native_revision: u64,
    pub update: PreparationUpdate,
}
pub(super) enum PreparationUpdate {
    Started,
    Ready(Box<completion::PreparedDocument>),
    Structure(skript_parser::RawNodeId, Box<completion::PreparedDocument>),
    Section(completion::PreparedSection),
    Invalidated(completion::OwnerAttempt),
    Aborted,
    Unavailable(String),
}
pub(super) enum Event {
    Preparation(Job, Preparation, f64),
    Ready(Result<Option<Arc<completion::CatalogIndex>>, String>, f64),
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
    latest: Option<String>,
    priority_burst: usize,
    active: Option<Job>,
    stopped: bool,
}
impl Queue {
    fn pending(&mut self, uri: &str) -> &mut Pending {
        if !self.documents.contains_key(uri) {
            self.order.push_back(uri.to_owned());
        }
        self.documents.entry(uri.to_owned()).or_default()
    }
    fn submit(&mut self, job: Job) {
        if self.stopped {
            job.control.cancel();
            return;
        }
        let uri = job.uri.as_str().to_owned();
        let identity = (job.generation, job.revision);
        if let Some(active) = self.active.as_ref().filter(|active| active.uri == job.uri) {
            if (active.generation, active.revision) > identity {
                job.control.cancel();
                return;
            }
            if (active.generation, active.revision) < identity {
                active.control.cancel();
            }
        }
        let pending = self.pending(&uri);
        if let Some(previous) = pending.job.as_ref() {
            if (previous.generation, previous.revision) > identity {
                job.control.cancel();
                return;
            }
            if (previous.generation, previous.revision) != identity {
                previous.control.cancel();
            }
        }
        pending.job = Some(job);
        self.latest = Some(uri);
    }

    fn release(&mut self, uri: &str, revision: u64) {
        if self.stopped {
            return;
        }
        if let Some(active) = self
            .active
            .as_ref()
            .filter(|active| active.uri.as_str() == uri && active.revision <= revision)
        {
            active.control.cancel();
        }
        let pending = self.pending(uri);
        if pending
            .job
            .as_ref()
            .is_some_and(|job| job.revision <= revision)
        {
            pending.job.take().unwrap().control.cancel();
        }
        pending.release_through = Some(pending.release_through.unwrap_or(0).max(revision));
    }

    fn pop(&mut self) -> Option<(String, Pending)> {
        // Latest input wins a short burst; the oldest valid work still advances under continuous typing.
        let index = if self.priority_burst < 3 {
            self.latest
                .as_ref()
                .and_then(|latest| self.order.iter().position(|uri| uri == latest))
                .unwrap_or(0)
        } else {
            0
        };
        let uri = self.order.remove(index)?;
        if index == 0 {
            self.priority_burst = 0;
        } else {
            self.priority_burst += 1;
        }
        if self.latest.as_ref() == Some(&uri) {
            self.latest = None;
        }
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
                    Ok(analyzer) => { let _ = sender.send(Event::Ready(Ok(analyzer.completion_index()), startup_ms)); analyzer }
                    Err(error) => { let _ = sender.send(Event::Ready(Err(error), startup_ms)); return; }
                };
                while wake_receiver.recv().is_ok() {
                    loop {
                        let pending = {
                            let mut queue = thread_queue.lock().unwrap();
                            if queue.stopped { return; }
                            let pending = queue.pop();
                            queue.active = pending.as_ref().and_then(|(_, pending)| pending.job.clone());
                            pending
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
                        let analysis = analyzer.analyze_with_preparation(&job, &mut |preparation| {
                            let _ = sender.send(Event::Preparation(job.clone(), preparation, started.elapsed().as_secs_f64()*1000.0));
                        });
                        let analysis_ms = started.elapsed().as_secs_f64() * 1000.0;
                        thread_queue.lock().unwrap().active = None;
                        let status = match &analysis.outcome { Outcome::Diagnostics(_, status) => *status, Outcome::Cancelled => "cancelled", Outcome::Stale => "stale" };
                        let metrics = json!({"uri":job.uri.as_str(), "revision":job.revision, "status":status, "queueMs":queue_ms, "parseMs":analysis.parse_ms, "reportMs":analysis.report_ms, "analysisMs":analysis_ms, "cancelToFinishMs":job.control.cancellation_ms(), "details":analysis.analysis_metrics});
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
        self.submit_all(std::iter::once(job));
    }
    pub fn submit_all(&self, jobs: impl IntoIterator<Item = Job>) {
        // Publish the whole immutable project batch before the worker can choose
        // a document. The existing bounded priority/fairness policy still applies.
        let mut queue = self.queue.lock().unwrap();
        for job in jobs {
            queue.submit(job);
        }
        drop(queue);
        let _ = self.wake.try_send(());
    }
    pub fn release(&self, uri: &str, revision: u64) {
        let mut queue = self.queue.lock().unwrap();
        queue.release(uri, revision);
        drop(queue);
        let _ = self.wake.try_send(());
    }
    pub fn stop(&mut self) {
        {
            let mut queue = self.queue.lock().unwrap();
            queue.stopped = true;
            if let Some(active) = &queue.active {
                active.control.cancel();
            }
            for pending in queue.documents.values() {
                if let Some(job) = &pending.job {
                    job.control.cancel();
                }
            }
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

#[cfg(test)]
mod scheduling_tests {
    use super::*;

    fn job(uri: &str, revision: u64) -> Job {
        Job {
            uri: uri.parse().unwrap(),
            generation: 1,
            revision,
            version: revision as i32,
            source: Arc::from(""),
            control: Control::new(),
            queued_at: Instant::now(),
            project: None,
        }
    }

    #[test]
    fn project_batch_is_locked_and_complete_before_waking_the_worker() {
        let queue = Arc::new(Mutex::new(Queue::default()));
        let (wake, wake_receiver) = bounded(1);
        let (_, events) = unbounded();
        let worker = Worker {
            queue: Arc::clone(&queue),
            wake,
            events,
            handle: None,
        };
        let jobs = [
            job("file:///background.sk", 1),
            job("file:///current.sk", 2),
        ];
        worker.submit_all(jobs.into_iter().inspect(|_| {
            assert!(
                queue.try_lock().is_err(),
                "a worker cannot pop a partly published batch"
            );
            assert!(
                wake_receiver.try_recv().is_err(),
                "do not wake the worker before the batch is complete"
            );
        }));
        wake_receiver.try_recv().unwrap();
        assert!(wake_receiver.try_recv().is_err());
        let mut queue = queue.lock().unwrap();
        assert_eq!(queue.pop().unwrap().0, "file:///current.sk");
        assert_eq!(queue.pop().unwrap().0, "file:///background.sk");
    }

    #[test]
    fn latest_input_precedes_pending_background_without_losing_it() {
        let mut queue = Queue::default();
        queue.submit(job("file:///background.sk", 1));
        queue.submit(job("file:///current.sk", 2));
        assert_eq!(queue.pop().unwrap().0, "file:///current.sk");
        assert_eq!(queue.pop().unwrap().0, "file:///background.sk");
        assert!(queue.pop().is_none());
    }

    #[test]
    fn continuously_updated_input_cannot_starve_the_oldest_document() {
        let mut queue = Queue::default();
        queue.submit(job("file:///background.sk", 1));
        for revision in 2..5 {
            queue.submit(job("file:///current.sk", revision));
            assert_eq!(queue.pop().unwrap().0, "file:///current.sk");
        }
        queue.submit(job("file:///current.sk", 5));
        assert_eq!(queue.pop().unwrap().0, "file:///background.sk");
        assert_eq!(queue.pop().unwrap().1.job.unwrap().revision, 5);
    }

    #[test]
    fn replacements_cancel_obsolete_work_and_reject_out_of_order_jobs() {
        let mut queue = Queue::default();
        let active = job("file:///current.sk", 1);
        let active_control = active.control.clone();
        queue.active = Some(active);
        let first = job("file:///current.sk", 2);
        let first_control = first.control.clone();
        queue.submit(first);
        assert!(active_control.token.is_cancelled());
        queue.submit(job("file:///current.sk", 3));
        assert!(first_control.token.is_cancelled());
        let stale = job("file:///current.sk", 2);
        let stale_control = stale.control.clone();
        queue.submit(stale);
        assert!(stale_control.token.is_cancelled());
        assert_eq!(queue.pop().unwrap().1.job.unwrap().revision, 3);
    }

    #[test]
    fn release_barrier_survives_priority_and_preserves_a_reopened_revision() {
        let mut queue = Queue::default();
        let old = job("file:///current.sk", 1);
        let old_control = old.control.clone();
        queue.submit(old);
        queue.release("file:///current.sk", 1);
        assert!(old_control.token.is_cancelled());
        queue.submit(job("file:///current.sk", 2));
        queue.release("file:///current.sk", 1);
        let pending = queue.pop().unwrap().1;
        assert_eq!(pending.release_through, Some(1));
        assert_eq!(pending.job.as_ref().unwrap().revision, 2);
        assert!(!pending.job.unwrap().control.token.is_cancelled());
    }

    #[test]
    fn shutdown_cancels_running_and_pending_work_and_refuses_new_submissions() {
        let mut queue = Queue::default();
        let active = job("file:///active.sk", 1);
        let active_control = active.control.clone();
        queue.active = Some(active);
        let pending = job("file:///pending.sk", 2);
        let pending_control = pending.control.clone();
        queue.submit(pending);
        let queue = Arc::new(Mutex::new(queue));
        let (wake, _) = bounded(1);
        let (_, events) = unbounded();
        let mut worker = Worker {
            queue: Arc::clone(&queue),
            wake,
            events,
            handle: None,
        };
        worker.stop();
        assert!(active_control.token.is_cancelled());
        assert!(pending_control.token.is_cancelled());
        let late = job("file:///late.sk", 3);
        let late_control = late.control.clone();
        worker.submit(late);
        assert!(late_control.token.is_cancelled());
        assert!(queue.lock().unwrap().pop().is_none());
    }
    #[test]
    fn unrelated_current_work_is_not_cancelled_by_input_in_another_document() {
        let mut queue = Queue::default();
        let active = job("file:///background.sk", 1);
        let control = active.control.clone();
        queue.active = Some(active);
        queue.submit(job("file:///current.sk", 2));
        assert!(!control.token.is_cancelled());
    }
}

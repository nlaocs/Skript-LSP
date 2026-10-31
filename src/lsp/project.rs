//! Worker-owned project snapshots. Open text overrides disk; accepted parser
//! definitions, including failed lookups, drive the project fixed point.
use super::semantic;
use lsp_types::Uri;
use parser_wasm::{
    DocumentParseError, DocumentParseRequest, DocumentParseResult, DocumentParserConfig, ParserHost,
    DeclarationExports, DeclarationInputs, DeclarationSource,
};
use sha2::{Digest, Sha256};
use skript_parser::{ImportedFunction, StructurePreparationEvent};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
use syntaxes::Catalog;

const MAX_FILES: usize = 512;
const MAX_ENTRIES: usize = 50_000;
const MAX_DEPTH: usize = 64;
const MAX_FILE_BYTES: usize = 1024 * 1024;
const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
const MAX_PARSES: usize = 4 * MAX_FILES;

#[derive(Clone, Debug)]
pub(super) struct ProjectSource {
    pub uri: Uri,
    pub generation: u64,
    pub source_epoch: u64,
    pub source: Arc<str>,
}
#[derive(Clone, Debug)]
pub(super) struct ProjectSnapshot {
    pub epoch: u64,
    pub root: PathBuf,
    pub open: Vec<ProjectSource>,
}
/// A native parse attempt belongs to its source, including side documents in a batch.
pub(super) struct Preparation<'a> {
    pub source: ProjectSource,
    pub native_revision: u64,
    pub stage: PreparationStage<'a>,
}
pub(super) enum PreparationStage<'a> {
    Started,
    Native(StructurePreparationEvent<'a>),
    Aborted,
}

type PreparationObserver<'a> = dyn for<'p> FnMut(Preparation<'p>) + 'a;
type PreparedAttempts = BTreeMap<String, (ProjectSource, u64)>;

fn track_preparation(
    event: Preparation<'_>,
    attempts: &mut PreparedAttempts,
    observer: &mut PreparationObserver<'_>,
) {
    match &event.stage {
        PreparationStage::Started => {
            attempts.insert(
                event.source.uri.as_str().to_owned(),
                (event.source.clone(), event.native_revision),
            );
        }
        PreparationStage::Aborted => {
            if attempts
                .get(event.source.uri.as_str())
                .is_some_and(|(_, revision)| *revision == event.native_revision)
            {
                attempts.remove(event.source.uri.as_str());
            }
        }
        PreparationStage::Native(_) => {}
    }
    observer(event);
}
fn abort_preparation(attempts: &mut PreparedAttempts, observer: &mut PreparationObserver<'_>) {
    for (_, (source, native_revision)) in std::mem::take(attempts) {
        observer(Preparation {
            source,
            native_revision,
            stage: PreparationStage::Aborted,
        });
    }
}

#[derive(Debug)]
pub(super) enum ProjectError {
    Cancelled,
    Incomplete(String),
    Failed(String),
}
impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("project analysis cancelled"),
            Self::Incomplete(message) | Self::Failed(message) => f.write_str(message),
        }
    }
}
impl From<DocumentParseError> for ProjectError {
    fn from(error: DocumentParseError) -> Self {
        match error {
            DocumentParseError::Cancelled { .. } => Self::Cancelled,
            error => Self::Failed(error.to_string()),
        }
    }
}
impl From<String> for ProjectError {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

pub(super) struct ProjectAnalysis {
    pub result: DocumentParseResult,
    pub current_semantic: Arc<semantic::Document>,
    pub documents: Vec<Arc<semantic::Document>>,
    pub native_revision: u64,
    pub project_epoch: u64,
    pub changed_export_names: BTreeSet<String>,
    pub scanned_documents: usize,
    pub parsed_documents: usize,
    pub incomplete_reason: Option<String>,
}

#[derive(Clone, Debug)]
struct Source {
    view: ProjectSource,
    witness: String,
    path: Option<PathBuf>,
}
impl Source {
    fn key(&self) -> &str {
        self.view.uri.as_str()
    }
    fn same(&self, other: &Self) -> bool {
        self.view.generation == other.view.generation
            && self.view.source_epoch == other.view.source_epoch
            && self.witness == other.witness
    }
}
#[derive(Clone)]
struct Cached {
    source: Source,
    exports: Vec<ImportedFunction>,
    export_key: String,
    imported_key: String,
    prototype_inputs: String,
    prototypes: Option<DeclarationExports>,
    lookup_names: BTreeSet<String>,
    semantic: Arc<semantic::Document>,
}
// One owned, completed side-document result can be handed to another job in the
// same immutable batch. This does not transplant an AST into a new native revision.
struct ParsedResult {
    result: DocumentParseResult,
    source: Source,
    import_key: String,
    prototype_key: String,
    revision: u64,
}
struct ResultHandoff {
    epoch: u64,
    project_uri: String,
    root: PathBuf,
    open: Vec<(String, u64, u64, String)>,
    raw_tree: Option<skript_parser::RawTreeOptions>,
    structure: skript_parser::StructureParserConfig,
    catalog: Arc<Catalog>,
    participants: Vec<parser_wasm::host::ComponentInfo>,
    parsed: ParsedResult,
}
fn open_key(snapshot: &ProjectSnapshot) -> Vec<(String, u64, u64, String)> {
    snapshot
        .open
        .iter()
        .map(|source| {
            (
                source.uri.as_str().to_owned(),
                source.generation,
                source.source_epoch,
                digest(&source.source),
            )
        })
        .collect()
}
#[derive(Default)]
pub(super) struct ProjectIndex {
    documents: BTreeMap<String, Cached>,
    next_native_revision: u64,
    revisions: BTreeMap<String, u64>,
    handoff: Option<ResultHandoff>,
}

fn digest(source: &str) -> String {
    format!("{:x}", Sha256::digest(source.as_bytes()))
}
pub(super) fn document_key(uri: &str) -> String {
    format!("lsp-document:{}", digest(uri))
}
fn failure(message: impl Into<String>) -> ProjectError {
    ProjectError::Failed(message.into())
}

fn incomplete(message: impl Into<String>) -> ProjectError {
    ProjectError::Incomplete(message.into())
}

fn uri_path(uri: &Uri) -> Result<PathBuf, ProjectError> {
    url::Url::parse(uri.as_str())
        .map_err(|error| failure(format!("invalid project URI: {error}")))?
        .to_file_path()
        .map_err(|_| failure("project sources require file URIs"))
}
fn path_uri(path: &Path) -> Result<Uri, ProjectError> {
    url::Url::from_file_path(path)
        .map_err(|_| failure(format!("cannot represent project path {}", path.display())))?
        .as_str()
        .parse()
        .map_err(|error| failure(format!("invalid project file URI: {error}")))
}
fn canonical_source(path: &Path, root: &Path) -> Result<PathBuf, ProjectError> {
    let resolved = if path.exists() {
        fs::canonicalize(path)
            .map_err(|error| failure(format!("cannot resolve {}: {error}", path.display())))?
    } else {
        let ancestor = path
            .ancestors()
            .find(|ancestor| ancestor.exists())
            .ok_or_else(|| failure("project file has no existing ancestor"))?;
        let suffix = path
            .strip_prefix(ancestor)
            .map_err(|_| failure("invalid project path ancestry"))?;
        if suffix
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(failure("project path contains unresolved traversal"));
        }
        fs::canonicalize(ancestor)
            .map_err(|error| failure(format!("cannot resolve {}: {error}", ancestor.display())))?
            .join(suffix)
    };
    if !resolved.starts_with(root) {
        return Err(failure(format!(
            "project file resolves outside workspace: {}",
            path.display()
        )));
    }
    Ok(resolved)
}
fn link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}
fn project_marker(directory: &Path) -> Result<bool, ProjectError> {
    match fs::symlink_metadata(directory.join(".git")) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(incomplete(format!(
            "cannot inspect project boundary {}: {error}",
            directory.display()
        ))),
    }
}
fn validate_member(path: &Path, root: &Path) -> Result<(), ProjectError> {
    canonical_source(path, root).map_err(|error| incomplete(error.to_string()))?;
    // Compare canonical ancestors, but inspect their original spelling for links.
    for (index, ancestor) in path.ancestors().enumerate() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if link(&metadata) => {
                return Err(incomplete(format!(
                    "project source path became a link: {}",
                    path.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(incomplete(format!(
                    "cannot inspect source ancestry {}: {error}",
                    ancestor.display()
                )));
            }
        }
        let canonical =
            canonical_source(ancestor, root).map_err(|error| incomplete(error.to_string()))?;
        if canonical == root {
            return Ok(());
        }
        if index > 0 && project_marker(ancestor)? {
            return Err(incomplete(format!(
                "project source belongs to a nested project: {}",
                path.display()
            )));
        }
    }
    Err(incomplete("project source has no workspace ancestor"))
}
fn read_source(path: &Path) -> Result<Arc<str>, ProjectError> {
    if fs::canonicalize(path).ok().as_deref() != Some(path) {
        return Err(incomplete(format!(
            "project source changed or became a link: {}",
            path.display()
        )));
    }
    let before = fs::metadata(path).map_err(|error| {
        incomplete(format!(
            "cannot read project source {}: {error}",
            path.display()
        ))
    })?;
    if before.len() > MAX_FILE_BYTES as u64 {
        return Err(incomplete(format!(
            "project source exceeds {MAX_FILE_BYTES} byte analysis limit: {}",
            path.display()
        )));
    }
    // Bound the actual read too: a file can grow after its metadata was checked.
    let mut bytes = Vec::with_capacity((before.len() as usize).min(MAX_FILE_BYTES));
    fs::File::open(path)
        .and_then(|file| {
            file.take((MAX_FILE_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
        })
        .map_err(|error| {
            incomplete(format!(
                "cannot read project source {}: {error}",
                path.display()
            ))
        })?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(incomplete(
            "project source exceeded the byte limit while reading",
        ));
    }
    let after = fs::metadata(path).map_err(|error| {
        incomplete(format!(
            "cannot recheck project source {}: {error}",
            path.display()
        ))
    })?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || fs::canonicalize(path).ok().as_deref() != Some(path)
    {
        return Err(incomplete(format!(
            "project source changed while reading: {}",
            path.display()
        )));
    }
    let source = String::from_utf8(bytes)
        .map_err(|_| incomplete(format!("project source is not UTF-8: {}", path.display())))?;
    Ok(Arc::from(source))
}
fn sources(
    snapshot: &ProjectSnapshot,
    cancel: &parser_wasm::DocumentCancellationToken,
) -> Result<BTreeMap<String, Source>, ProjectError> {
    let root = fs::canonicalize(&snapshot.root).map_err(|error| {
        incomplete(format!(
            "cannot resolve project root {}: {error}",
            snapshot.root.display()
        ))
    })?;
    if !root.is_dir() {
        return Err(incomplete("project root is not a directory"));
    }
    let mut open = BTreeMap::new();
    for view in &snapshot.open {
        let original = uri_path(&view.uri)?;
        validate_member(&original, &root)?;
        let path =
            canonical_source(&original, &root).map_err(|error| incomplete(error.to_string()))?;
        if open.insert(path, view.clone()).is_some() {
            return Err(incomplete("two open project URIs resolve to the same file"));
        }
    }
    let mut pending = vec![(root.clone(), 0)];
    let mut files = BTreeSet::new();
    let mut entries = 0usize;
    while let Some((directory, depth)) = pending.pop() {
        if cancel.is_cancelled() {
            return Err(ProjectError::Cancelled);
        }
        if depth > MAX_DEPTH {
            return Err(incomplete(format!(
                "project directory nesting exceeds {MAX_DEPTH}"
            )));
        }
        let metadata = fs::symlink_metadata(&directory).map_err(|error| {
            incomplete(format!("cannot inspect {}: {error}", directory.display()))
        })?;
        if link(&metadata) || fs::canonicalize(&directory).ok().as_ref() != Some(&directory) {
            return Err(incomplete(format!(
                "project directory changed or became a link: {}",
                directory.display()
            )));
        }
        if directory != root && project_marker(&directory)? {
            continue;
        }
        let mut children = Vec::new();
        for child in fs::read_dir(&directory)
            .map_err(|error| incomplete(format!("cannot scan {}: {error}", directory.display())))?
        {
            if cancel.is_cancelled() {
                return Err(ProjectError::Cancelled);
            }
            entries += 1;
            if entries > MAX_ENTRIES {
                return Err(incomplete(format!(
                    "project scan exceeds {MAX_ENTRIES} directory entries"
                )));
            }
            children.push(child.map_err(|error| {
                incomplete(format!("cannot scan {}: {error}", directory.display()))
            })?);
        }
        if fs::canonicalize(&directory).ok().as_ref() != Some(&directory)
            || fs::symlink_metadata(&directory).map_or(true, |m| link(&m))
        {
            return Err(incomplete(format!(
                "project directory changed during discovery: {}",
                directory.display()
            )));
        }
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let path = child.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| {
                incomplete(format!("cannot inspect {}: {error}", path.display()))
            })?;
            if link(&metadata) {
                continue;
            }
            if metadata.is_dir() {
                if child.file_name() != ".git" {
                    pending.push((path, depth + 1));
                }
            } else if metadata.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("sk"))
            {
                files.insert(
                    canonical_source(&path, &root)
                        .map_err(|error| incomplete(error.to_string()))?,
                );
                if files.len() > MAX_FILES {
                    return Err(incomplete(format!(
                        "project source analysis limit is {MAX_FILES}"
                    )));
                }
            }
        }
    }
    files.extend(open.keys().cloned());
    if files.len() > MAX_FILES {
        return Err(incomplete(format!(
            "project contains {} sources; analysis limit is {MAX_FILES}",
            files.len()
        )));
    }
    let mut output = BTreeMap::new();
    let mut total = 0usize;
    for path in files {
        if cancel.is_cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let (view, disk) = if let Some(view) = open.remove(&path) {
            (view, None)
        } else {
            let source = read_source(&path)?;
            (
                ProjectSource {
                    uri: path_uri(&path)?,
                    generation: 0,
                    source_epoch: 0,
                    source,
                },
                Some(path),
            )
        };
        total = total
            .checked_add(view.source.len())
            .ok_or_else(|| failure("project source size overflow"))?;
        if view.source.len() > MAX_FILE_BYTES || total > MAX_SOURCE_BYTES {
            return Err(incomplete(format!(
                "project analysis source budget exceeded ({MAX_FILE_BYTES} bytes per file; {MAX_SOURCE_BYTES} total)"
            )));
        }
        let source = Source {
            witness: digest(&view.source),
            view,
            path: disk,
        };
        if output.insert(source.key().to_owned(), source).is_some() {
            return Err(failure("project URI collision"));
        }
    }
    Ok(output)
}

/// Reuse discovery, link/boundary checks and source limits before a source edit.
/// Watch notifications alone cannot prove that no participant was added/deleted.
pub(super) fn verify_source_set(
    snapshot: &ProjectSnapshot,
    expected: &BTreeMap<String, String>,
) -> Result<(), ProjectError> {
    let found = sources(snapshot, &parser_wasm::DocumentCancellationToken::new())?;
    if found.len() != expected.len()
        || found
            .iter()
            .any(|(uri, source)| expected.get(uri) != Some(&source.witness))
    {
        return Err(incomplete(
            "project membership or source changed since analysis; reanalysis is required",
        ));
    }
    Ok(())
}

fn declaration_source(source: &Source) -> DeclarationSource {
    DeclarationSource::new(source.key(), document_key(source.key()), source.view.generation,
        source.view.source_epoch, &source.view.source)
}
fn declaration_inputs(host: &ParserHost, project: &str, sources: &BTreeMap<String, DeclarationSource>,
    current: &Source, epoch: u64, records: &BTreeMap<String, Cached>) -> DeclarationInputs
{
    if !host.project_declarations_enabled() { return DeclarationInputs::default(); }
    let Some(current) = sources.get(current.key()) else { return DeclarationInputs::default(); };
    DeclarationInputs::new(current.clone(), sources.values().cloned().collect(), true)
        .with_epoch(epoch).with_exports(records.values().filter_map(|record| record.prototypes.as_ref())
            .filter(|record| host.declaration_exports_are_current(project, record)).cloned().collect())
}
fn prototype_current(host: &ParserHost, project: &str, record: &Cached) -> bool {
    record.prototypes.as_ref().is_none_or(|record| host.declaration_exports_are_current(project, record))
}
fn prototype_export_key(record: &Cached) -> &str {
    record.prototypes.as_ref().map_or("", DeclarationExports::fingerprint)
}

fn definitions_key(functions: &[ImportedFunction]) -> String {
    functions
        .iter()
        .map(|item| {
            let mut definition = item.definition.clone();
            definition.metadata.remove("function.revision");
            format!("{}|{definition:?}", item.document_id)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
fn imports(records: &BTreeMap<String, Cached>, current: &str) -> Vec<ImportedFunction> {
    records
        .iter()
        .filter(|(uri, _)| uri.as_str() != current)
        .flat_map(|(_, record)| record.exports.clone())
        .collect()
}
fn relevant_key(functions: &[ImportedFunction], names: &BTreeSet<String>) -> String {
    definitions_key(
        &functions
            .iter()
            .filter(|function| names.contains(&function.definition.name))
            .cloned()
            .collect::<Vec<_>>(),
    )
}

impl ProjectIndex {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn contains_uri(&self, uri: &str) -> bool {
        self.revisions.contains_key(uri)
    }
    pub fn release_uri(
        &mut self,
        host: &mut ParserHost,
        project_uri: &str,
        uri: &Uri,
    ) -> Result<(), String> {
        if self
            .handoff
            .as_ref()
            .is_some_and(|handoff| handoff.parsed.source.key() == uri.as_str())
        {
            self.handoff = None;
        }
        self.documents.remove(uri.as_str());
        if let Some(revision) = self.revisions.remove(uri.as_str()) {
            host.release_document(project_uri, &document_key(uri.as_str()), revision)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
    fn parse(
        &mut self,
        host: &mut ParserHost,
        catalog: Arc<Catalog>,
        project_uri: &str,
        source: &Source,
        mut config: DocumentParserConfig,
        imported: Vec<ImportedFunction>,
        _epoch: u64,
        observer: Option<&mut PreparationObserver<'_>>,
    ) -> Result<(DocumentParseResult, Cached, u64), ProjectError> {
        if config.cancellation.is_cancelled() {
            return Err(ProjectError::Cancelled);
        }
        self.next_native_revision = self
            .next_native_revision
            .checked_add(1)
            .ok_or_else(|| failure("project native revision exhausted"))?;
        let revision = self.next_native_revision;
        self.revisions.insert(source.key().to_owned(), revision);
        config.imported_functions = imported.clone();
        let request = DocumentParseRequest::new(
            project_uri,
            document_key(source.key()),
            revision,
            Arc::clone(&source.view.source),
        );
        let mut observer = observer;
        if let Some(observer) = observer.as_mut() {
            observer(Preparation {
                source: source.view.clone(),
                native_revision: revision,
                stage: PreparationStage::Started,
            });
        }
        let prototype_inputs = config.declarations.fingerprint();
        let semantic_cancellation=config.cancellation.clone();
        let parsed = match observer.as_mut() {
            Some(observer) => host.parse_document_with_preparation(request, config, &mut |event| {
                observer(Preparation {
                    source: source.view.clone(),
                    native_revision: revision,
                    stage: PreparationStage::Native(event),
                });
            }),
            None => host.parse_document(request, config),
        };
        let result = match parsed {
            Ok(result) => result,
            Err(error) => {
                if let Some(observer) = observer.as_mut() {
                    observer(Preparation {
                        source: source.view.clone(),
                        native_revision: revision,
                        stage: PreparationStage::Aborted,
                    });
                }
                return Err(error.into());
            }
        };
        if result.execution_status != parser_wasm::DocumentExecutionStatus::Complete
            || !result.fault_regions.is_empty()
            || !result.component_failures.is_empty()
        {
            if let Some(observer) = observer.as_mut() {
                observer(Preparation {
                    source: source.view.clone(),
                    native_revision: revision,
                    stage: PreparationStage::Aborted,
                });
            }
        }
        let semantic = Arc::new(semantic::Document::from_result_with_host(
            &result,
            Arc::clone(&source.view.source),
            source.view.uri.clone(),
            catalog,
            host,Some(semantic_cancellation),
        )?);
        let lookup_names = result
            .functions
            .lookups()
            .iter()
            .map(|lookup| lookup.name.clone())
            .collect::<BTreeSet<_>>();
        let imported_key = relevant_key(&imported, &lookup_names);
        let mut exports = Vec::new();
        for symbol in &semantic.functions {
            if symbol.local {
                continue;
            }
            let mut definition = symbol.definition.clone();
            definition
                .metadata
                .insert("function.source-witness".into(), source.witness.clone());
            exports.push(ImportedFunction {
                definition,
                document_id: result.functions.document_id().to_owned(),
                document_revision: revision,
            });
        }
        let export_key = definitions_key(&exports);
        let prototypes = result.declaration_exports.clone();
        Ok((
            result,
            Cached {
                source: source.clone(),
                exports,
                export_key,
                imported_key,
                prototype_inputs,
                prototypes,
                lookup_names,
                semantic,
            },
            revision,
        ))
    }
    pub fn analyze(
        &mut self,
        host: &mut ParserHost,
        catalog: Arc<Catalog>,
        project_uri: &str,
        snapshot: &ProjectSnapshot,
        current_uri: &Uri,
        config: DocumentParserConfig,
        observer: &mut PreparationObserver<'_>,
    ) -> Result<ProjectAnalysis, ProjectError> {
        let mut attempts = PreparedAttempts::new();
        let result = self.analyze_complete(
            host,
            Arc::clone(&catalog),
            project_uri,
            snapshot,
            current_uri,
            config.clone(),
            &mut |event| track_preparation(event, &mut attempts, observer),
        );
        let result = match result {
            Err(ProjectError::Incomplete(reason)) => {
                // A failed discovery/final witness cannot lend proposals to the local fallback.
                abort_preparation(&mut attempts, observer);
                self.analyze_local(
                    host,
                    catalog,
                    project_uri,
                    snapshot,
                    current_uri,
                    config,
                    &mut |event| track_preparation(event, &mut attempts, observer),
                    reason,
                )
            }
            result => result,
        };
        if result.is_err() {
            abort_preparation(&mut attempts, observer);
        }
        result
    }
    fn analyze_complete(
        &mut self,
        host: &mut ParserHost,
        catalog: Arc<Catalog>,
        project_uri: &str,
        snapshot: &ProjectSnapshot,
        current_uri: &Uri,
        config: DocumentParserConfig,
        observer: &mut PreparationObserver<'_>,
    ) -> Result<ProjectAnalysis, ProjectError> {
        // Take before discovery so cancelled, failed and incomplete passes cannot
        // leave a result available for a later job. Host/index/catalog share the
        // immutable ParserAnalyzer lifetime; every semantic input is checked below.
        let previous = self.handoff.take();
        let scan_root = fs::canonicalize(&snapshot.root)
            .map_err(|error| incomplete(format!("cannot resolve project root: {error}")))?;
        let discovered = sources(snapshot, &config.cancellation)?;
        // Hash actual bytes once per immutable discovery batch, rather than
        // rehashing all source text for every fixed-point comparison/attempt.
        let declaration_sources = if host.project_declarations_enabled() {
            discovered.iter().map(|(uri, source)| (uri.clone(), declaration_source(source))).collect()
        } else { BTreeMap::new() };
        require_stable_project(discovered.len(), host.project_semantics_are_stable())?;
        let participants = host.components();
        let batch_open = open_key(snapshot);
        let cancellation = config.cancellation.clone();
        let raw_tree = config.raw_tree;
        let structure = config.structure.clone();
        let previous = previous.filter(|handoff| {
            handoff.epoch == snapshot.epoch
                && handoff.project_uri == project_uri
                && handoff.root == snapshot.root
                && handoff.open == batch_open
                && handoff.raw_tree == raw_tree
                && handoff.structure == structure
                && Arc::ptr_eq(&handoff.catalog, &catalog)
                && handoff.participants == participants
        });
        let current = discovered.get(current_uri.as_str()).ok_or_else(|| {
            failure("current project document is absent from immutable open snapshot")
        })?;
        if !snapshot
            .open
            .iter()
            .any(|source| source.uri == *current_uri)
        {
            return Err(failure(
                "current project document must be an open-buffer snapshot",
            ));
        }
        let old_names = self
            .documents
            .values()
            .flat_map(|record| {
                record
                    .exports
                    .iter()
                    .map(|item| item.definition.name.clone())
            })
            .collect::<BTreeSet<_>>();
        // Revoke stale source views before lending any cached header or result.
        let source_inputs = declaration_inputs(host, project_uri, &declaration_sources, current, snapshot.epoch, &BTreeMap::new());
        host.synchronize_declaration_sources(&DocumentParseRequest::new(project_uri, document_key(current.key()),
            0, Arc::clone(&current.view.source)), &source_inputs).map_err(|error| failure(error.to_string()))?;
        let before = self
            .documents
            .iter()
            .map(|(uri, record)| (uri.clone(), record.export_key.clone()))
            .collect::<BTreeMap<_, _>>();
        // Do not lend stale exports to a first discovery pass after owner edits.
        let mut working = self
            .documents
            .iter()
            .filter_map(|(uri, record)| {
                discovered
                    .get(uri)
                    .filter(|source| source.same(&record.source)&&record.semantic.is_current()&&prototype_current(host, project_uri, record))
                    .map(|_| (uri.clone(), record.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let removed = self
            .documents
            .keys()
            .filter(|uri| !discovered.contains_key(*uri))
            .cloned()
            .collect::<Vec<_>>();
        for uri in removed {
            let uri: Uri = uri
                .parse()
                .map_err(|error| failure(format!("invalid cached URI: {error}")))?;
            self.release_uri(host, project_uri, &uri)?;
        }
        let mut parsed_documents = 0usize;
        if discovered.len() == 1 {
            let mut single_config = config;
            single_config.declarations = declaration_inputs(host, project_uri, &declaration_sources, current, snapshot.epoch, &working);
            let (result, record, revision) = self.parse(
                host,
                Arc::clone(&catalog),
                project_uri,
                current,
                single_config,
                Vec::new(),
                snapshot.epoch,
                Some(observer),
            )?;
            parsed_documents += 1;
            let current_semantic = Arc::clone(&record.semantic);
            working.insert(current_uri.as_str().to_owned(), record);
            return self.finish(
                discovered,
                working,
                before,
                old_names,
                result,
                current_semantic,
                revision,
                snapshot.epoch,
                parsed_documents,
                &scan_root,
            );
        }
        let mut order = vec![current_uri.as_str().to_owned()];
        order.extend(
            discovered
                .keys()
                .filter(|uri| uri.as_str() != current_uri.as_str())
                .cloned(),
        );
        // A prior successful job may already own this exact native result.
        // Fresh discovery and the final full-namespace/source/revision guards
        // still run before it is consumed. Only one side result is staged.
        let mut current_result = previous
            .map(|handoff| handoff.parsed)
            .filter(|parsed| parsed.source.key() == current_uri.as_str()
                && parsed.result.declaration_exports.as_ref().is_none_or(|exports| host.declaration_exports_are_current(project_uri, exports)));
        let mut staged = None;
        let mut stable = false;
        let max_rounds = discovered.len().saturating_add(1);
        for _round in 0..max_rounds {
            if config.cancellation.is_cancelled() {
                return Err(ProjectError::Cancelled);
            }
            let mut any = false;
            for uri in &order {
                let source = &discovered[uri];
                let imported = imports(&working, uri);
                let prototypes = declaration_inputs(host, project_uri, &declaration_sources, source, snapshot.epoch, &working);
                let prototype_key = prototypes.fingerprint();
                let key = working
                    .get(uri)
                    .map(|record| relevant_key(&imported, &record.lookup_names));
                let needs = working
                    .get(uri)
                    .is_none_or(|record| Some(&record.imported_key) != key.as_ref()
                        || record.prototype_inputs != prototype_key || !prototype_current(host, project_uri, record));
                if !needs {
                    continue;
                }
                if parsed_documents >= MAX_PARSES {
                    return Err(failure(
                        "project function analysis exceeded its parse budget",
                    ));
                }
                let import_key = definitions_key(&imported);
                let mut parse_config = config.clone();
                parse_config.declarations = prototypes;
                let (result, record, revision) = self.parse(
                    host,
                    Arc::clone(&catalog),
                    project_uri,
                    source,
                    parse_config,
                    imported,
                    snapshot.epoch,
                    Some(observer),
                )?;
                let parsed = ParsedResult {
                    result,
                    source: source.clone(),
                    import_key,
                    prototype_key,
                    revision,
                };
                if uri == current_uri.as_str() {
                    current_result = Some(parsed);
                } else if snapshot.open.iter().any(|view| view.uri.as_str() == uri) {
                    staged = Some(parsed);
                }
                working.insert(uri.clone(), record);
                parsed_documents += 1;
                any = true;
            }
            if !any {
                stable = true;
                break;
            }
            let matched = order.iter().all(|uri| {
                working.get(uri).is_some_and(|record| {
                    record.imported_key
                        == relevant_key(&imports(&working, uri), &record.lookup_names)
                        && record.prototype_inputs == declaration_inputs(host, project_uri, &declaration_sources, &discovered[uri], snapshot.epoch, &working).fingerprint()
                        && prototype_current(host, project_uri, record)
                })
            });
            if matched {
                stable = true;
                break;
            }
        }
        if !stable {
            return Err(failure(format!(
                "project function declarations did not converge within {max_rounds} rounds"
            )));
        }
        if config.cancellation.is_cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let expected = working
            .get(current_uri.as_str())
            .map(|record| record.export_key.clone())
            .unwrap_or_default();
        let expected_prototype = working.get(current_uri.as_str()).map(|record| prototype_export_key(record).to_owned()).unwrap_or_default();
        let imported = imports(&working, current_uri.as_str());
        let final_prototypes = declaration_inputs(host, project_uri, &declaration_sources, current, snapshot.epoch, &working);
        let final_prototype_key = final_prototypes.fingerprint();
        let retained = current_result.filter(|parsed| {
            parsed.import_key == definitions_key(&imported)
                && parsed.prototype_key == final_prototype_key
                && parsed.result.declaration_exports.as_ref().is_none_or(|exports| host.declaration_exports_are_current(project_uri, exports))
                && parsed.source.same(current)
                && self.revisions.get(current_uri.as_str()) == Some(&parsed.revision)
                && working
                    .get(current_uri.as_str())
                    .is_some_and(|record| record.source.same(current)&&record.semantic.is_current())
        });
        let (result, record, revision) = if let Some(parsed) = retained {
            let record = working
                .get(current_uri.as_str())
                .cloned()
                .ok_or_else(|| failure("retained current project record is absent"))?;
            (parsed.result, record, parsed.revision)
        } else {
            let mut final_config = config;
            final_config.declarations = final_prototypes;
            let parsed = self.parse(
                host,
                Arc::clone(&catalog),
                project_uri,
                current,
                final_config,
                imported,
                snapshot.epoch,
                Some(observer),
            )?;
            parsed_documents += 1;
            parsed
        };
        // A component can disable itself after a trap during these parses.
        // Earlier side-document facts then describe a different environment.
        require_stable_project(discovered.len(), host.project_semantics_are_stable())?;
        if host.components() != participants {
            return Err(failure(
                "project component environment changed during analysis",
            ));
        }
        if record.export_key != expected || prototype_export_key(&record) != expected_prototype {
            return Err(failure(
                "current function declarations changed after project stabilization; extension state is not a stable project input",
            ));
        }
        let current_semantic = Arc::clone(&record.semantic);
        working.insert(current_uri.as_str().to_owned(), record);
        if working.values().any(|record| !prototype_current(host, project_uri, record)) {
            return Err(incomplete("project declaration source/provider inputs changed during analysis"));
        }
        if host.project_declarations_enabled() && working.values().any(|record| record.prototypes.as_ref().is_none_or(|record| !record.complete() || !record.imports_complete())) {
            return Err(incomplete("project declaration namespace is incomplete; using document-only analysis"));
        }
        // The compact declaration source stamps remain available after finish.
        let analysis = self.finish(
            discovered,
            working,
            before,
            old_names,
            result,
            current_semantic,
            revision,
            snapshot.epoch,
            parsed_documents,
            &scan_root,
        )?;
        // Publish a handoff only after membership and closed-source witnesses
        // passed finish. A changed complete import namespace requires a reparse,
        // even when the side result never looked up that newly available name.
        if cancellation.is_cancelled() {
            return Err(ProjectError::Cancelled);
        }
        if let Some(parsed) = staged.filter(|parsed| {
            parsed.result.execution_status == parser_wasm::DocumentExecutionStatus::Complete
                && parsed.result.fault_regions.is_empty()
                && parsed.result.component_failures.is_empty()
                && parsed.import_key
                    == definitions_key(&imports(&self.documents, parsed.source.key()))
                && parsed.prototype_key == declaration_inputs(host, project_uri, &declaration_sources, &parsed.source, snapshot.epoch, &self.documents).fingerprint()
                && parsed.result.declaration_exports.as_ref().is_none_or(|exports| host.declaration_exports_are_current(project_uri, exports))
                && self.revisions.get(parsed.source.key()) == Some(&parsed.revision)
                && self
                    .documents
                    .get(parsed.source.key())
                    .is_some_and(|record| record.source.same(&parsed.source)&&record.semantic.is_current())
        }) {
            self.handoff = Some(ResultHandoff {
                epoch: snapshot.epoch,
                project_uri: project_uri.to_owned(),
                root: snapshot.root.clone(),
                open: batch_open,
                raw_tree,
                structure,
                catalog,
                participants,
                parsed,
            });
        }
        Ok(analysis)
    }
    fn analyze_local(
        &mut self,
        host: &mut ParserHost,
        catalog: Arc<Catalog>,
        project_uri: &str,
        snapshot: &ProjectSnapshot,
        current_uri: &Uri,
        mut config: DocumentParserConfig,
        observer: &mut PreparationObserver<'_>,
        reason: String,
    ) -> Result<ProjectAnalysis, ProjectError> {
        if config.cancellation.is_cancelled() {
            return Err(ProjectError::Cancelled);
        }
        let view = snapshot
            .open
            .iter()
            .find(|view| view.uri == *current_uri)
            .ok_or_else(|| failure("current project document must be an open-buffer snapshot"))?;
        if view.source.len() > MAX_FILE_BYTES {
            return Err(failure(format!(
                "open document exceeds {MAX_FILE_BYTES} byte analysis limit"
            )));
        }
        let old_names = self
            .documents
            .values()
            .flat_map(|record| {
                record
                    .exports
                    .iter()
                    .map(|item| item.definition.name.clone())
            })
            .collect();
        // Release every previous owner before parsing with an explicitly unknown namespace.
        for uri in self.revisions.keys().cloned().collect::<Vec<_>>() {
            let uri: Uri = uri
                .parse()
                .map_err(|error| failure(format!("invalid cached URI: {error}")))?;
            self.release_uri(host, project_uri, &uri)?;
        }
        let source = Source {
            view: view.clone(),
            witness: digest(&view.source),
            path: None,
        };
        config.structure.expression.function_namespace_complete = false;
        config.declarations = DeclarationInputs::default();
        let (result, record, revision) = self.parse(
            host,
            catalog,
            project_uri,
            &source,
            config,
            Vec::new(),
            snapshot.epoch,
            Some(observer),
        )?;
        let mut semantic = Arc::try_unwrap(record.semantic)
            .map_err(|_| failure("local semantic snapshot unexpectedly shared"))?;
        semantic.project_complete = false;
        let semantic = Arc::new(semantic);
        Ok(ProjectAnalysis {
            result,
            current_semantic: Arc::clone(&semantic),
            documents: vec![semantic],
            native_revision: revision,
            project_epoch: snapshot.epoch,
            changed_export_names: old_names,
            scanned_documents: 0,
            parsed_documents: 1,
            incomplete_reason: Some(reason),
        })
    }
    fn finish(
        &mut self,
        sources: BTreeMap<String, Source>,
        working: BTreeMap<String, Cached>,
        before: BTreeMap<String, String>,
        old_names: BTreeSet<String>,
        result: DocumentParseResult,
        current_semantic: Arc<semantic::Document>,
        revision: u64,
        epoch: u64,
        parsed_documents: usize,
        root: &Path,
    ) -> Result<ProjectAnalysis, ProjectError> {
        // A closed file changing during background work cannot publish a jump
        // location or signature from the earlier bytes.
        if fs::canonicalize(root).ok().as_deref() != Some(root) {
            return Err(incomplete("project root changed during analysis"));
        }
        for source in sources.values() {
            let path = source
                .path
                .clone()
                .map(Ok)
                .unwrap_or_else(|| uri_path(&source.view.uri))?;
            validate_member(&path, root)?;
            if let Some(path) = &source.path {
                if digest(&read_source(path)?) != source.witness {
                    return Err(incomplete(format!(
                        "project source changed during analysis: {}",
                        path.display()
                    )));
                }
            }
        }
        let changed = working
            .iter()
            .any(|(uri, record)| before.get(uri) != Some(&record.export_key))
            || before.keys().any(|uri| !working.contains_key(uri));
        let changed_export_names = if changed {
            old_names
                .into_iter()
                .chain(working.values().flat_map(|record| {
                    record
                        .exports
                        .iter()
                        .map(|item| item.definition.name.clone())
                }))
                .collect()
        } else {
            BTreeSet::new()
        };
        let documents = working
            .values()
            .filter(|record|record.semantic.is_current())
            .map(|record| Arc::clone(&record.semantic))
            .collect();
        self.documents = working;
        Ok(ProjectAnalysis {
            result,
            current_semantic,
            documents,
            native_revision: revision,
            project_epoch: epoch,
            changed_export_names,
            scanned_documents: sources.len(),
            parsed_documents,
            incomplete_reason: None,
        })
    }
}

fn require_stable_project(documents: usize, stable: bool) -> Result<(), ProjectError> {
    if documents > 1 && !stable {
        return Err(failure(
            "cross-document analysis requires every enabled component to declare a supported parser.project-stable-semantics contract (v1 or v2); current component state has no project stability contract",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "project_tests.rs"]
mod tests;

//! Worker-owned project snapshots. Open text overrides disk; accepted parser
//! definitions, including failed lookups, drive the project fixed point.
use super::semantic;
use lsp_types::Uri;
use parser_wasm::{
    DocumentParseError, DocumentParseRequest, DocumentParseResult, DocumentParserConfig, ParserHost,
};
use sha2::{Digest, Sha256};
use skript_parser::{ImportedFunction, StructurePreparationEvent};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
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
#[derive(Debug)]
pub(super) enum ProjectError {
    Cancelled,
    Failed(String),
}
impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("project analysis cancelled"),
            Self::Failed(message) => f.write_str(message),
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
    lookup_names: BTreeSet<String>,
    semantic: Arc<semantic::Document>,
}
#[derive(Default)]
pub(super) struct ProjectIndex {
    documents: BTreeMap<String, Cached>,
    next_native_revision: u64,
    revisions: BTreeMap<String, u64>,
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
fn read_source(path: &Path) -> Result<Arc<str>, ProjectError> {
    let before = fs::metadata(path).map_err(|error| {
        failure(format!(
            "cannot read project source {}: {error}",
            path.display()
        ))
    })?;
    if before.len() > MAX_FILE_BYTES as u64 {
        return Err(failure(format!(
            "project source exceeds {MAX_FILE_BYTES} byte analysis limit: {}",
            path.display()
        )));
    }
    let bytes = fs::read(path).map_err(|error| {
        failure(format!(
            "cannot read project source {}: {error}",
            path.display()
        ))
    })?;
    let after = fs::metadata(path).map_err(|error| {
        failure(format!(
            "cannot recheck project source {}: {error}",
            path.display()
        ))
    })?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(failure(format!(
            "project source changed while reading: {}",
            path.display()
        )));
    }
    let source = String::from_utf8(bytes)
        .map_err(|_| failure(format!("project source is not UTF-8: {}", path.display())))?;
    Ok(Arc::from(source))
}
fn sources(
    snapshot: &ProjectSnapshot,
    cancel: &parser_wasm::DocumentCancellationToken,
) -> Result<BTreeMap<String, Source>, ProjectError> {
    let root = fs::canonicalize(&snapshot.root).map_err(|error| {
        failure(format!(
            "cannot resolve project root {}: {error}",
            snapshot.root.display()
        ))
    })?;
    if !root.is_dir() {
        return Err(failure("project root is not a directory"));
    }
    let mut open = BTreeMap::new();
    for view in &snapshot.open {
        let path = canonical_source(&uri_path(&view.uri)?, &root)?;
        if open.insert(path, view.clone()).is_some() {
            return Err(failure("two open project URIs resolve to the same file"));
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
            return Err(failure(format!(
                "project directory nesting exceeds {MAX_DEPTH}"
            )));
        }
        let mut children = Vec::new();
        for child in fs::read_dir(&directory)
            .map_err(|error| failure(format!("cannot scan {}: {error}", directory.display())))?
        {
            if cancel.is_cancelled() {
                return Err(ProjectError::Cancelled);
            }
            entries += 1;
            if entries > MAX_ENTRIES {
                return Err(failure(format!(
                    "project scan exceeds {MAX_ENTRIES} directory entries"
                )));
            }
            children.push(child.map_err(|error| {
                failure(format!("cannot scan {}: {error}", directory.display()))
            })?);
        }
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let path = child.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| failure(format!("cannot inspect {}: {error}", path.display())))?;
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
                files.insert(canonical_source(&path, &root)?);
            }
        }
    }
    files.extend(open.keys().cloned());
    if files.len() > MAX_FILES {
        return Err(failure(format!(
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
            return Err(failure(format!(
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
        observer: Option<&mut dyn for<'p> FnMut(StructurePreparationEvent<'p>)>,
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
        let result = match observer {
            Some(observer) => host.parse_document_with_preparation(request, config, observer),
            None => host.parse_document(request, config),
        }?;
        let semantic = Arc::new(semantic::Document::from_result(
            &result,
            Arc::clone(&source.view.source),
            source.view.uri.clone(),
            catalog,
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
        Ok((
            result,
            Cached {
                source: source.clone(),
                exports,
                export_key,
                imported_key,
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
        observer: &mut dyn for<'p> FnMut(StructurePreparationEvent<'p>),
    ) -> Result<ProjectAnalysis, ProjectError> {
        let discovered = sources(snapshot, &config.cancellation)?;
        require_stable_project(discovered.len(), host.project_semantics_are_stable())?;
        let participants = host.components();
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
                    .filter(|source| source.same(&record.source))
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
            let (result, record, revision) = self.parse(
                host,
                Arc::clone(&catalog),
                project_uri,
                current,
                config,
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
            );
        }
        let mut order = vec![current_uri.as_str().to_owned()];
        order.extend(
            discovered
                .keys()
                .filter(|uri| uri.as_str() != current_uri.as_str())
                .cloned(),
        );
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
                let key = working
                    .get(uri)
                    .map(|record| relevant_key(&imported, &record.lookup_names));
                let needs = working
                    .get(uri)
                    .is_none_or(|record| Some(&record.imported_key) != key.as_ref());
                if !needs {
                    continue;
                }
                if parsed_documents >= MAX_PARSES {
                    return Err(failure(
                        "project function analysis exceeded its parse budget",
                    ));
                }
                let (_, record, _) = self.parse(
                    host,
                    Arc::clone(&catalog),
                    project_uri,
                    source,
                    config.clone(),
                    imported,
                    snapshot.epoch,
                    None,
                )?;
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
        let expected = working
            .get(current_uri.as_str())
            .map(|record| record.export_key.clone())
            .unwrap_or_default();
        let imported = imports(&working, current_uri.as_str());
        let (result, record, revision) = self.parse(
            host,
            Arc::clone(&catalog),
            project_uri,
            current,
            config,
            imported,
            snapshot.epoch,
            Some(observer),
        )?;
        parsed_documents += 1;
        // A component can disable itself after a trap during these parses.
        // Earlier side-document facts then describe a different environment.
        require_stable_project(discovered.len(), host.project_semantics_are_stable())?;
        if host.components() != participants {
            return Err(failure(
                "project component environment changed during analysis",
            ));
        }
        if record.export_key != expected {
            return Err(failure(
                "current function declarations changed after project stabilization; extension state is not a stable project input",
            ));
        }
        let current_semantic = Arc::clone(&record.semantic);
        working.insert(current_uri.as_str().to_owned(), record);
        self.finish(
            discovered,
            working,
            before,
            old_names,
            result,
            current_semantic,
            revision,
            snapshot.epoch,
            parsed_documents,
        )
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
    ) -> Result<ProjectAnalysis, ProjectError> {
        // A closed file changing during background work cannot publish a jump
        // location or signature from the earlier bytes.
        for source in sources.values() {
            if let Some(path) = &source.path {
                if digest(&read_source(path)?) != source.witness {
                    return Err(failure(format!(
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
        })
    }
}

fn require_stable_project(documents: usize, stable: bool) -> Result<(), ProjectError> {
    if documents > 1 && !stable {
        return Err(failure(
            "cross-document analysis requires every enabled component to declare parser.project-stable-semantics v1; current component state has no project stability contract",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "project_tests.rs"]
mod tests;

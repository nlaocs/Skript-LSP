use crate::args::snapshot_directory;
use crate::event_context::{
    EventContext, EventContextComponentFailure, EventContextDiagnostic, EventSummary,
    event_summaries, normalize_event_header,
};
use crate::report::{
    AnalysisReport, DocumentAnalysisReport, SnapshotDescription, diagnostic_severity_name,
};
use crate::section_context::{
    SectionContext, SectionContextComponentFailure, SectionContextDiagnostic,
    normalize_section_header,
};
use parser_wasm::host::{
    HostConfig, InvocationContext, ParserHost, RuntimePlugin, RuntimeProfile,
    apply_parser_context_updates,
};
use parser_wasm::state::StateSavepoint;
use parser_wasm::{DocumentParseRequest, DocumentParserConfig, ParseTransaction};
use skript_parser::{
    EffectParseRequest, EffectParserConfig, ExpressionParseContext, MappedSource,
    PatternFailureReason, RawNodeKind, RawTreeOptions, SectionParseRequest, SectionParserConfig,
    StructureDocumentNode, StructureParseRequest, StructureParserConfig, parse_raw_tree,
};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use thiserror::Error;

const PROJECT_URI: &str = "file:///skript-repl";
const DOCUMENT_URI: &str = "file:///skript-repl/input.sk";
const CONTEXT_DOCUMENTS: [&str; 2] = [
    "file:///skript-repl/event-context-0.sk",
    "file:///skript-repl/event-context-1.sk",
];
const EVENT_LIST_DOCUMENT: &str = "file:///skript-repl/event-list.sk";

/// Snapshot loading, input or Event-context validation, parser-host, or transaction failure.
#[derive(Debug, Error)]
pub enum SkriptSessionError {
    /// The SSG snapshot could not be loaded or validated.
    #[error("failed to load SSG snapshot {path}: {source}")]
    Snapshot {
        path: PathBuf,
        #[source]
        source: ssg::SnapshotError,
    },
    /// An optional parser addon component could not be read.
    #[error("failed to read parser addon {path}: {source}")]
    AddonRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// The manifest does not identify the Skript plugin used for lexical compatibility.
    #[error("SSG snapshot has no enabled Skript plugin entry")]
    MissingSkriptPlugin,
    /// The Skript version cannot provide the required major/minor pair.
    #[error("cannot parse Skript version {version:?} from the snapshot manifest")]
    InvalidSkriptVersion { version: String },
    /// One-shot Effect input does not contain exactly one simple Skript line.
    #[error("invalid Effect input: {message}")]
    InvalidInput { message: String },
    /// An Event header could not establish an Event context.
    #[error("invalid Event context: {message}")]
    InvalidEventContext { message: String },
    /// A Section header could not establish a Section body context.
    #[error("invalid Section context: {message}")]
    InvalidSectionContext { message: String },
    /// The parser worker could not be created with its bounded stack.
    #[error("failed to start parser worker: {source}")]
    ParserThread {
        #[source]
        source: io::Error,
    },
    /// The parser worker panicked instead of returning a typed failure.
    #[error("parser worker panicked")]
    ParserThreadPanicked,
    /// CoreLibrary or the transactional parser pipeline failed.
    #[error(transparent)]
    Host(#[from] parser_wasm::HostError),
    /// A parse transaction could not be closed after analysis.
    #[error(transparent)]
    State(#[from] parser_wasm::StateError),
    /// The end-to-end document pipeline failed before producing a report.
    #[error(transparent)]
    Document(#[from] parser_wasm::DocumentParseError),
}

/// Reusable SSG catalog and WASM parser host for one-line and document analysis.
///
/// Loading performs full schema and digest validation once. Successive calls to
/// [`Self::analyze_effect`] reuse the catalog and CoreLibrary host. Without a retained
/// Event or Section context, each analysis uses a new document revision and
/// cancels its speculative transaction. With a context, it restores a savepoint
/// in the retained transaction. Both paths discard analysis state before
/// constructing the report and never execute an Effect.
pub struct SkriptSession {
    snapshot_path: PathBuf,
    addon_paths: Vec<PathBuf>,
    snapshot: SnapshotDescription,
    skript_version: (u32, u32),
    catalog: Arc<syntaxes::Catalog>,
    host: ParserHost,
    next_revision: u64,
    event_context: Option<EventContext>,
    section_contexts: Vec<SelectedSection>,
    context_transaction: Option<ParseTransaction>,
}

#[derive(Clone, Copy)]
enum EffectStatePolicy {
    Discard,
    RetainMatched,
}

struct SelectedSection {
    context: SectionContext,
    baseline: StateSavepoint,
}

impl SkriptSession {
    /// Loads and validates an SSG schema 3 through 6 snapshot and initializes CoreLibrary.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, SkriptSessionError> {
        Self::load_with_addons(path, std::iter::empty::<PathBuf>())
    }

    /// Loads a snapshot and optional parser addon components in argument order.
    pub fn load_with_addons<P, I, A>(path: P, addons: I) -> Result<Self, SkriptSessionError>
    where
        P: AsRef<Path>,
        I: IntoIterator<Item = A>,
        A: AsRef<Path>,
    {
        let snapshot_path = snapshot_directory(path);
        let addon_paths = addons
            .into_iter()
            .map(|path| path.as_ref().to_owned())
            .collect::<Vec<_>>();
        let loaded = ssg::load(&snapshot_path).map_err(|source| SkriptSessionError::Snapshot {
            path: snapshot_path.clone(),
            source,
        })?;
        let manifest = loaded.manifest();
        let skript_plugin = manifest
            .plugins
            .iter()
            .find(|plugin| plugin.enabled && plugin.name.eq_ignore_ascii_case("Skript"))
            .ok_or(SkriptSessionError::MissingSkriptPlugin)?;
        let skript_version = parse_skript_version(&skript_plugin.version)?;
        let snapshot = SnapshotDescription {
            snapshot_id: manifest.snapshot_id.clone(),
            minecraft_version: manifest.server.minecraft_version.clone(),
            skript_version: skript_plugin.version.clone(),
            plugin_count: manifest
                .plugins
                .iter()
                .filter(|plugin| plugin.enabled)
                .count(),
        };
        let runtime_profile = RuntimeProfile {
            snapshot_schema_version: Some(manifest.schema_version),
            snapshot_id: Some(manifest.snapshot_id.clone()),
            server_name: Some(manifest.server.name.clone()),
            server_version: Some(manifest.server.version.clone()),
            minecraft_version: Some(manifest.server.minecraft_version.clone()),
            java_version: Some(manifest.server.java_version.clone()),
            language: Some(manifest.language.clone()),
            skript_version: Some(skript_plugin.version.clone()),
            plugins: manifest
                .plugins
                .iter()
                .filter(|plugin| plugin.enabled)
                .map(|plugin| RuntimePlugin {
                    load_order: plugin.load_order,
                    name: plugin.name.clone(),
                    version: plugin.version.clone(),
                    main: plugin.main.clone(),
                })
                .collect(),
            snapshot_capabilities: None,
        };
        let catalog = Arc::new(loaded.into_catalog());
        let mut host = skript_lsp::new_parser_host(HostConfig {
            syntax_catalog: Some(Arc::clone(&catalog)),
            runtime_profile,
            ..HostConfig::default()
        })?;
        for addon_path in &addon_paths {
            let component =
                fs::read(addon_path).map_err(|source| SkriptSessionError::AddonRead {
                    path: addon_path.clone(),
                    source,
                })?;
            host.load_addon(&component)?;
        }
        Ok(Self {
            snapshot_path,
            addon_paths,
            snapshot,
            skript_version,
            catalog,
            host,
            next_revision: 1,
            event_context: None,
            section_contexts: Vec::new(),
            context_transaction: None,
        })
    }

    /// Reloads the configured snapshot and rebuilds the catalog and parser host.
    pub fn reload(&mut self) -> Result<(), SkriptSessionError> {
        let replacement =
            Self::load_with_addons(self.snapshot_path.clone(), self.addon_paths.clone())?;
        if let Some(transaction) = &self.context_transaction {
            transaction.cancel()?;
        }
        *self = replacement;
        Ok(())
    }

    /// Returns the directory reloaded by the REPL's `:reload` command.
    pub fn snapshot_path(&self) -> &Path {
        &self.snapshot_path
    }

    /// Returns the Event context inherited by subsequent Effect parses.
    pub fn event_context(&self) -> Option<&EventContext> {
        self.event_context.as_ref()
    }

    /// Returns active Section contexts from the outermost to the innermost.
    pub fn section_contexts(&self) -> impl ExactSizeIterator<Item = &SectionContext> {
        self.section_contexts
            .iter()
            .map(|selected| &selected.context)
    }

    /// Clears the current Event context.
    pub fn clear_event_context(&mut self) -> Result<(), SkriptSessionError> {
        if let Some(transaction) = &self.context_transaction {
            transaction.cancel()?;
        }
        self.context_transaction = None;
        self.event_context = None;
        self.section_contexts.clear();
        Ok(())
    }

    /// Removes the innermost Section context and rolls back its addon state.
    pub fn pop_section_context(&mut self) -> Result<Option<SectionContext>, SkriptSessionError> {
        let Some(selected) = self.section_contexts.last() else {
            return Ok(None);
        };
        let transaction = self
            .context_transaction
            .as_ref()
            .expect("selected Sections always retain their parse transaction")
            .clone();
        transaction.rollback_to(&selected.baseline)?;
        if self.section_contexts.len() == 1 && self.event_context.is_none() {
            transaction.cancel()?;
            self.context_transaction = None;
        }
        Ok(self.section_contexts.pop().map(|selected| selected.context))
    }

    /// Clears every active Section while retaining the selected Event context.
    pub fn clear_section_contexts(&mut self) -> Result<(), SkriptSessionError> {
        let Some(first) = self.section_contexts.first() else {
            return Ok(());
        };
        let transaction = self
            .context_transaction
            .as_ref()
            .expect("selected Sections always retain their parse transaction");
        if self.event_context.is_some() {
            transaction.rollback_to(&first.baseline)?;
        } else {
            transaction.cancel()?;
            self.context_transaction = None;
        }
        self.section_contexts.clear();
        Ok(())
    }

    /// Lists static catalog and dynamically registered Events in parser order.
    pub fn events(&mut self) -> Result<Vec<EventSummary>, SkriptSessionError> {
        let (transaction, temporary) = if let Some(transaction) = &self.context_transaction {
            (transaction.clone(), false)
        } else {
            let revision = self.next_revision;
            self.next_revision = self.next_revision.saturating_add(1);
            (
                self.host
                    .begin_parse(PROJECT_URI, EVENT_LIST_DOCUMENT, revision)?,
                true,
            )
        };
        let snapshot = self.host.dynamic_syntax_snapshot(&transaction);
        let close = temporary.then(|| transaction.cancel());
        let snapshot = snapshot?;
        if let Some(close) = close {
            close?;
        }
        Ok(event_summaries(self.catalog.as_ref(), Some(&snapshot)))
    }

    /// Parses an Event header through StructEvent and installs its body context.
    pub fn select_event_header(
        &mut self,
        input: &str,
    ) -> Result<&EventContext, SkriptSessionError> {
        let input = normalize_event_header(input).map_err(invalid_event)?;
        let (selected, transaction) = std::thread::scope(|scope| {
            let worker = std::thread::Builder::new()
                .name("skript-repl-event-parser".to_owned())
                .stack_size(32 * 1024 * 1024)
                .spawn_scoped(scope, || self.select_event_header_inner(input))
                .map_err(|source| SkriptSessionError::ParserThread { source })?;
            worker
                .join()
                .map_err(|_| SkriptSessionError::ParserThreadPanicked)?
        })?;
        if let Some(previous) = &self.context_transaction
            && let Err(error) = previous.cancel()
        {
            let _ = transaction.cancel();
            return Err(error.into());
        }
        self.event_context = Some(selected);
        self.section_contexts.clear();
        self.context_transaction = Some(transaction);
        Ok(self
            .event_context
            .as_ref()
            .expect("the selected Event context was just stored"))
    }

    /// Parses a Section header and pushes its body context.
    ///
    /// A trailing colon is accepted but optional because the CLI synthesizes
    /// the physical Section line before invoking the normal Section parser.
    pub fn select_section_header(
        &mut self,
        input: &str,
    ) -> Result<&SectionContext, SkriptSessionError> {
        let input = normalize_section_header(input).map_err(invalid_section)?;
        let (selected, baseline, transaction) = std::thread::scope(|scope| {
            let worker = std::thread::Builder::new()
                .name("skript-repl-section-parser".to_owned())
                .stack_size(32 * 1024 * 1024)
                .spawn_scoped(scope, || self.select_section_header_inner(input))
                .map_err(|source| SkriptSessionError::ParserThread { source })?;
            worker
                .join()
                .map_err(|_| SkriptSessionError::ParserThreadPanicked)?
        })?;
        if self.context_transaction.is_none() {
            self.context_transaction = Some(transaction);
        }
        self.section_contexts.push(SelectedSection {
            context: selected,
            baseline,
        });
        Ok(&self
            .section_contexts
            .last()
            .expect("the selected Section context was just stored")
            .context)
    }

    /// Parses one complete simple line as an Effect and returns a display report.
    pub fn analyze_effect(&mut self, input: &str) -> Result<AnalysisReport, SkriptSessionError> {
        self.analyze_effect_with_policy(input, EffectStatePolicy::Discard)
    }

    /// Parses one REPL Effect and retains state written by its selected branch.
    ///
    /// Failed and incomplete candidates are rolled back. A successful parse
    /// commits a temporary transaction, or keeps its writes in the active
    /// Event/Section transaction so later REPL inputs can observe them.
    pub(crate) fn analyze_repl_effect(
        &mut self,
        input: &str,
    ) -> Result<AnalysisReport, SkriptSessionError> {
        self.analyze_effect_with_policy(input, EffectStatePolicy::RetainMatched)
    }

    fn analyze_effect_with_policy(
        &mut self,
        input: &str,
        state_policy: EffectStatePolicy,
    ) -> Result<AnalysisReport, SkriptSessionError> {
        std::thread::scope(|scope| {
            let worker = std::thread::Builder::new()
                .name("skript-repl-effect-parser".to_owned())
                .stack_size(32 * 1024 * 1024)
                .spawn_scoped(scope, || self.analyze_inner(input, state_policy))
                .map_err(|source| SkriptSessionError::ParserThread { source })?;
            worker
                .join()
                .map_err(|_| SkriptSessionError::ParserThreadPanicked)?
        })
    }

    /// Parses and commits one complete Skript document revision.
    ///
    /// The same host, project URI, and monotonically increasing document
    /// revision are reused for the lifetime of this session. This lets parser
    /// addons retain project/document state between submitted REPL inputs while
    /// keeping unsubmitted and cancelled drafts outside the parser entirely.
    pub fn analyze_document(
        &mut self,
        input: &str,
    ) -> Result<DocumentAnalysisReport, SkriptSessionError> {
        if input.trim().is_empty() {
            return Err(SkriptSessionError::InvalidInput {
                message: "Skript document is empty".to_owned(),
            });
        }
        std::thread::scope(|scope| {
            let worker = std::thread::Builder::new()
                .name("skript-repl-document-parser".to_owned())
                .stack_size(32 * 1024 * 1024)
                .spawn_scoped(scope, || self.analyze_document_inner(input))
                .map_err(|source| SkriptSessionError::ParserThread { source })?;
            worker
                .join()
                .map_err(|_| SkriptSessionError::ParserThreadPanicked)?
        })
    }

    pub(crate) fn raw_tree_options(&self) -> RawTreeOptions {
        RawTreeOptions::for_skript_version(self.skript_version.0, self.skript_version.1)
    }

    fn analyze_document_inner(
        &mut self,
        input: &str,
    ) -> Result<DocumentAnalysisReport, SkriptSessionError> {
        let started = Instant::now();
        let revision = self.next_revision;
        self.next_revision = self.next_revision.saturating_add(1);
        let result = self.host.parse_document(
            DocumentParseRequest::new(PROJECT_URI, DOCUMENT_URI, revision, input),
            DocumentParserConfig {
                raw_tree: Some(self.raw_tree_options()),
                ..DocumentParserConfig::default()
            },
        )?;
        Ok(DocumentAnalysisReport::from_result(
            input,
            &self.snapshot,
            result,
            self.catalog.as_ref(),
            started.elapsed(),
        ))
    }

    fn analyze_inner(
        &mut self,
        input: &str,
        state_policy: EffectStatePolicy,
    ) -> Result<AnalysisReport, SkriptSessionError> {
        if input.trim().is_empty() {
            return Err(SkriptSessionError::InvalidInput {
                message: "Effect text is empty".to_owned(),
            });
        }
        let started = Instant::now();
        let source = MappedSource::identity(input);
        let tree = parse_raw_tree(
            &source,
            RawTreeOptions::for_skript_version(self.skript_version.0, self.skript_version.1),
        );
        if tree.roots.len() != 1 {
            let diagnostics = tree
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            let detail = if diagnostics.is_empty() {
                format!(
                    "expected one Effect line, found {} root nodes",
                    tree.roots.len()
                )
            } else {
                format!(
                    "expected one Effect line, found {} root nodes ({diagnostics})",
                    tree.roots.len()
                )
            };
            return Err(SkriptSessionError::InvalidInput { message: detail });
        }
        let node = tree
            .get(tree.roots[0])
            .expect("RawTree roots always refer to arena nodes");
        if node.kind != RawNodeKind::Simple {
            return Err(SkriptSessionError::InvalidInput {
                message: format!("expected a simple Effect line, found {:?}", node.kind),
            });
        }

        let invocation_id = self.next_revision;
        self.next_revision = self.next_revision.saturating_add(1);
        let (transaction, baseline) = if let Some(transaction) = &self.context_transaction {
            let transaction = transaction.clone();
            let baseline = transaction.savepoint()?;
            (transaction, Some(baseline))
        } else {
            (
                self.host
                    .begin_parse(PROJECT_URI, DOCUMENT_URI, invocation_id)?,
                None,
            )
        };
        let document_id = transaction.document_id()?;
        let document_revision = transaction.document_revision()?;
        let result = self.host.parse_effect_in_parse(
            &transaction,
            invocation_context(invocation_id, &document_id, document_revision),
            EffectParseRequest {
                source: &source,
                node,
                context: self.active_parser_context(),
            },
            EffectParserConfig::default(),
        );
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                baseline.map_or_else(
                    || transaction.cancel(),
                    |baseline| transaction.rollback_to(&baseline),
                )?;
                return Err(error.into());
            }
        };
        let retain = matches!(state_policy, EffectStatePolicy::RetainMatched)
            && result.matches.selected.is_some();
        match (baseline, retain) {
            (Some(_), true) => {}
            (Some(baseline), false) => transaction.rollback_to(&baseline)?,
            (None, true) => {
                transaction.commit()?;
            }
            (None, false) => transaction.cancel()?,
        }
        let parse_duration = started.elapsed();
        Ok(AnalysisReport::from_result(
            input,
            &self.snapshot,
            result,
            self.catalog.as_ref(),
            parse_duration,
            self.event_context.as_ref(),
            self.section_contexts(),
        ))
    }

    fn active_parser_context(&self) -> ExpressionParseContext {
        self.section_contexts
            .last()
            .map(|selected| selected.context.parser_context.clone())
            .or_else(|| {
                self.event_context
                    .as_ref()
                    .map(|event| event.parser_context.clone())
            })
            .unwrap_or_default()
    }

    fn select_section_header_inner(
        &mut self,
        input: String,
    ) -> Result<(SectionContext, StateSavepoint, ParseTransaction), SkriptSessionError> {
        let source = MappedSource::identity(format!("{input}:\n"));
        let tree = parse_raw_tree(
            &source,
            RawTreeOptions::for_skript_version(self.skript_version.0, self.skript_version.1),
        );
        if tree.roots.len() != 1 {
            return Err(invalid_section(format!(
                "expected one Section header, found {} root nodes",
                tree.roots.len()
            )));
        }
        let node = tree
            .get(tree.roots[0])
            .expect("RawTree roots always refer to arena nodes");
        if node.kind != RawNodeKind::Section {
            return Err(invalid_section("Section header did not form a Section"));
        }

        let invocation_id = self.next_revision;
        self.next_revision = self.next_revision.saturating_add(1);
        let created = self.context_transaction.is_none();
        let transaction = if let Some(transaction) = &self.context_transaction {
            transaction.clone()
        } else {
            self.host
                .begin_parse(PROJECT_URI, CONTEXT_DOCUMENTS[0], invocation_id)?
        };
        let baseline = transaction.savepoint()?;
        let document_id = transaction.document_id()?;
        let document_revision = transaction.document_revision()?;
        let parsed = self.host.parse_section_in_parse(
            &transaction,
            invocation_context(invocation_id, &document_id, document_revision),
            SectionParseRequest {
                source: &source,
                tree: &tree,
                node,
                context: self.active_parser_context(),
            },
            SectionParserConfig {
                root_lifecycle: skript_parser::SectionRootLifecycle::RetainBody,
                ..SectionParserConfig::default()
            },
        );
        let result = match parsed {
            Ok(result) => result,
            Err(error) => {
                transaction.rollback_to(&baseline)?;
                if created {
                    transaction.cancel()?;
                }
                return Err(error.into());
            }
        };
        let diagnostics = result
            .effects
            .diagnostics
            .iter()
            .map(|diagnostic| SectionContextDiagnostic {
                code: diagnostic.code.clone(),
                message: diagnostic.message.clone(),
                severity: diagnostic_severity_name(diagnostic.severity).to_owned(),
            })
            .collect::<Vec<_>>();
        let component_failures = result
            .failures
            .iter()
            .map(|failure| SectionContextComponentFailure {
                component_id: failure.component_id.clone(),
                subscription_id: failure.subscription_id.clone(),
                message: failure.error.to_string(),
            })
            .collect::<Vec<_>>();
        let candidate = match result.matches.selected {
            Some(candidate) => candidate,
            None => {
                let rejection = result
                    .matches
                    .unknown
                    .as_ref()
                    .and_then(|unknown| unknown.failure.as_ref())
                    .and_then(|failure| {
                        failure.root_cause().failure.reasons.iter().find_map(
                            |reason| match reason {
                                PatternFailureReason::HookRejected { reason } => {
                                    Some(reason.as_str())
                                }
                                _ => None,
                            },
                        )
                    });
                transaction.rollback_to(&baseline)?;
                if created {
                    transaction.cancel()?;
                }
                return Err(invalid_section(rejection.map_or_else(
                    || format!("{input:?} does not match a registered Section"),
                    |reason| format!("{input:?} matched a Section but was rejected: {reason}"),
                )));
            }
        };
        let selected =
            match SectionContext::from_candidate(input, candidate, diagnostics, component_failures)
            {
                Ok(selected) => selected,
                Err(message) => {
                    transaction.rollback_to(&baseline)?;
                    if created {
                        transaction.cancel()?;
                    }
                    return Err(invalid_section(message));
                }
            };
        Ok((selected, baseline, transaction))
    }

    fn select_event_header_inner(
        &mut self,
        input: String,
    ) -> Result<(EventContext, ParseTransaction), SkriptSessionError> {
        let source = MappedSource::identity(format!("{input}:\n"));
        let tree = parse_raw_tree(
            &source,
            RawTreeOptions::for_skript_version(self.skript_version.0, self.skript_version.1),
        );
        if tree.roots.len() != 1 {
            return Err(invalid_event(format!(
                "expected one Event header, found {} root nodes",
                tree.roots.len()
            )));
        }
        let node = tree
            .get(tree.roots[0])
            .expect("RawTree roots always refer to arena nodes");
        if node.kind != RawNodeKind::Section {
            return Err(invalid_event("Event header did not form a Section"));
        }

        let revision = self.next_revision;
        self.next_revision = self.next_revision.saturating_add(1);
        let active_document_id = self
            .context_transaction
            .as_ref()
            .map(ParseTransaction::document_id)
            .transpose()?;
        let document_id = if active_document_id.as_deref() == Some(CONTEXT_DOCUMENTS[0]) {
            CONTEXT_DOCUMENTS[1]
        } else {
            CONTEXT_DOCUMENTS[0]
        };
        let transaction = self.host.begin_parse(PROJECT_URI, document_id, revision)?;
        let selected = (|| -> Result<EventContext, SkriptSessionError> {
            let result = self.host.parse_structures_in_parse(
                &transaction,
                invocation_context(revision, document_id, revision),
                StructureParseRequest {
                    source: &source,
                    tree: &tree,
                    context: ExpressionParseContext::default(),
                },
                StructureParserConfig {
                    headers_only: true,
                    ..StructureParserConfig::default()
                },
            )?;

            let parser_context = apply_parser_context_updates(
                &ExpressionParseContext::default(),
                &result.effects.context_updates,
            )
            .map_err(invalid_event)?;
            let diagnostics = result
                .effects
                .diagnostics
                .iter()
                .map(|diagnostic| EventContextDiagnostic {
                    code: diagnostic.code.clone(),
                    message: diagnostic.message.clone(),
                    severity: diagnostic_severity_name(diagnostic.severity).to_owned(),
                })
                .collect::<Vec<_>>();
            let component_failures = result
                .failures
                .iter()
                .map(|failure| EventContextComponentFailure {
                    component_id: failure.component_id.clone(),
                    subscription_id: failure.subscription_id.clone(),
                    message: failure.error.to_string(),
                })
                .collect::<Vec<_>>();
            let structure_diagnostics = result
                .document
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.clone())
                .collect::<Vec<_>>();
            let matches = result
                .document
                .roots
                .into_iter()
                .find_map(|root| match root {
                    StructureDocumentNode::Structure(matches) => Some(matches),
                    StructureDocumentNode::Trivia(_) | StructureDocumentNode::Unclaimed(_) => None,
                })
                .ok_or_else(|| {
                    invalid_event(format!("{input:?} does not match a registered Event"))
                })?;
            // Only the parser-selected Structure establishes runtime context. An
            // Event alternative may have matched textually but lost Skript's
            // Structure ordering or an addon hook, so accepting it here would
            // create a context that the real parser would not enter.
            let structure = match matches.selected {
                Some(structure) => structure,
                None => {
                    let rejection = matches
                        .unknown
                        .as_ref()
                        .and_then(|unknown| unknown.failure.as_ref())
                        .and_then(|failure| {
                            failure
                                .root_cause()
                                .failure
                                .reasons
                                .iter()
                                .find_map(|reason| match reason {
                                    PatternFailureReason::HookRejected { reason } => {
                                        Some(reason.as_str())
                                    }
                                    _ => None,
                                })
                        });
                    return Err(invalid_event(rejection.map_or_else(
                        || format!("{input:?} does not match a registered Event"),
                        |reason| format!("{input:?} matched an Event but was rejected: {reason}"),
                    )));
                }
            };
            if !identifies_event_structure(
                &structure.metadata,
                structure.element_class.as_ref().map(|class| class.as_str()),
            ) {
                return Err(invalid_event(format!(
                    "{input:?} matched a non-Event Structure ({})",
                    structure.matched.pattern
                )));
            }
            let event = structure
                .parsed_captures
                .into_iter()
                .find_map(|capture| match capture.result.value {
                    Some(skript_parser::ParsedCaptureValue::Event(event)) => Some(*event),
                    _ => None,
                })
                .ok_or_else(|| {
                    invalid_event("StructEvent did not retain its parsed Event capture")
                })?;
            if parser_context.event_classes.is_empty() {
                let details = diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.message.as_str())
                    .chain(structure_diagnostics.iter().map(String::as_str))
                    .chain(
                        component_failures
                            .iter()
                            .map(|failure| failure.message.as_str()),
                    )
                    .collect::<Vec<_>>()
                    .join("; ");
                return Err(invalid_event(if details.is_empty() {
                    "StructEvent did not establish any reference Event classes".to_owned()
                } else {
                    format!("StructEvent did not establish any reference Event classes: {details}")
                }));
            }
            Ok(EventContext::from_candidate(
                self.catalog.as_ref(),
                input,
                event,
                parser_context,
                structure.metadata,
                diagnostics,
                component_failures,
            ))
        })();
        match selected {
            Ok(context) => Ok((context, transaction)),
            Err(error) => {
                transaction.cancel()?;
                Err(error)
            }
        }
    }
}

fn invocation_context(
    invocation_id: u64,
    document_id: &str,
    document_revision: u64,
) -> InvocationContext {
    InvocationContext {
        invocation_id,
        subscription_id: String::new(),
        document_id: document_id.to_owned(),
        document_revision,
        expansion: None,
        syntax_context: 0,
    }
}

fn invalid_event(message: impl Into<String>) -> SkriptSessionError {
    SkriptSessionError::InvalidEventContext {
        message: message.into(),
    }
}

fn invalid_section(message: impl Into<String>) -> SkriptSessionError {
    SkriptSessionError::InvalidSectionContext {
        message: message.into(),
    }
}

fn identifies_event_structure(
    metadata: &BTreeMap<String, String>,
    element_class: Option<&str>,
) -> bool {
    metadata.iter().any(|(key, value)| {
        (key == "semantic-mode" || key.ends_with("/semantic-mode")) && value == "event-structure"
    }) || element_class.is_some_and(|class| class.ends_with(".StructEvent"))
}

fn parse_skript_version(version: &str) -> Result<(u32, u32), SkriptSessionError> {
    let numeric = version
        .trim()
        .strip_prefix('v')
        .unwrap_or(version.trim())
        .split(|character: char| !character.is_ascii_digit() && character != '.')
        .next()
        .unwrap_or_default();
    let mut parts = numeric.split('.');
    let major = parts.next().and_then(|part| part.parse().ok());
    let minor = parts.next().and_then(|part| part.parse().ok());
    major
        .zip(minor)
        .ok_or_else(|| SkriptSessionError::InvalidSkriptVersion {
            version: version.to_owned(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_release_and_prefixed_skript_versions() {
        assert_eq!(parse_skript_version("2.15.4").unwrap(), (2, 15));
        assert_eq!(parse_skript_version("v2.6.4-SNAPSHOT").unwrap(), (2, 6));
    }

    #[test]
    fn rejects_versions_without_major_and_minor() {
        assert!(matches!(
            parse_skript_version("development"),
            Err(SkriptSessionError::InvalidSkriptVersion { .. })
        ));
    }

    #[test]
    fn event_structure_metadata_requires_the_semantic_mode_key() {
        assert!(identifies_event_structure(
            &BTreeMap::from([("semantic-mode".to_owned(), "event-structure".to_owned())]),
            None,
        ));
        assert!(identifies_event_structure(
            &BTreeMap::from([(
                "addon.example/semantic-mode".to_owned(),
                "event-structure".to_owned(),
            )]),
            None,
        ));
        assert!(!identifies_event_structure(
            &BTreeMap::from([("addon-note".to_owned(), "event-structure".to_owned())]),
            None,
        ));
        assert!(identifies_event_structure(
            &BTreeMap::new(),
            Some("ch.njol.skript.structures.StructEvent"),
        ));
    }
}

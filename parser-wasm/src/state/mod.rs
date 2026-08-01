//! Transactional key/value state shared with WASM parser addons.
//!
//! Invocation overlays roll back rejected candidates and traps. Parse transactions
//! commit only after the current document revision completes, with scoped namespaces and quotas.
#![allow(missing_docs)] // WIT transport fields are documented as aggregate contracts.

mod persistent;

use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
};

use directories::ProjectDirs;
use sha2::{Digest, Sha256};
use url::Url;

use persistent::PersistentProject;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Lifetime and sharing boundary of one StateStore value.
pub enum StateScope {
    Invocation,
    Parse,
    Document,
    Project,
    PersistentProject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Access-control mode declared for a state namespace.
pub enum NamespaceVisibility {
    Private,
    Shared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Guest-selected payload encoding; the host treats bytes as opaque.
pub enum StateEncoding {
    Raw,
    Cbor,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Schema-tagged opaque value stored by an addon.
pub struct StateValue {
    pub schema_id: String,
    pub encoding: StateEncoding,
    pub bytes: Vec<u8>,
}

impl StateValue {
    /// Creates an opaque value tagged with its schema and encoding.
    pub fn new(
        schema_id: impl Into<String>,
        encoding: StateEncoding,
        bytes: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            schema_id: schema_id.into(),
            encoding,
            bytes: bytes.into(),
        }
    }

    fn stored_size(&self) -> usize {
        self.schema_id
            .len()
            .saturating_add(self.bytes.len())
            .saturating_add(1)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Key/value pair returned by a deterministic prefix scan.
pub struct StateEntry {
    pub key: String,
    pub value: StateValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Private or shared namespace schema and access-control declaration.
pub struct NamespaceDeclaration {
    pub name: String,
    pub visibility: NamespaceVisibility,
    pub schema_id: String,
    pub schema_version: u32,
    pub readers: BTreeSet<String>,
    pub writers: BTreeSet<String>,
}

impl NamespaceDeclaration {
    /// Declares a namespace accessible only to its owning component.
    pub fn private(
        name: impl Into<String>,
        schema_id: impl Into<String>,
        schema_version: u32,
    ) -> Self {
        Self {
            name: name.into(),
            visibility: NamespaceVisibility::Private,
            schema_id: schema_id.into(),
            schema_version,
            readers: BTreeSet::new(),
            writers: BTreeSet::new(),
        }
    }

    /// Declares a namespace with explicit component reader and writer sets.
    pub fn shared<R, W, RI, WI>(
        name: impl Into<String>,
        schema_id: impl Into<String>,
        schema_version: u32,
        readers: R,
        writers: W,
    ) -> Self
    where
        R: IntoIterator<Item = RI>,
        W: IntoIterator<Item = WI>,
        RI: Into<String>,
        WI: Into<String>,
    {
        Self {
            name: name.into(),
            visibility: NamespaceVisibility::Shared,
            schema_id: schema_id.into(),
            schema_version,
            readers: readers.into_iter().map(Into::into).collect(),
            writers: writers.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
/// Fully qualified record identity captured in a read/write set.
pub struct StateRecordKey {
    pub scope: StateScope,
    pub visibility: NamespaceVisibility,
    pub owner: Option<String>,
    pub namespace: String,
    pub key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
/// Fully qualified namespace identity and revision dependency.
pub struct StateNamespaceKey {
    pub scope: StateScope,
    pub visibility: NamespaceVisibility,
    pub owner: Option<String>,
    pub namespace: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Records and namespace revisions observed or changed by a transaction.
pub struct StateReadWriteSet {
    pub reads: BTreeSet<StateRecordKey>,
    pub writes: BTreeSet<StateRecordKey>,
    pub namespace_revisions: BTreeMap<StateNamespaceKey, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Per-key, per-value, per-namespace, and scan resource limits.
pub struct StateQuotas {
    pub max_key_bytes: usize,
    pub max_value_bytes: usize,
    pub max_namespace_bytes: usize,
    pub max_scan_entries: usize,
}

impl Default for StateQuotas {
    fn default() -> Self {
        Self {
            max_key_bytes: 4 * 1024,
            max_value_bytes: 1024 * 1024,
            max_namespace_bytes: 16 * 1024 * 1024,
            max_scan_entries: 1_024,
        }
    }
}

impl StateQuotas {
    fn validate(&self) -> Result<(), StateError> {
        if self.max_key_bytes == 0
            || self.max_value_bytes == 0
            || self.max_namespace_bytes == 0
            || self.max_scan_entries == 0
        {
            Err(StateError::InvalidInput {
                message: "every StateStore quota must be non-zero".to_owned(),
            })
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Default)]
/// Persistent location override and quotas for a StateStore.
pub struct StateStoreConfig {
    pub data_directory: Option<PathBuf>,
    pub quotas: StateQuotas,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
/// Input, access, quota, revision, transaction, or persistence failure.
#[allow(missing_docs)] // Variant messages describe the rejected state operation.
pub enum StateError {
    #[error("no StateStore transaction is active for this WASM invocation")]
    NoActiveTransaction,
    #[error("invalid StateStore input: {message}")]
    InvalidInput { message: String },
    #[error("unknown {visibility:?} namespace {namespace}")]
    UnknownNamespace {
        visibility: NamespaceVisibility,
        namespace: String,
    },
    #[error("component {component_id} may not {operation} namespace {namespace}")]
    AccessDenied {
        component_id: String,
        namespace: String,
        operation: &'static str,
    },
    #[error("namespace {namespace} requires schema {expected}, found {actual}")]
    SchemaMismatch {
        namespace: String,
        expected: String,
        actual: String,
    },
    #[error("StateStore quota exceeded: {message}")]
    QuotaExceeded { message: String },
    #[error(
        "document {document_id} revision {actual} is stale; latest started revision is {latest}"
    )]
    StaleDocumentRevision {
        document_id: String,
        actual: u64,
        latest: u64,
    },
    #[error("StateStore transaction conflicts with a newer namespace revision: {namespace}")]
    TransactionConflict { namespace: String },
    #[error("StateStore transaction is already closed")]
    TransactionClosed,
    #[error("savepoint belongs to another StateStore transaction")]
    ForeignSavepoint,
    #[error("persistent StateStore error: {message}")]
    Persistence { message: String },
    #[error("StateStore internal error: {message}")]
    Internal { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum NamespaceKey {
    Private { owner: String, name: String },
    Shared { name: String },
}

impl NamespaceKey {
    fn visibility(&self) -> NamespaceVisibility {
        match self {
            Self::Private { .. } => NamespaceVisibility::Private,
            Self::Shared { .. } => NamespaceVisibility::Shared,
        }
    }

    pub(super) fn owner(&self) -> Option<&str> {
        match self {
            Self::Private { owner, .. } => Some(owner),
            Self::Shared { .. } => None,
        }
    }

    pub(super) fn name(&self) -> &str {
        match self {
            Self::Private { name, .. } | Self::Shared { name } => name,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct NamespaceRecord {
    owner: String,
    pub(super) declaration: NamespaceDeclaration,
}

#[derive(Debug, Clone, Default)]
struct NamespaceRegistry {
    records: BTreeMap<NamespaceKey, NamespaceRecord>,
}

impl NamespaceRegistry {
    fn register(
        &mut self,
        component_id: &str,
        declarations: &[NamespaceDeclaration],
    ) -> Result<(), StateError> {
        validate_identifier("component ID", component_id)?;
        let mut names = BTreeSet::new();
        for declaration in declarations {
            validate_identifier("namespace", &declaration.name)?;
            validate_identifier("schema ID", &declaration.schema_id)?;
            if declaration.schema_version == 0 {
                return Err(StateError::InvalidInput {
                    message: format!(
                        "namespace {} must have a non-zero schema version",
                        declaration.name
                    ),
                });
            }
            if !names.insert((declaration.visibility, declaration.name.as_str())) {
                return Err(StateError::InvalidInput {
                    message: format!(
                        "component {component_id} declares namespace {} more than once",
                        declaration.name
                    ),
                });
            }
            if declaration.visibility == NamespaceVisibility::Private
                && (!declaration.readers.is_empty() || !declaration.writers.is_empty())
            {
                return Err(StateError::InvalidInput {
                    message: format!(
                        "private namespace {} cannot declare readers or writers",
                        declaration.name
                    ),
                });
            }
            for accessor in declaration.readers.iter().chain(declaration.writers.iter()) {
                validate_identifier("component access ID", accessor)?;
            }
            let key = match declaration.visibility {
                NamespaceVisibility::Private => NamespaceKey::Private {
                    owner: component_id.to_owned(),
                    name: declaration.name.clone(),
                },
                NamespaceVisibility::Shared => NamespaceKey::Shared {
                    name: declaration.name.clone(),
                },
            };
            if self.records.contains_key(&key) {
                return Err(StateError::InvalidInput {
                    message: format!("namespace {} is already declared", declaration.name),
                });
            }
            self.records.insert(
                key,
                NamespaceRecord {
                    owner: component_id.to_owned(),
                    declaration: declaration.clone(),
                },
            );
        }
        Ok(())
    }

    fn resolve(
        &self,
        component_id: &str,
        visibility: NamespaceVisibility,
        namespace: &str,
        write: bool,
    ) -> Result<(NamespaceKey, NamespaceRecord), StateError> {
        let key = match visibility {
            NamespaceVisibility::Private => NamespaceKey::Private {
                owner: component_id.to_owned(),
                name: namespace.to_owned(),
            },
            NamespaceVisibility::Shared => NamespaceKey::Shared {
                name: namespace.to_owned(),
            },
        };
        let record =
            self.records
                .get(&key)
                .cloned()
                .ok_or_else(|| StateError::UnknownNamespace {
                    visibility,
                    namespace: namespace.to_owned(),
                })?;
        let allowed = record.owner == component_id
            || if write {
                record.declaration.writers.contains(component_id)
            } else {
                record.declaration.readers.contains(component_id)
                    || record.declaration.writers.contains(component_id)
            };
        if !allowed {
            return Err(StateError::AccessDenied {
                component_id: component_id.to_owned(),
                namespace: namespace.to_owned(),
                operation: if write { "write" } else { "read" },
            });
        }
        Ok((key, record))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Address {
    pub(super) scope: StateScope,
    pub(super) namespace: NamespaceKey,
    pub(super) key: String,
}

impl Address {
    fn public(&self) -> StateRecordKey {
        StateRecordKey {
            scope: self.scope,
            visibility: self.namespace.visibility(),
            owner: self.namespace.owner().map(str::to_owned),
            namespace: self.namespace.name().to_owned(),
            key: self.key.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RevisionKey {
    scope: StateScope,
    namespace: NamespaceKey,
}

impl RevisionKey {
    fn public(&self) -> StateNamespaceKey {
        StateNamespaceKey {
            scope: self.scope,
            visibility: self.namespace.visibility(),
            owner: self.namespace.owner().map(str::to_owned),
            namespace: self.namespace.name().to_owned(),
        }
    }
}

#[derive(Default)]
struct ProjectState {
    documents: BTreeMap<String, BTreeMap<NamespaceKey, BTreeMap<String, StateValue>>>,
    project: BTreeMap<NamespaceKey, BTreeMap<String, StateValue>>,
    persistent: Option<PersistentProject>,
    revisions: BTreeMap<RevisionKey, u64>,
}

struct StateStoreInner {
    data_directory: PathBuf,
    quotas: StateQuotas,
    namespaces: NamespaceRegistry,
    projects: BTreeMap<String, ProjectState>,
    latest_document_revisions: BTreeMap<(String, String), u64>,
    next_transaction_id: u64,
}

#[derive(Clone)]
/// Thread-safe namespace registry and transaction coordinator.
///
/// State is isolated by project, scope, namespace visibility, and schema. A
/// component writes inside an [InvocationTransaction]; accepted invocation
/// writes are merged into a [ParseTransaction], and only a current completed
/// document revision publishes durable scopes. Dropping or rolling back the
/// invocation leaves the parent parse unchanged.
///
/// # Examples
///
/// The following stores a variable type in a private project namespace, commits
/// it, and reads it from the next document revision:
///
/// ~~~
/// use parser_wasm::state::{
///     NamespaceDeclaration, NamespaceVisibility, StateEncoding, StateScope,
///     StateStore, StateStoreConfig, StateValue,
/// };
///
/// let directory = tempfile::tempdir()?;
/// let store = StateStore::new(StateStoreConfig {
///     data_directory: Some(directory.path().to_owned()),
///     ..StateStoreConfig::default()
/// })?;
/// store.register_component(
///     "example.types",
///     &[NamespaceDeclaration::private(
///         "variables",
///         "example.variable-type",
///         1,
///     )],
/// )?;
///
/// let parse = store.begin_parse(
///     "file:///workspace",
///     "file:///workspace/main.sk",
///     1,
/// )?;
/// let mut invocation = parse.begin_invocation("example.types")?;
/// invocation.put(
///     StateScope::Project,
///     NamespaceVisibility::Private,
///     "variables",
///     "player-name",
///     StateValue::new(
///         "example.variable-type",
///         StateEncoding::Json,
///         br#"{"type":"string"}"#,
///     ),
/// )?;
/// invocation.commit()?;
///
/// let committed = parse.commit()?;
/// assert_eq!(committed.writes, 1);
///
/// let next_parse = store.begin_parse(
///     "file:///workspace",
///     "file:///workspace/main.sk",
///     2,
/// )?;
/// let mut reader = next_parse.begin_invocation("example.types")?;
/// let value = reader.get(
///     StateScope::Project,
///     NamespaceVisibility::Private,
///     "variables",
///     "player-name",
/// )?.expect("committed project value exists");
/// assert_eq!(value.bytes, br#"{"type":"string"}"#);
/// reader.rollback();
/// next_parse.cancel()?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ~~~
pub struct StateStore {
    inner: Arc<Mutex<StateStoreInner>>,
}

impl StateStore {
    /// Opens the transactional store and its optional persistent backend.
    pub fn new(config: StateStoreConfig) -> Result<Self, StateError> {
        config.quotas.validate()?;
        let data_directory = match config.data_directory {
            Some(path) => path,
            None => default_data_directory()?,
        };
        Ok(Self {
            inner: Arc::new(Mutex::new(StateStoreInner {
                data_directory,
                quotas: config.quotas,
                namespaces: NamespaceRegistry::default(),
                projects: BTreeMap::new(),
                latest_document_revisions: BTreeMap::new(),
                next_transaction_id: 1,
            })),
        })
    }

    /// Validates and registers every namespace declared by one component.
    pub fn register_component(
        &self,
        component_id: &str,
        declarations: &[NamespaceDeclaration],
    ) -> Result<(), StateError> {
        let mut inner = self.lock()?;
        let mut registry = inner.namespaces.clone();
        registry.register(component_id, declarations)?;
        for project in inner.projects.values_mut() {
            if let Some(persistent) = project.persistent.as_mut() {
                persistent.synchronize(&registry.records)?;
            }
        }
        inner.namespaces = registry;
        Ok(())
    }

    /// Starts an isolated overlay for one project/document revision.
    pub fn begin_parse(
        &self,
        project_uri: &str,
        document_id: &str,
        document_revision: u64,
    ) -> Result<ParseTransaction, StateError> {
        validate_identifier("document ID", document_id)?;
        let project_uri = canonical_project_uri(project_uri)?;
        let mut inner = self.lock()?;
        let latest = inner
            .latest_document_revisions
            .entry((project_uri.clone(), document_id.to_owned()))
            .or_insert(document_revision);
        *latest = (*latest).max(document_revision);
        inner.projects.entry(project_uri.clone()).or_default();
        let transaction_id = inner.next_transaction_id;
        inner.next_transaction_id = inner.next_transaction_id.saturating_add(1);
        Ok(ParseTransaction {
            inner: Arc::new(Mutex::new(ParseTransactionInner {
                transaction_id,
                store: self.clone(),
                project_uri,
                document_id: document_id.to_owned(),
                document_revision,
                writes: BTreeMap::new(),
                read_write_set: StateReadWriteSet::default(),
                base_revisions: BTreeMap::new(),
                current_revisions: BTreeMap::new(),
                closed: false,
            })),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, StateStoreInner>, StateError> {
        self.inner.lock().map_err(|_| StateError::Internal {
            message: "StateStore mutex was poisoned".to_owned(),
        })
    }
}

struct ParseTransactionInner {
    transaction_id: u64,
    store: StateStore,
    project_uri: String,
    document_id: String,
    document_revision: u64,
    writes: BTreeMap<Address, Option<StateValue>>,
    read_write_set: StateReadWriteSet,
    base_revisions: BTreeMap<RevisionKey, u64>,
    current_revisions: BTreeMap<RevisionKey, u64>,
    closed: bool,
}

impl ParseTransactionInner {
    fn ensure_open(&self) -> Result<(), StateError> {
        if self.closed {
            Err(StateError::TransactionClosed)
        } else {
            Ok(())
        }
    }

    fn revision(
        &mut self,
        store: &mut StateStoreInner,
        key: &RevisionKey,
    ) -> Result<u64, StateError> {
        if let Some(revision) = self.current_revisions.get(key) {
            return Ok(*revision);
        }
        let revision = store.committed_revision(&self.project_uri, key)?;
        self.base_revisions.insert(key.clone(), revision);
        self.current_revisions.insert(key.clone(), revision);
        Ok(revision)
    }
}

#[derive(Clone)]
/// Revision-bound parse overlay that owns invocation transactions and commit.
pub struct ParseTransaction {
    inner: Arc<Mutex<ParseTransactionInner>>,
}

impl ParseTransaction {
    /// Starts a speculative hook overlay owned by `component_id`.
    pub fn begin_invocation(
        &self,
        component_id: impl Into<String>,
    ) -> Result<InvocationTransaction, StateError> {
        let component_id = component_id.into();
        validate_identifier("component ID", &component_id)?;
        self.lock()?.ensure_open()?;
        Ok(InvocationTransaction {
            parse: Arc::clone(&self.inner),
            component_id,
            writes: BTreeMap::new(),
            reads: BTreeSet::new(),
            write_set: BTreeSet::new(),
            observed_revisions: BTreeMap::new(),
        })
    }

    /// Captures a rollback point in the current parse overlay.
    pub fn savepoint(&self) -> Result<StateSavepoint, StateError> {
        let inner = self.lock()?;
        inner.ensure_open()?;
        Ok(StateSavepoint {
            transaction_id: inner.transaction_id,
            writes: inner.writes.clone(),
            read_write_set: inner.read_write_set.clone(),
            base_revisions: inner.base_revisions.clone(),
            current_revisions: inner.current_revisions.clone(),
        })
    }

    /// Discards all parse-overlay changes after a compatible savepoint.
    pub fn rollback_to(&self, savepoint: &StateSavepoint) -> Result<(), StateError> {
        let mut inner = self.lock()?;
        inner.ensure_open()?;
        if inner.transaction_id != savepoint.transaction_id {
            return Err(StateError::ForeignSavepoint);
        }
        inner.writes.clone_from(&savepoint.writes);
        inner.read_write_set.clone_from(&savepoint.read_write_set);
        inner.base_revisions.clone_from(&savepoint.base_revisions);
        inner
            .current_revisions
            .clone_from(&savepoint.current_revisions);
        Ok(())
    }

    /// Returns dependencies accumulated by every accepted invocation so far.
    pub fn read_write_set(&self) -> Result<StateReadWriteSet, StateError> {
        Ok(self.lock()?.read_write_set.clone())
    }

    /// Commits a current parse revision and durable scopes atomically.
    pub fn commit(&self) -> Result<CommitSummary, StateError> {
        let mut parse = self.lock()?;
        parse.ensure_open()?;
        let store_handle = parse.store.clone();
        let mut store = store_handle.lock()?;
        let latest = store
            .latest_document_revisions
            .get(&(parse.project_uri.clone(), parse.document_id.clone()))
            .copied()
            .unwrap_or(parse.document_revision);
        if latest != parse.document_revision {
            return Err(StateError::StaleDocumentRevision {
                document_id: parse.document_id.clone(),
                actual: parse.document_revision,
                latest,
            });
        }
        for (key, expected) in &parse.base_revisions {
            if matches!(key.scope, StateScope::Invocation | StateScope::Parse) {
                continue;
            }
            if store.committed_revision(&parse.project_uri, key)? != *expected {
                return Err(StateError::TransactionConflict {
                    namespace: key.namespace.name().to_owned(),
                });
            }
        }
        let committed =
            store.commit_writes(&parse.project_uri, &parse.document_id, &parse.writes)?;
        parse.closed = true;
        Ok(CommitSummary {
            writes: committed,
            read_write_set: parse.read_write_set.clone(),
        })
    }

    /// Closes the parse transaction without publishing any writes.
    pub fn cancel(&self) -> Result<(), StateError> {
        let mut inner = self.lock()?;
        inner.ensure_open()?;
        inner.closed = true;
        inner.writes.clear();
        Ok(())
    }

    /// Returns the document identity bound to this transaction.
    pub fn document_id(&self) -> Result<String, StateError> {
        Ok(self.lock()?.document_id.clone())
    }

    /// Returns the document revision bound to this transaction.
    pub fn document_revision(&self) -> Result<u64, StateError> {
        Ok(self.lock()?.document_revision)
    }

    fn lock(&self) -> Result<MutexGuard<'_, ParseTransactionInner>, StateError> {
        self.inner.lock().map_err(|_| StateError::Internal {
            message: "parse transaction mutex was poisoned".to_owned(),
        })
    }
}

#[derive(Debug, Clone)]
/// Opaque snapshot of a parse overlay used for candidate rollback.
pub struct StateSavepoint {
    transaction_id: u64,
    writes: BTreeMap<Address, Option<StateValue>>,
    read_write_set: StateReadWriteSet,
    base_revisions: BTreeMap<RevisionKey, u64>,
    current_revisions: BTreeMap<RevisionKey, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Committed write count and complete parse read/write dependency set.
pub struct CommitSummary {
    pub writes: usize,
    pub read_write_set: StateReadWriteSet,
}

/// Speculative hook overlay merged only when its candidate is accepted.
pub struct InvocationTransaction {
    parse: Arc<Mutex<ParseTransactionInner>>,
    component_id: String,
    writes: BTreeMap<Address, Option<StateValue>>,
    reads: BTreeSet<StateRecordKey>,
    write_set: BTreeSet<StateRecordKey>,
    observed_revisions: BTreeMap<RevisionKey, u64>,
}

impl InvocationTransaction {
    /// Reads one value through the invocation overlay and records the dependency.
    pub fn get(
        &mut self,
        scope: StateScope,
        visibility: NamespaceVisibility,
        namespace: &str,
        key: &str,
    ) -> Result<Option<StateValue>, StateError> {
        validate_key(key, self.quotas()?.max_key_bytes)?;
        let (namespace_key, _, values, revision) =
            self.materialize(scope, visibility, namespace, false)?;
        let address = Address {
            scope,
            namespace: namespace_key.clone(),
            key: key.to_owned(),
        };
        self.reads.insert(address.public());
        self.observed_revisions.insert(
            RevisionKey {
                scope,
                namespace: namespace_key,
            },
            revision,
        );
        Ok(values.get(key).cloned())
    }

    /// Returns ordered entries whose key begins with `prefix`, bounded by both quotas.
    pub fn scan_prefix(
        &mut self,
        scope: StateScope,
        visibility: NamespaceVisibility,
        namespace: &str,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<StateEntry>, StateError> {
        let quotas = self.quotas()?;
        validate_key(prefix, quotas.max_key_bytes)?;
        if limit > quotas.max_scan_entries {
            return Err(StateError::QuotaExceeded {
                message: format!(
                    "scan limit {limit} exceeds maximum {}",
                    quotas.max_scan_entries
                ),
            });
        }
        let (namespace_key, _, values, revision) =
            self.materialize(scope, visibility, namespace, false)?;
        self.observed_revisions.insert(
            RevisionKey {
                scope,
                namespace: namespace_key.clone(),
            },
            revision,
        );
        let mut entries = Vec::new();
        for (key, value) in values
            .range(prefix.to_owned()..)
            .take_while(|(key, _)| key.starts_with(prefix))
            .take(limit)
        {
            let address = Address {
                scope,
                namespace: namespace_key.clone(),
                key: key.clone(),
            };
            self.reads.insert(address.public());
            entries.push(StateEntry {
                key: key.clone(),
                value: value.clone(),
            });
        }
        Ok(entries)
    }

    /// Stages one schema-compatible value in this invocation.
    pub fn put(
        &mut self,
        scope: StateScope,
        visibility: NamespaceVisibility,
        namespace: &str,
        key: &str,
        value: StateValue,
    ) -> Result<(), StateError> {
        self.write(scope, visibility, namespace, key, Some(value))
            .map(|_| ())
    }

    /// Stages deletion of one key and reports whether it was visible.
    pub fn delete(
        &mut self,
        scope: StateScope,
        visibility: NamespaceVisibility,
        namespace: &str,
        key: &str,
    ) -> Result<bool, StateError> {
        self.write(scope, visibility, namespace, key, None)
    }

    /// Conditionally replaces a value using schema ID, encoding, and bytes for equality.
    pub fn compare_and_swap(
        &mut self,
        scope: StateScope,
        visibility: NamespaceVisibility,
        namespace: &str,
        key: &str,
        expected: Option<&StateValue>,
        replacement: Option<StateValue>,
    ) -> Result<bool, StateError> {
        let current = self.get(scope, visibility, namespace, key)?;
        if current.as_ref() != expected {
            return Ok(false);
        }
        self.write(scope, visibility, namespace, key, replacement)?;
        Ok(true)
    }

    /// Returns dependencies accumulated by this speculative invocation.
    pub fn read_write_set(&self) -> StateReadWriteSet {
        let namespace_revisions = self
            .observed_revisions
            .iter()
            .map(|(key, revision)| (key.public(), *revision))
            .collect();
        StateReadWriteSet {
            reads: self.reads.clone(),
            writes: self.write_set.clone(),
            namespace_revisions,
        }
    }

    /// Merges invocation writes into the parent parse overlay.
    pub fn commit(self) -> Result<(), StateError> {
        let mut parse = self.parse.lock().map_err(|_| StateError::Internal {
            message: "parse transaction mutex was poisoned".to_owned(),
        })?;
        parse.ensure_open()?;
        for (key, observed) in &self.observed_revisions {
            if parse
                .current_revisions
                .get(key)
                .is_some_and(|current| current != observed)
            {
                return Err(StateError::TransactionConflict {
                    namespace: key.namespace.name().to_owned(),
                });
            }
        }

        let touched = self
            .writes
            .keys()
            .filter(|address| address.scope != StateScope::Invocation)
            .map(|address| RevisionKey {
                scope: address.scope,
                namespace: address.namespace.clone(),
            })
            .collect::<BTreeSet<_>>();
        {
            let store_handle = parse.store.clone();
            let mut store = store_handle.lock()?;
            let quota = store.quotas.max_namespace_bytes;
            for key in &touched {
                let mut values = if key.scope == StateScope::Parse {
                    BTreeMap::new()
                } else {
                    store.committed_namespace(
                        &parse.project_uri,
                        &parse.document_id,
                        key.scope,
                        &key.namespace,
                    )?
                };
                for (address, value) in &parse.writes {
                    if address.scope == key.scope && address.namespace == key.namespace {
                        apply_overlay(&mut values, &address.key, value);
                    }
                }
                for (address, value) in &self.writes {
                    if address.scope == key.scope && address.namespace == key.namespace {
                        apply_overlay(&mut values, &address.key, value);
                    }
                }
                let size = namespace_size(&values);
                if size > quota {
                    return Err(StateError::QuotaExceeded {
                        message: format!(
                            "namespace {} uses {size} bytes, maximum is {quota}",
                            key.namespace.name()
                        ),
                    });
                }
            }
        }

        for (address, value) in self.writes {
            if address.scope == StateScope::Invocation {
                continue;
            }
            parse.writes.insert(address, value);
        }
        parse.read_write_set.reads.extend(self.reads);
        parse.read_write_set.writes.extend(self.write_set);
        for (key, revision) in self.observed_revisions {
            parse.base_revisions.entry(key.clone()).or_insert(revision);
            parse
                .read_write_set
                .namespace_revisions
                .entry(key.public())
                .or_insert(revision);
            parse.current_revisions.entry(key).or_insert(revision);
        }
        for key in touched {
            let revision = parse.current_revisions.entry(key).or_insert(0);
            *revision = revision.saturating_add(1);
        }
        Ok(())
    }

    /// Explicitly discards every invocation write.
    pub fn rollback(self) {}

    fn write(
        &mut self,
        scope: StateScope,
        visibility: NamespaceVisibility,
        namespace: &str,
        key: &str,
        replacement: Option<StateValue>,
    ) -> Result<bool, StateError> {
        let quotas = self.quotas()?;
        validate_key(key, quotas.max_key_bytes)?;
        if let Some(value) = replacement.as_ref()
            && value.stored_size() > quotas.max_value_bytes
        {
            return Err(StateError::QuotaExceeded {
                message: format!(
                    "value uses {} bytes, maximum is {}",
                    value.stored_size(),
                    quotas.max_value_bytes
                ),
            });
        }
        let (namespace_key, record, mut values, revision) =
            self.materialize(scope, visibility, namespace, true)?;
        if let Some(value) = replacement.as_ref()
            && value.schema_id != record.declaration.schema_id
        {
            return Err(StateError::SchemaMismatch {
                namespace: namespace.to_owned(),
                expected: record.declaration.schema_id,
                actual: value.schema_id.clone(),
            });
        }
        let existed = values.contains_key(key);
        apply_overlay(&mut values, key, &replacement);
        let size = namespace_size(&values);
        if size > quotas.max_namespace_bytes {
            return Err(StateError::QuotaExceeded {
                message: format!(
                    "namespace {namespace} uses {size} bytes, maximum is {}",
                    quotas.max_namespace_bytes
                ),
            });
        }
        let address = Address {
            scope,
            namespace: namespace_key.clone(),
            key: key.to_owned(),
        };
        self.writes.insert(address.clone(), replacement);
        self.write_set.insert(address.public());
        self.observed_revisions.insert(
            RevisionKey {
                scope,
                namespace: namespace_key,
            },
            revision,
        );
        Ok(existed)
    }

    fn materialize(
        &self,
        scope: StateScope,
        visibility: NamespaceVisibility,
        namespace: &str,
        write: bool,
    ) -> Result<
        (
            NamespaceKey,
            NamespaceRecord,
            BTreeMap<String, StateValue>,
            u64,
        ),
        StateError,
    > {
        validate_identifier("namespace", namespace)?;
        let mut parse = self.parse.lock().map_err(|_| StateError::Internal {
            message: "parse transaction mutex was poisoned".to_owned(),
        })?;
        parse.ensure_open()?;
        let store_handle = parse.store.clone();
        let mut store = store_handle.lock()?;
        let (namespace_key, record) =
            store
                .namespaces
                .resolve(&self.component_id, visibility, namespace, write)?;
        let revision_key = RevisionKey {
            scope,
            namespace: namespace_key.clone(),
        };
        let revision = parse.revision(&mut store, &revision_key)?;
        let mut values = if matches!(scope, StateScope::Invocation | StateScope::Parse) {
            BTreeMap::new()
        } else {
            store.committed_namespace(
                &parse.project_uri,
                &parse.document_id,
                scope,
                &namespace_key,
            )?
        };
        for (address, value) in &parse.writes {
            if address.scope == scope && address.namespace == namespace_key {
                apply_overlay(&mut values, &address.key, value);
            }
        }
        for (address, value) in &self.writes {
            if address.scope == scope && address.namespace == namespace_key {
                apply_overlay(&mut values, &address.key, value);
            }
        }
        Ok((namespace_key, record, values, revision))
    }

    fn quotas(&self) -> Result<StateQuotas, StateError> {
        let parse = self.parse.lock().map_err(|_| StateError::Internal {
            message: "parse transaction mutex was poisoned".to_owned(),
        })?;
        parse.ensure_open()?;
        let quotas = parse.store.lock()?.quotas.clone();
        Ok(quotas)
    }
}

impl StateStoreInner {
    fn ensure_persistent(&mut self, project_uri: &str) -> Result<(), StateError> {
        let records = self.namespaces.records.clone();
        let path = self
            .data_directory
            .join("projects")
            .join(format!("{}.redb", project_uri_hash(project_uri)));
        let project = self.projects.entry(project_uri.to_owned()).or_default();
        if project.persistent.is_none() {
            let persistent = PersistentProject::open(&path, &records)?;
            for (namespace, revision) in &persistent.revisions {
                project.revisions.insert(
                    RevisionKey {
                        scope: StateScope::PersistentProject,
                        namespace: namespace.clone(),
                    },
                    *revision,
                );
            }
            project.persistent = Some(persistent);
        }
        Ok(())
    }

    fn committed_namespace(
        &mut self,
        project_uri: &str,
        document_id: &str,
        scope: StateScope,
        namespace: &NamespaceKey,
    ) -> Result<BTreeMap<String, StateValue>, StateError> {
        if scope == StateScope::PersistentProject {
            self.ensure_persistent(project_uri)?;
        }
        let project = self.projects.entry(project_uri.to_owned()).or_default();
        Ok(match scope {
            StateScope::Invocation | StateScope::Parse => BTreeMap::new(),
            StateScope::Document => project
                .documents
                .get(document_id)
                .and_then(|namespaces| namespaces.get(namespace))
                .cloned()
                .unwrap_or_default(),
            StateScope::Project => project.project.get(namespace).cloned().unwrap_or_default(),
            StateScope::PersistentProject => project
                .persistent
                .as_ref()
                .and_then(|persistent| persistent.values.get(namespace))
                .cloned()
                .unwrap_or_default(),
        })
    }

    fn committed_revision(
        &mut self,
        project_uri: &str,
        key: &RevisionKey,
    ) -> Result<u64, StateError> {
        if key.scope == StateScope::PersistentProject {
            self.ensure_persistent(project_uri)?;
        }
        Ok(self
            .projects
            .entry(project_uri.to_owned())
            .or_default()
            .revisions
            .get(key)
            .copied()
            .unwrap_or(0))
    }

    fn commit_writes(
        &mut self,
        project_uri: &str,
        document_id: &str,
        writes: &BTreeMap<Address, Option<StateValue>>,
    ) -> Result<usize, StateError> {
        let persistent_writes = writes
            .iter()
            .filter(|(address, _)| address.scope == StateScope::PersistentProject)
            .map(|(address, value)| (address.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>();
        if !persistent_writes.is_empty() {
            self.ensure_persistent(project_uri)?;
            let records = self.namespaces.records.clone();
            self.projects
                .get_mut(project_uri)
                .and_then(|project| project.persistent.as_mut())
                .expect("persistent project was initialized")
                .commit(&persistent_writes, &records)?;
        }
        let project = self.projects.entry(project_uri.to_owned()).or_default();
        let mut touched = BTreeSet::new();
        let mut committed = 0usize;
        for (address, value) in writes {
            match address.scope {
                StateScope::Invocation | StateScope::Parse => continue,
                StateScope::Document => {
                    let namespace = project
                        .documents
                        .entry(document_id.to_owned())
                        .or_default()
                        .entry(address.namespace.clone())
                        .or_default();
                    apply_overlay(namespace, &address.key, value);
                }
                StateScope::Project => {
                    let namespace = project
                        .project
                        .entry(address.namespace.clone())
                        .or_default();
                    apply_overlay(namespace, &address.key, value);
                }
                StateScope::PersistentProject => {}
            }
            touched.insert(RevisionKey {
                scope: address.scope,
                namespace: address.namespace.clone(),
            });
            committed = committed.saturating_add(1);
        }
        for key in touched {
            let revision = if key.scope == StateScope::PersistentProject {
                project
                    .persistent
                    .as_ref()
                    .and_then(|persistent| persistent.revisions.get(&key.namespace))
                    .copied()
                    .unwrap_or(0)
            } else {
                project
                    .revisions
                    .get(&key)
                    .copied()
                    .unwrap_or(0)
                    .saturating_add(1)
            };
            project.revisions.insert(key, revision);
        }
        Ok(committed)
    }
}

fn canonical_project_uri(project_uri: &str) -> Result<String, StateError> {
    let mut uri = Url::parse(project_uri).map_err(|error| StateError::InvalidInput {
        message: format!("invalid project URI {project_uri:?}: {error}"),
    })?;
    uri.set_fragment(None);
    uri.set_query(None);
    Ok(uri.to_string())
}

fn default_data_directory() -> Result<PathBuf, StateError> {
    ProjectDirs::from("dev", "nlaocs", "Skript-LSP")
        .map(|directories| directories.data_local_dir().join("state"))
        .ok_or_else(|| StateError::Persistence {
            message: "the operating system did not provide an LSP data directory".to_owned(),
        })
}

fn project_uri_hash(project_uri: &str) -> String {
    format!("{:x}", Sha256::digest(project_uri.as_bytes()))
}

fn validate_identifier(kind: &str, value: &str) -> Result<(), StateError> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        Err(StateError::InvalidInput {
            message: format!("{kind} must not be blank or contain control characters"),
        })
    } else if value.len() > 1024 {
        Err(StateError::InvalidInput {
            message: format!("{kind} exceeds 1024 UTF-8 bytes"),
        })
    } else {
        Ok(())
    }
}

fn validate_key(key: &str, maximum: usize) -> Result<(), StateError> {
    if key.len() > maximum {
        Err(StateError::QuotaExceeded {
            message: format!("key uses {} bytes, maximum is {maximum}", key.len()),
        })
    } else {
        Ok(())
    }
}

fn namespace_size(values: &BTreeMap<String, StateValue>) -> usize {
    values.iter().fold(0usize, |size, (key, value)| {
        size.saturating_add(key.len())
            .saturating_add(value.stored_size())
    })
}

fn apply_overlay(values: &mut BTreeMap<String, StateValue>, key: &str, value: &Option<StateValue>) {
    match value {
        Some(value) => {
            values.insert(key.to_owned(), value.clone());
        }
        None => {
            values.remove(key);
        }
    }
}

pub(super) fn persistence_error(error: impl std::fmt::Display) -> StateError {
    StateError::Persistence {
        message: error.to_string(),
    }
}

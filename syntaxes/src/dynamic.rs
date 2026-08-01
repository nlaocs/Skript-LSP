//! Transactional overlay for syntax registered or overridden by WASM components.
//!
//! Mutable document revisions are isolated from the immutable static catalog.
//! Freezing validates references and produces deterministic candidate order.
#![allow(missing_docs)] // Public fields are described by their owning domain type.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, MutexGuard},
};

use syntax_pattern_parser::syntax::{self, ParseResult};

use crate::{Catalog, DefinitionId, RegistrationId, SyntaxKind};

const MAX_ITEMS_PER_COMPONENT: usize = 256;
const MAX_PATTERNS_PER_SYNTAX: usize = 64;
const MAX_PATTERN_BYTES_PER_SYNTAX: usize = 64 * 1024;
const MAX_METADATA_ENTRIES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Component-qualified identity of a dynamically registered syntax.
pub struct DynamicSyntaxId {
    pub component_id: String,
    pub local_id: String,
}

impl DynamicSyntaxId {
    /// Creates a component-qualified ID from manifest and local names.
    pub fn new(component_id: impl Into<String>, local_id: impl Into<String>) -> Self {
        Self {
            component_id: component_id.into(),
            local_id: local_id.into(),
        }
    }

    /// Formats the stable `dynamic:<component>/<local>` identity.
    pub fn qualified(&self) -> String {
        format!("dynamic:{}/{}", self.component_id, self.local_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Multiplicity metadata accepted from a dynamic syntax component.
pub enum DynamicMultiplicity {
    Single,
    Multiple,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Static or dynamic target used by ordering constraints.
pub enum SyntaxReference {
    Dynamic(DynamicSyntaxId),
    Definition(DefinitionId),
    Registration(RegistrationId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Source and parsed AST of one dynamic registration pattern.
pub struct DynamicPattern {
    pub source: String,
    pub parsed: ParseResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Unqualified syntax declaration supplied by a component.
pub struct DynamicSyntaxInput {
    pub local_id: String,
    pub kind: SyntaxKind,
    pub patterns: Vec<String>,
    pub priority: i32,
    pub before: Vec<SyntaxReference>,
    pub after: Vec<SyntaxReference>,
    pub return_type: Option<String>,
    pub return_multiplicity: Option<DynamicMultiplicity>,
    pub handler: String,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Validated, component-qualified dynamic syntax definition.
pub struct DynamicSyntaxDefinition {
    pub id: DynamicSyntaxId,
    pub kind: SyntaxKind,
    pub patterns: Vec<DynamicPattern>,
    pub priority: i32,
    pub before: Vec<SyntaxReference>,
    pub after: Vec<SyntaxReference>,
    pub return_type: Option<String>,
    pub return_multiplicity: Option<DynamicMultiplicity>,
    pub handler: String,
    pub metadata: BTreeMap<String, String>,
    pub component_load_order: usize,
    pub declaration_order: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Static definition or exact registration selected for override.
pub enum SyntaxOverrideTarget {
    Definition(DefinitionId),
    Registration(RegistrationId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Override declaration supplied by a component.
pub struct DynamicSyntaxOverrideInput {
    pub local_id: String,
    pub target: SyntaxOverrideTarget,
    pub priority: i32,
    pub handler: String,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Validated, component-owned override attached to a static candidate.
pub struct DynamicSyntaxOverride {
    pub id: DynamicSyntaxId,
    pub target: SyntaxOverrideTarget,
    pub priority: i32,
    pub handler: String,
    pub metadata: BTreeMap<String, String>,
    pub component_load_order: usize,
    pub declaration_order: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Opaque component handler and metadata associated with a candidate.
pub struct DynamicHandler {
    pub registration_id: DynamicSyntaxId,
    pub handler: String,
    pub priority: i32,
    pub component_load_order: usize,
    pub declaration_order: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Whether a ranked candidate came from the static catalog or a component.
pub enum SyntaxCandidateSource {
    Static(usize),
    Dynamic(DynamicSyntaxId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One parser candidate after dynamic ordering and overrides are resolved.
pub struct RankedSyntaxCandidate {
    pub source: SyntaxCandidateSource,
    pub kind: SyntaxKind,
    pub overrides: Vec<DynamicHandler>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Immutable candidate view frozen for one document revision.
pub struct DynamicSyntaxSnapshot {
    pub document_id: String,
    pub document_revision: u64,
    pub registry_revision: u64,
    pub definitions: BTreeMap<DynamicSyntaxId, DynamicSyntaxDefinition>,
    pub overrides: BTreeMap<DynamicSyntaxId, DynamicSyntaxOverride>,
    pub candidates: Vec<RankedSyntaxCandidate>,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
/// Rejected dynamic update, ordering graph, revision, or quota operation.
pub enum DynamicRegistryError {
    #[error("invalid dynamic syntax input: {message}")]
    InvalidInput { message: String },
    #[error("dynamic syntax ID {id} is already registered")]
    DuplicateId { id: String },
    #[error("dynamic syntax ID {id} is not registered")]
    UnknownId { id: String },
    #[error("dynamic syntax pattern {pattern:?} is invalid: {message}")]
    InvalidPattern { pattern: String, message: String },
    #[error("document {document_id}@{document_revision} has no dynamic syntax prepass")]
    UnknownDocument {
        document_id: String,
        document_revision: u64,
    },
    #[error("document {document_id}@{actual} is stale; latest revision is {latest}")]
    StaleDocumentRevision {
        document_id: String,
        actual: u64,
        latest: u64,
    },
    #[error("dynamic syntax registry for {document_id}@{document_revision} is frozen")]
    Frozen {
        document_id: String,
        document_revision: u64,
    },
    #[error("dynamic syntax reference {reference} does not resolve")]
    UnknownReference { reference: String },
    #[error("dynamic syntax priority reference crosses syntax kinds: {reference}")]
    CrossKindReference { reference: String },
    #[error("dynamic syntax priority contains a cycle: {ids:?}")]
    PriorityCycle { ids: Vec<String> },
    #[error("dynamic syntax registry internal error: {message}")]
    Internal { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StoredItem {
    Definition(DynamicSyntaxDefinition),
    Override(DynamicSyntaxOverride),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ComponentLayer {
    load_order: usize,
    items: BTreeMap<String, StoredItem>,
}

#[derive(Debug, Clone)]
struct DocumentLayer {
    revision: u64,
    components: BTreeMap<String, ComponentLayer>,
    frozen: Option<DynamicSyntaxSnapshot>,
}

struct RegistryInner {
    initial: BTreeMap<String, ComponentLayer>,
    documents: BTreeMap<String, DocumentLayer>,
    latest_document_revisions: BTreeMap<String, u64>,
    next_declaration_order: u64,
    revision: u64,
}

#[derive(Clone)]
/// Thread-safe dynamic overlay rooted in an immutable static catalog.
pub struct DynamicSyntaxRegistry {
    catalog: Arc<Catalog>,
    inner: Arc<Mutex<RegistryInner>>,
}

impl DynamicSyntaxRegistry {
    /// Creates an empty overlay over an immutable static catalog.
    pub fn new(catalog: Arc<Catalog>) -> Self {
        Self {
            catalog,
            inner: Arc::new(Mutex::new(RegistryInner {
                initial: BTreeMap::new(),
                documents: BTreeMap::new(),
                latest_document_revisions: BTreeMap::new(),
                next_declaration_order: 0,
                revision: 0,
            })),
        }
    }

    /// Starts a transactional update to the component initialization baseline.
    pub fn begin_initial_update(
        &self,
        component_id: impl Into<String>,
        component_load_order: usize,
    ) -> Result<DynamicSyntaxUpdate, DynamicRegistryError> {
        self.begin_update(
            component_id.into(),
            component_load_order,
            UpdateTarget::Initial,
        )
    }

    /// Clones the baseline into a mutable document revision.
    pub fn begin_document(
        &self,
        document_id: &str,
        document_revision: u64,
    ) -> Result<(), DynamicRegistryError> {
        validate_identifier("document ID", document_id)?;
        let mut inner = self.lock()?;
        if let Some(latest) = inner.latest_document_revisions.get(document_id)
            && document_revision < *latest
        {
            return Err(DynamicRegistryError::StaleDocumentRevision {
                document_id: document_id.to_owned(),
                actual: document_revision,
                latest: *latest,
            });
        }
        inner
            .latest_document_revisions
            .insert(document_id.to_owned(), document_revision);
        let components = inner.initial.clone();
        inner
            .documents
            .entry(document_id.to_owned())
            .and_modify(|document| {
                if document_revision > document.revision {
                    *document = DocumentLayer {
                        revision: document_revision,
                        components: components.clone(),
                        frozen: None,
                    };
                }
            })
            .or_insert(DocumentLayer {
                revision: document_revision,
                components,
                frozen: None,
            });
        Ok(())
    }

    /// Starts a transactional update against one document revision.
    pub fn begin_document_update(
        &self,
        component_id: impl Into<String>,
        component_load_order: usize,
        document_id: &str,
        document_revision: u64,
    ) -> Result<DynamicSyntaxUpdate, DynamicRegistryError> {
        self.ensure_document_mutable(document_id, document_revision)?;
        self.begin_update(
            component_id.into(),
            component_load_order,
            UpdateTarget::Document {
                document_id: document_id.to_owned(),
                document_revision,
            },
        )
    }

    /// Captures an opaque document rollback point for candidate speculation.
    pub fn savepoint(
        &self,
        document_id: &str,
        document_revision: u64,
    ) -> Result<DynamicSyntaxSavepoint, DynamicRegistryError> {
        let inner = self.lock()?;
        let document = document(&inner, document_id, document_revision)?;
        if document.frozen.is_some() {
            return Err(DynamicRegistryError::Frozen {
                document_id: document_id.to_owned(),
                document_revision,
            });
        }
        Ok(DynamicSyntaxSavepoint {
            document_id: document_id.to_owned(),
            document_revision,
            components: document.components.clone(),
            registry_revision: inner.revision,
        })
    }

    /// Restores a document registry to a compatible savepoint.
    pub fn rollback_to(
        &self,
        savepoint: &DynamicSyntaxSavepoint,
    ) -> Result<(), DynamicRegistryError> {
        let mut inner = self.lock()?;
        let document = document_mut(
            &mut inner,
            &savepoint.document_id,
            savepoint.document_revision,
        )?;
        if document.frozen.is_some() {
            return Err(DynamicRegistryError::Frozen {
                document_id: savepoint.document_id.clone(),
                document_revision: savepoint.document_revision,
            });
        }
        document.components.clone_from(&savepoint.components);
        inner.revision = inner
            .revision
            .max(savepoint.registry_revision)
            .saturating_add(1);
        Ok(())
    }

    /// Validates and deterministically ranks a revision into an immutable snapshot.
    pub fn freeze(
        &self,
        document_id: &str,
        document_revision: u64,
    ) -> Result<DynamicSyntaxSnapshot, DynamicRegistryError> {
        let mut inner = self.lock()?;
        if let Some(snapshot) = document(&inner, document_id, document_revision)?
            .frozen
            .clone()
        {
            return Ok(snapshot);
        }
        let components = document(&inner, document_id, document_revision)?
            .components
            .clone();
        let snapshot = build_snapshot(
            &self.catalog,
            document_id,
            document_revision,
            inner.revision,
            &components,
        )?;
        document_mut(&mut inner, document_id, document_revision)?.frozen = Some(snapshot.clone());
        Ok(snapshot)
    }

    /// Removes all baseline definitions and overrides owned by a component.
    pub fn remove_component(&self, component_id: &str) -> Result<(), DynamicRegistryError> {
        let mut inner = self.lock()?;
        let mut changed = inner.initial.remove(component_id).is_some();
        for document in inner.documents.values_mut() {
            if document.frozen.is_none() {
                changed |= document.components.remove(component_id).is_some();
            }
        }
        if changed {
            inner.revision = inner.revision.saturating_add(1);
        }
        Ok(())
    }

    fn begin_update(
        &self,
        component_id: String,
        component_load_order: usize,
        target: UpdateTarget,
    ) -> Result<DynamicSyntaxUpdate, DynamicRegistryError> {
        validate_identifier("component ID", &component_id)?;
        let known = {
            let inner = self.lock()?;
            match &target {
                UpdateTarget::Initial => inner.initial.get(&component_id),
                UpdateTarget::Document {
                    document_id,
                    document_revision,
                } => document(&inner, document_id, *document_revision)?
                    .components
                    .get(&component_id),
            }
            .map(|layer| layer.items.keys().cloned().collect())
            .unwrap_or_default()
        };
        Ok(DynamicSyntaxUpdate {
            registry: self.clone(),
            component_id,
            component_load_order,
            target,
            known,
            operations: Vec::new(),
        })
    }

    fn ensure_document_mutable(
        &self,
        document_id: &str,
        document_revision: u64,
    ) -> Result<(), DynamicRegistryError> {
        let inner = self.lock()?;
        let document = document(&inner, document_id, document_revision)?;
        if document.frozen.is_some() {
            Err(DynamicRegistryError::Frozen {
                document_id: document_id.to_owned(),
                document_revision,
            })
        } else {
            Ok(())
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, RegistryInner>, DynamicRegistryError> {
        self.inner
            .lock()
            .map_err(|_| DynamicRegistryError::Internal {
                message: "dynamic syntax registry mutex was poisoned".to_owned(),
            })
    }
}

#[derive(Debug, Clone)]
enum UpdateTarget {
    Initial,
    Document {
        document_id: String,
        document_revision: u64,
    },
}

#[derive(Debug, Clone)]
enum StagedOperation {
    Register(DynamicSyntaxInput, Vec<DynamicPattern>),
    Override(DynamicSyntaxOverrideInput),
    Remove(String),
}

/// Transactional batch of registrations and overrides from one component.
pub struct DynamicSyntaxUpdate {
    registry: DynamicSyntaxRegistry,
    component_id: String,
    component_load_order: usize,
    target: UpdateTarget,
    known: BTreeSet<String>,
    operations: Vec<StagedOperation>,
}

impl DynamicSyntaxUpdate {
    /// Returns the component that owns this transaction.
    pub fn component_id(&self) -> &str {
        &self.component_id
    }

    /// Stages a syntax after validating identity, quotas, and patterns.
    pub fn register(&mut self, input: DynamicSyntaxInput) -> Result<(), DynamicRegistryError> {
        validate_local_id(&input.local_id)?;
        validate_handler(&input.handler)?;
        validate_metadata(&input.metadata)?;
        if input.patterns.is_empty() {
            return Err(DynamicRegistryError::InvalidInput {
                message: format!("dynamic syntax {} has no patterns", input.local_id),
            });
        }
        if input.patterns.len() > MAX_PATTERNS_PER_SYNTAX {
            return Err(DynamicRegistryError::InvalidInput {
                message: format!(
                    "dynamic syntax {} has {} patterns, maximum is {MAX_PATTERNS_PER_SYNTAX}",
                    input.local_id,
                    input.patterns.len()
                ),
            });
        }
        let pattern_bytes = input.patterns.iter().map(String::len).sum::<usize>();
        if pattern_bytes > MAX_PATTERN_BYTES_PER_SYNTAX {
            return Err(DynamicRegistryError::InvalidInput {
                message: format!(
                    "dynamic syntax {} patterns use {pattern_bytes} bytes, maximum is {MAX_PATTERN_BYTES_PER_SYNTAX}",
                    input.local_id
                ),
            });
        }
        if input
            .return_type
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(DynamicRegistryError::InvalidInput {
                message: "dynamic return type must not be blank".to_owned(),
            });
        }
        let mut patterns = Vec::with_capacity(input.patterns.len());
        for source in &input.patterns {
            if source.trim().is_empty() {
                return Err(DynamicRegistryError::InvalidPattern {
                    pattern: source.clone(),
                    message: "pattern must not be blank".to_owned(),
                });
            }
            let parsed =
                syntax::parse(source, self.registry.catalog.plural_rules()).map_err(|error| {
                    DynamicRegistryError::InvalidPattern {
                        pattern: source.clone(),
                        message: error.to_string(),
                    }
                })?;
            patterns.push(DynamicPattern {
                source: source.clone(),
                parsed,
            });
        }
        self.insert_id(&input.local_id)?;
        self.operations
            .push(StagedOperation::Register(input, patterns));
        Ok(())
    }

    /// Stages an override of a static definition or exact registration.
    pub fn register_override(
        &mut self,
        input: DynamicSyntaxOverrideInput,
    ) -> Result<(), DynamicRegistryError> {
        validate_local_id(&input.local_id)?;
        validate_handler(&input.handler)?;
        validate_metadata(&input.metadata)?;
        match &input.target {
            SyntaxOverrideTarget::Definition(value) => {
                validate_identifier("definition ID", value.as_str())?
            }
            SyntaxOverrideTarget::Registration(value) => {
                validate_identifier("registration ID", value.as_str())?
            }
        }
        self.insert_id(&input.local_id)?;
        self.operations.push(StagedOperation::Override(input));
        Ok(())
    }

    /// Stages removal of one local dynamic definition.
    pub fn remove(&mut self, local_id: &str) -> Result<bool, DynamicRegistryError> {
        validate_local_id(local_id)?;
        if self.known.remove(local_id) {
            self.operations
                .push(StagedOperation::Remove(local_id.to_owned()));
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Atomically publishes every staged change.
    pub fn commit(self) -> Result<(), DynamicRegistryError> {
        let mut inner = self.registry.lock()?;
        let current_items = match &self.target {
            UpdateTarget::Initial => inner
                .initial
                .get(&self.component_id)
                .map(|layer| layer.items.clone())
                .unwrap_or_default(),
            UpdateTarget::Document {
                document_id,
                document_revision,
            } => {
                let document = document(&inner, document_id, *document_revision)?;
                if document.frozen.is_some() {
                    return Err(DynamicRegistryError::Frozen {
                        document_id: document_id.clone(),
                        document_revision: *document_revision,
                    });
                }
                document
                    .components
                    .get(&self.component_id)
                    .map(|layer| layer.items.clone())
                    .unwrap_or_default()
            }
        };
        let mut items = current_items;
        for operation in self.operations {
            match operation {
                StagedOperation::Register(input, patterns) => {
                    let declaration_order = inner.next_declaration_order;
                    inner.next_declaration_order = inner.next_declaration_order.saturating_add(1);
                    let id = DynamicSyntaxId::new(&self.component_id, &input.local_id);
                    items.insert(
                        input.local_id,
                        StoredItem::Definition(DynamicSyntaxDefinition {
                            id,
                            kind: input.kind,
                            patterns,
                            priority: input.priority,
                            before: input.before,
                            after: input.after,
                            return_type: input.return_type,
                            return_multiplicity: input.return_multiplicity,
                            handler: input.handler,
                            metadata: input.metadata,
                            component_load_order: self.component_load_order,
                            declaration_order,
                        }),
                    );
                }
                StagedOperation::Override(input) => {
                    let declaration_order = inner.next_declaration_order;
                    inner.next_declaration_order = inner.next_declaration_order.saturating_add(1);
                    let id = DynamicSyntaxId::new(&self.component_id, &input.local_id);
                    items.insert(
                        input.local_id,
                        StoredItem::Override(DynamicSyntaxOverride {
                            id,
                            target: input.target,
                            priority: input.priority,
                            handler: input.handler,
                            metadata: input.metadata,
                            component_load_order: self.component_load_order,
                            declaration_order,
                        }),
                    );
                }
                StagedOperation::Remove(local_id) => {
                    items.remove(&local_id);
                }
            }
        }
        if items.len() > MAX_ITEMS_PER_COMPONENT {
            return Err(DynamicRegistryError::InvalidInput {
                message: format!(
                    "component {} registers {} dynamic syntax items, maximum is {MAX_ITEMS_PER_COMPONENT}",
                    self.component_id,
                    items.len()
                ),
            });
        }
        let layer = ComponentLayer {
            load_order: self.component_load_order,
            items,
        };
        match self.target {
            UpdateTarget::Initial => {
                inner.initial.insert(self.component_id, layer);
            }
            UpdateTarget::Document {
                document_id,
                document_revision,
            } => {
                document_mut(&mut inner, &document_id, document_revision)?
                    .components
                    .insert(self.component_id, layer);
            }
        }
        inner.revision = inner.revision.saturating_add(1);
        Ok(())
    }

    /// Explicitly discards the staged update.
    pub fn rollback(self) {}

    fn insert_id(&mut self, local_id: &str) -> Result<(), DynamicRegistryError> {
        if self.known.contains(local_id) {
            return Err(DynamicRegistryError::DuplicateId {
                id: DynamicSyntaxId::new(&self.component_id, local_id).qualified(),
            });
        }
        if self.known.len() >= MAX_ITEMS_PER_COMPONENT {
            return Err(DynamicRegistryError::InvalidInput {
                message: format!(
                    "component {} exceeds the maximum of {MAX_ITEMS_PER_COMPONENT} dynamic syntax items",
                    self.component_id
                ),
            });
        }
        self.known.insert(local_id.to_owned());
        Ok(())
    }
}

#[derive(Debug, Clone)]
/// Opaque rollback point for document-scoped dynamic registry changes.
pub struct DynamicSyntaxSavepoint {
    document_id: String,
    document_revision: u64,
    components: BTreeMap<String, ComponentLayer>,
    registry_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum CandidateKey {
    Static(usize),
    Dynamic(DynamicSyntaxId),
}

#[derive(Debug, Clone)]
struct CandidateNode {
    key: CandidateKey,
    kind: SyntaxKind,
    priority: i32,
    source_class: u8,
    component_load_order: usize,
    declaration_order: u64,
}

fn build_snapshot(
    catalog: &Catalog,
    document_id: &str,
    document_revision: u64,
    registry_revision: u64,
    components: &BTreeMap<String, ComponentLayer>,
) -> Result<DynamicSyntaxSnapshot, DynamicRegistryError> {
    let mut definitions = BTreeMap::new();
    let mut overrides = BTreeMap::new();
    for layer in components.values() {
        for item in layer.items.values() {
            match item {
                StoredItem::Definition(value) => {
                    definitions.insert(value.id.clone(), value.clone());
                }
                StoredItem::Override(value) => {
                    overrides.insert(value.id.clone(), value.clone());
                }
            }
        }
    }

    let mut nodes = BTreeMap::new();
    let mut registration_targets: BTreeMap<String, Vec<CandidateKey>> = BTreeMap::new();
    let mut definition_targets: BTreeMap<String, Vec<CandidateKey>> = BTreeMap::new();
    for (index, syntax) in catalog.syntaxes().iter().enumerate() {
        let key = CandidateKey::Static(index);
        registration_targets
            .entry(syntax.registration_id().as_str().to_owned())
            .or_default()
            .push(key.clone());
        definition_targets
            .entry(syntax.definition_id().as_str().to_owned())
            .or_default()
            .push(key.clone());
        nodes.insert(
            key.clone(),
            CandidateNode {
                key,
                kind: syntax.kind(),
                priority: 0,
                source_class: 0,
                component_load_order: 0,
                declaration_order: syntax.registration_order() as u64,
            },
        );
    }
    for definition in definitions.values() {
        let key = CandidateKey::Dynamic(definition.id.clone());
        nodes.insert(
            key.clone(),
            CandidateNode {
                key,
                kind: definition.kind,
                priority: definition.priority,
                source_class: 1,
                component_load_order: definition.component_load_order,
                declaration_order: definition.declaration_order,
            },
        );
    }

    let mut edges: BTreeMap<CandidateKey, BTreeSet<CandidateKey>> = nodes
        .keys()
        .cloned()
        .map(|key| (key, BTreeSet::new()))
        .collect();
    for definition in definitions.values() {
        let source = CandidateKey::Dynamic(definition.id.clone());
        for reference in &definition.before {
            for target in resolve_reference(
                reference,
                &definitions,
                &registration_targets,
                &definition_targets,
            )? {
                ensure_same_kind(&nodes, &source, &target, reference)?;
                edges.entry(source.clone()).or_default().insert(target);
            }
        }
        for reference in &definition.after {
            for target in resolve_reference(
                reference,
                &definitions,
                &registration_targets,
                &definition_targets,
            )? {
                ensure_same_kind(&nodes, &source, &target, reference)?;
                edges.entry(target).or_default().insert(source.clone());
            }
        }
    }

    let order = topological_order(&nodes, &edges)?;
    let mut handlers: BTreeMap<usize, Vec<DynamicHandler>> = BTreeMap::new();
    for value in overrides.values() {
        let targets = match &value.target {
            SyntaxOverrideTarget::Definition(id) => definition_targets.get(id.as_str()),
            SyntaxOverrideTarget::Registration(id) => registration_targets.get(id.as_str()),
        }
        .ok_or_else(|| DynamicRegistryError::UnknownReference {
            reference: override_target_name(&value.target),
        })?;
        for target in targets {
            let CandidateKey::Static(index) = target else {
                continue;
            };
            handlers.entry(*index).or_default().push(DynamicHandler {
                registration_id: value.id.clone(),
                handler: value.handler.clone(),
                priority: value.priority,
                component_load_order: value.component_load_order,
                declaration_order: value.declaration_order,
            });
        }
    }
    for values in handlers.values_mut() {
        values.sort_by_key(|value| {
            (
                value.priority,
                value.component_load_order,
                value.declaration_order,
                value.registration_id.clone(),
            )
        });
    }

    let candidates = order
        .into_iter()
        .map(|key| match key {
            CandidateKey::Static(index) => RankedSyntaxCandidate {
                kind: catalog
                    .syntax_at(index)
                    .expect("static candidate index came from the catalog")
                    .kind(),
                source: SyntaxCandidateSource::Static(index),
                overrides: handlers.remove(&index).unwrap_or_default(),
            },
            CandidateKey::Dynamic(id) => RankedSyntaxCandidate {
                kind: definitions
                    .get(&id)
                    .expect("dynamic candidate ID came from definitions")
                    .kind,
                source: SyntaxCandidateSource::Dynamic(id),
                overrides: Vec::new(),
            },
        })
        .collect();

    Ok(DynamicSyntaxSnapshot {
        document_id: document_id.to_owned(),
        document_revision,
        registry_revision,
        definitions,
        overrides,
        candidates,
    })
}

fn resolve_reference(
    reference: &SyntaxReference,
    definitions: &BTreeMap<DynamicSyntaxId, DynamicSyntaxDefinition>,
    registrations: &BTreeMap<String, Vec<CandidateKey>>,
    definition_ids: &BTreeMap<String, Vec<CandidateKey>>,
) -> Result<Vec<CandidateKey>, DynamicRegistryError> {
    let result = match reference {
        SyntaxReference::Dynamic(id) if definitions.contains_key(id) => {
            Some(vec![CandidateKey::Dynamic(id.clone())])
        }
        SyntaxReference::Dynamic(_) => None,
        SyntaxReference::Definition(id) => definition_ids.get(id.as_str()).cloned(),
        SyntaxReference::Registration(id) => registrations.get(id.as_str()).cloned(),
    };
    result.ok_or_else(|| DynamicRegistryError::UnknownReference {
        reference: reference_name(reference),
    })
}

fn ensure_same_kind(
    nodes: &BTreeMap<CandidateKey, CandidateNode>,
    source: &CandidateKey,
    target: &CandidateKey,
    reference: &SyntaxReference,
) -> Result<(), DynamicRegistryError> {
    if nodes.get(source).map(|node| node.kind) != nodes.get(target).map(|node| node.kind) {
        Err(DynamicRegistryError::CrossKindReference {
            reference: reference_name(reference),
        })
    } else {
        Ok(())
    }
}

fn topological_order(
    nodes: &BTreeMap<CandidateKey, CandidateNode>,
    edges: &BTreeMap<CandidateKey, BTreeSet<CandidateKey>>,
) -> Result<Vec<CandidateKey>, DynamicRegistryError> {
    let mut indegree = nodes
        .keys()
        .cloned()
        .map(|key| (key, 0usize))
        .collect::<BTreeMap<_, _>>();
    for targets in edges.values() {
        for target in targets {
            *indegree.entry(target.clone()).or_default() += 1;
        }
    }
    let mut remaining = nodes.keys().cloned().collect::<BTreeSet<_>>();
    let mut order = Vec::with_capacity(nodes.len());
    while !remaining.is_empty() {
        let next = remaining
            .iter()
            .filter(|key| indegree.get(*key).copied().unwrap_or(0) == 0)
            .min_by_key(|key| candidate_sort_key(nodes.get(*key).expect("remaining node exists")))
            .cloned();
        let Some(next) = next else {
            let ids = remaining.iter().map(candidate_key_name).collect();
            return Err(DynamicRegistryError::PriorityCycle { ids });
        };
        remaining.remove(&next);
        order.push(next.clone());
        for target in edges.get(&next).into_iter().flatten() {
            if let Some(value) = indegree.get_mut(target) {
                *value = value.saturating_sub(1);
            }
        }
    }
    Ok(order)
}

fn candidate_sort_key(node: &CandidateNode) -> (u8, i32, u8, usize, u64, CandidateKey) {
    (
        node.kind.order(),
        node.priority,
        node.source_class,
        node.component_load_order,
        node.declaration_order,
        node.key.clone(),
    )
}

fn document<'a>(
    inner: &'a RegistryInner,
    document_id: &str,
    document_revision: u64,
) -> Result<&'a DocumentLayer, DynamicRegistryError> {
    let latest = inner
        .latest_document_revisions
        .get(document_id)
        .copied()
        .ok_or_else(|| DynamicRegistryError::UnknownDocument {
            document_id: document_id.to_owned(),
            document_revision,
        })?;
    if latest != document_revision {
        return Err(DynamicRegistryError::StaleDocumentRevision {
            document_id: document_id.to_owned(),
            actual: document_revision,
            latest,
        });
    }
    inner
        .documents
        .get(document_id)
        .filter(|document| document.revision == document_revision)
        .ok_or_else(|| DynamicRegistryError::UnknownDocument {
            document_id: document_id.to_owned(),
            document_revision,
        })
}

fn document_mut<'a>(
    inner: &'a mut RegistryInner,
    document_id: &str,
    document_revision: u64,
) -> Result<&'a mut DocumentLayer, DynamicRegistryError> {
    let latest = inner
        .latest_document_revisions
        .get(document_id)
        .copied()
        .ok_or_else(|| DynamicRegistryError::UnknownDocument {
            document_id: document_id.to_owned(),
            document_revision,
        })?;
    if latest != document_revision {
        return Err(DynamicRegistryError::StaleDocumentRevision {
            document_id: document_id.to_owned(),
            actual: document_revision,
            latest,
        });
    }
    inner
        .documents
        .get_mut(document_id)
        .filter(|document| document.revision == document_revision)
        .ok_or_else(|| DynamicRegistryError::UnknownDocument {
            document_id: document_id.to_owned(),
            document_revision,
        })
}

fn validate_identifier(kind: &str, value: &str) -> Result<(), DynamicRegistryError> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        Err(DynamicRegistryError::InvalidInput {
            message: format!("{kind} must not be blank or contain control characters"),
        })
    } else if value.len() > 1024 {
        Err(DynamicRegistryError::InvalidInput {
            message: format!("{kind} exceeds 1024 UTF-8 bytes"),
        })
    } else {
        Ok(())
    }
}

fn validate_local_id(value: &str) -> Result<(), DynamicRegistryError> {
    validate_identifier("dynamic syntax local ID", value)?;
    if value.contains('/') {
        Err(DynamicRegistryError::InvalidInput {
            message: "dynamic syntax local ID must not contain '/'".to_owned(),
        })
    } else {
        Ok(())
    }
}

fn validate_handler(value: &str) -> Result<(), DynamicRegistryError> {
    validate_identifier("dynamic syntax handler", value)
}

fn validate_metadata(metadata: &BTreeMap<String, String>) -> Result<(), DynamicRegistryError> {
    if metadata.len() > MAX_METADATA_ENTRIES {
        return Err(DynamicRegistryError::InvalidInput {
            message: format!(
                "dynamic syntax metadata has {} entries, maximum is {MAX_METADATA_ENTRIES}",
                metadata.len()
            ),
        });
    }
    for key in metadata.keys() {
        validate_identifier("dynamic syntax metadata key", key)?;
    }
    Ok(())
}

fn reference_name(reference: &SyntaxReference) -> String {
    match reference {
        SyntaxReference::Dynamic(id) => id.qualified(),
        SyntaxReference::Definition(id) => format!("definition:{}", id.as_str()),
        SyntaxReference::Registration(id) => format!("registration:{}", id.as_str()),
    }
}

fn override_target_name(target: &SyntaxOverrideTarget) -> String {
    match target {
        SyntaxOverrideTarget::Definition(id) => format!("definition:{}", id.as_str()),
        SyntaxOverrideTarget::Registration(id) => format!("registration:{}", id.as_str()),
    }
}

fn candidate_key_name(key: &CandidateKey) -> String {
    match key {
        CandidateKey::Static(index) => format!("static:{index}"),
        CandidateKey::Dynamic(id) => id.qualified(),
    }
}

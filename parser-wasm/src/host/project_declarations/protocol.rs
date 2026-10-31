//! Opt-in data carrier and native prototype arena. The registry owns atomic
//! visibility; the arena owns authenticated data, never guest-supplied owners.
use super::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Mutex, atomic::AtomicU64};
use syntaxes::{NativeSyntaxBinding, NativeSyntaxBindings};

pub(in crate::host) const CAPABILITY: &str = "parser.project-declarations";
pub(in crate::host) const OPTION: &str = "native.project-declarations.v1";
pub(in crate::host) const EXPORT: &str = "parser.project-export.v1";
pub(in crate::host) const IMPORT: &str = "parser.project-import.v1";
pub(in crate::host) const STATUS: &str = "parser.project-completeness.v1";
const MAX_ITEMS: usize = 256;
const MAX_OPTION_BYTES: usize = 64 * 1024;
const MAX_ALLOCATIONS: usize = 1024;
const MAX_ARENA_BYTES: usize = 256 * 1024;
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

fn token() -> Result<u64, String> {
    let mut current = NEXT_TOKEN.load(Ordering::Relaxed);
    loop {
        let next = current.checked_add(1).ok_or("project declaration token exhausted")?;
        match NEXT_TOKEN.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return Ok(current), Err(actual) => current = actual,
        }
    }
}
fn number(value: &str) -> Result<u64, String> {
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|c| c.is_ascii_digit())
        || value.len() > 1 && value.starts_with('0') { return Err("invalid opaque project identity".into()); }
    value.parse().map_err(|_| "invalid opaque project identity".into())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Tree { version: u32, epoch_high: String, epoch_low: String, token: String, generation: String }
impl Tree {
    fn reference(&self) -> Result<tree_view::Reference, String> {
        if self.version != 1 { return Err("unsupported project tree reference".into()); }
        Ok(tree_view::Reference { epoch_high: number(&self.epoch_high)?, epoch_low: number(&self.epoch_low)?,
            token: number(&self.token)?, generation: number(&self.generation)? })
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExportDraft { version: u32, tree: Tree, root: String, namespace: String, key: String,
    visibility: String, callable: bool }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ImportDraft { version: u32, reference: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Complete { version: u32, complete: bool }

#[derive(Clone, Debug)]
pub(in crate::host) struct Origin {
    pub(in crate::host) source: SourceLease,
    pub(in crate::host) root: u64,
    pub(in crate::host) header: MappedSpan,
    pub(in crate::host) namespace: String,
    pub(in crate::host) key: String,
    local: bool,
}
#[derive(Clone, Debug)]
pub(in crate::host) struct Binding {
    pub(in crate::host) owner: String,
    pub(in crate::host) input: DynamicSyntaxInput,
    pub(in crate::host) origin: Origin,
    imported: bool,
}
impl Binding {
    fn import_id(&self) -> String {
        format!("project-{:x}", Sha256::digest(format!("{}\0{}\0{}\0{}",
            self.origin.source.stamp.document_id, self.origin.namespace, self.owner, self.input.local_id).as_bytes()))
    }
    fn imported_input(&self) -> DynamicSyntaxInput {
        let mut input = self.input.clone(); input.local_id = self.import_id(); input
    }
    fn semantic_key(&self) -> String {
        format!("{}|{:?}|{}|{}|{}|{:?}", self.owner, self.origin.source.stamp,
            self.origin.namespace, self.origin.key, self.origin.local, self.input)
    }
}

struct ExportData { source: SourceLease, view: Arc<SourceView>, provider: ProviderStamp,
    complete: bool, imports_complete: bool, provider_current: bool, bindings: Vec<Arc<Binding>>,
    selected: BTreeMap<String, Arc<Binding>>, import_bytes: usize, key: String }

/// Immutable, host-authenticated header data. It contains no foreign AST,
/// receipt or getter body. Current authority must still be checked when consumed.
#[derive(Clone)]
pub struct DeclarationExports(Arc<ExportData>);
impl std::fmt::Debug for DeclarationExports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeclarationExports").field("source", &self.0.source.stamp)
            .field("complete", &self.0.complete).field("headers", &self.0.bindings.len()).finish()
    }
}
impl DeclarationExports {
    /// Stable descriptors only. Attempt revisions, references and Arc identities
    /// do not participate in the project fixed-point key.
    pub fn fingerprint(&self) -> &str { &self.0.key }
    /// Source identity, not a claim that this source is still current.
    pub fn source(&self) -> &DeclarationSource { &self.0.source.stamp }
    /// Whether all expected declaration hooks reported complete collection.
    pub fn complete(&self) -> bool { self.0.complete }
    /// Whether every requested foreign header could be materialized within bounds.
    /// Separate from completeness of this source's own exported headers.
    pub fn imports_complete(&self) -> bool { self.0.imports_complete }
    /// Maximum encoded import-option bytes actually sent to a declaration hook.
    /// Resource observation only; never semantic, type or completeness authority.
    pub fn import_bytes(&self) -> usize { self.0.import_bytes }
    pub(in crate::host) fn locations(&self, consumer: Arc<editor_completion::typed::Fence>) -> DeclarationLocations {
        DeclarationLocations { data: Arc::clone(&self.0), consumer }
    }

    pub(in crate::host) fn current(&self, project: &str, authority: &SourceAuthority, provider: &ProviderStamp) -> bool {
        self.0.provider.matches(provider) && self.0.view.view_is_current(authority, project)
            && authority.is_current(project, &self.0.source)
    }
}

/// Current-result editing locations backed by native registration provenance.
/// This is not a type, callback, purity or executable-body certificate.
#[derive(Clone)]
pub struct DeclarationLocations { data: Arc<ExportData>, consumer: Arc<editor_completion::typed::Fence> }
impl std::fmt::Debug for DeclarationLocations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeclarationLocations").field("bindings", &self.data.selected.len()).finish()
    }
}
impl DeclarationLocations {
    /// Both the consumer revision and the complete source/provider view must live.
    pub fn is_current(&self) -> bool {
        self.consumer.current.load(Ordering::Acquire) && self.consumer.providers.is_current()
            && self.data.provider_current && self.data.provider.epoch.load(Ordering::Acquire)
            && self.data.view.view.live.load(Ordering::Acquire) && self.data.source.is_live()
    }
    /// Returns original source identity/header, without executing or moving a getter.
    pub fn site(&self, registration: &str) -> Option<(&DeclarationSource, &MappedSpan)> {
        if !self.is_current() {return None;}
        let binding=self.data.selected.get(registration)?;
        binding.origin.source.is_live().then_some((&binding.origin.source.stamp, &binding.origin.header))
    }
}

pub(in crate::host) struct Arena {
    pub(in crate::host) project: String,
    pub(in crate::host) view: Arc<SourceView>,
    pub(in crate::host) provider: ProviderStamp,
    pub(in crate::host) expected: BTreeSet<(String, String)>,
    import_bytes: usize,
    pub(in crate::host) body_key: String,
    pub(in crate::host) imports_complete: bool,
    pub(in crate::host) project_mode: bool,
    body_inputs: BTreeMap<(String, String), String>,
    inputs: Vec<DeclarationExports>,
    bindings: BTreeMap<u64, Arc<Binding>>,
    allocations: usize,
    bytes: usize,
    violated: bool,
}
impl Arena {
    pub(in crate::host) fn new(project: String, view: SourceView, provider: ProviderStamp,
        inputs: &DeclarationInputs, expected: BTreeSet<(String, String)>) -> Self
    {
        Self { project, view: Arc::new(view), provider, expected, import_bytes: 0,
            body_key: String::new(), imports_complete: true, project_mode: inputs.current.is_some(), body_inputs: BTreeMap::new(),
            inputs: inputs.exports.clone(), bindings: BTreeMap::new(), allocations: 0, bytes: 0, violated: false }
    }
    fn allocate(&mut self, binding: Binding) -> Result<NativeSyntaxBinding, String> {
        let bytes = input_bytes(&binding.input).saturating_add(binding.origin.source.stamp.bytes())
            .saturating_add(binding.origin.namespace.len()).saturating_add(binding.origin.key.len())
            .saturating_add(binding.origin.header.origins.len().saturating_mul(64));
        if self.allocations >= MAX_ALLOCATIONS || self.bytes.saturating_add(bytes) > MAX_ARENA_BYTES {
            self.violated = true;
            return Err("project declaration staging quota exceeded".into());
        }
        let token = token()?;
        self.allocations += 1; self.bytes += bytes;
        self.bindings.insert(token, Arc::new(binding));
        Ok(NativeSyntaxBinding { token, callable: true })
    }
    pub(in crate::host) fn finish(&self, annotations: NativeSyntaxBindings, provider: &ProviderStamp,
        header_execution_complete: bool) -> Result<DeclarationExports, String>
    {
        let complete = header_execution_complete && !self.violated && self.provider.matches(provider)
            && self.expected.iter().all(|hook| annotations.completeness.get(hook) == Some(&true));
        let mut bindings = Vec::new();
        let mut selected = BTreeMap::new();
        for (id, annotation) in annotations.definitions {
            let binding = self.bindings.get(&annotation.token).ok_or("native project binding is not in its arena")?;
            if id.component_id != binding.owner || id.local_id != binding.input.local_id {
                return Err("native project binding differs from its final definition".into());
            }
            if annotation.callable { selected.insert(id.qualified(), Arc::clone(binding)); }
            if !binding.imported && !binding.origin.local { bindings.push(Arc::clone(binding)); }
        }
        bindings.sort_by_key(|binding| (binding.owner.clone(), binding.input.local_id.clone()));
        let key = format!("{:x}", Sha256::digest(format!("{:?}|{}|{:?}", self.view.current.stamp, complete,
            bindings.iter().map(|b| b.semantic_key()).collect::<Vec<_>>()).as_bytes()));
        Ok(DeclarationExports(Arc::new(ExportData { source: self.view.current.clone(), view: Arc::clone(&self.view),
            provider: self.provider.clone(), complete, imports_complete: self.imports_complete,
            provider_current: self.provider.matches(provider), bindings, selected, import_bytes: self.import_bytes, key })))
    }
}

pub(in crate::host) struct Invocation {
    pub(in crate::host) arena: Arc<Mutex<Arena>>,
    original: ParseRequest,
    owner: String,
    hook: String,
    tree: tree_view::Reference,
    imports: BTreeMap<u64, Arc<Binding>>,
}

fn lock(arena: &Mutex<Arena>) -> Result<std::sync::MutexGuard<'_, Arena>, String> {
    arena.lock().map_err(|_| "project declaration arena poisoned".into())
}

impl Invocation {
    pub(in crate::host) fn prepare(arena: Arc<Mutex<Arena>>, authority: &SourceAuthority,
        owner: &str, hook: &str, tree: tree_view::Reference, payload: &mut HookPayload) -> Result<Self, String>
    {
        let HookPayload::Parser(request) = payload else { return Err("invalid project declaration carrier".into()); };
        if request.parser_id != declaration_prepass::PARSER_ID || request.options.len() != 1 {
            return Err("invalid project declaration input options".into());
        }
        let mut frame = lock(&arena)?;
        let mut records = BTreeMap::new();
        let mut complete = frame.project_mode && frame.view.complete;
        for record in &frame.inputs {
            if record.0.source.stamp.document_id == frame.view.current.stamp.document_id { continue; }
            if !record.current(&frame.project, authority, &frame.provider)
                || !frame.view.permits(authority, &frame.project, &record.0.source)
                || records.insert(record.0.source.stamp.document_id.clone(), record.clone()).is_some()
            { complete = false; }
        }
        for document in frame.view.membership.keys().filter(|document| **document != frame.view.current.stamp.document_id) {
            if !records.get(document).is_some_and(DeclarationExports::complete) { complete = false; }
        }
        let mut imports = BTreeMap::new();
        let mut descriptors = Vec::new();
        let mut descriptor_bytes = 128usize;
        let mut body_bindings = Vec::new();
        if complete {
            for record in records.values() {
                for binding in record.0.bindings.iter().filter(|binding| binding.owner == owner) {
                    if imports.len() >= MAX_ITEMS { complete = false; break; }
                    let reference = token()?;
                    let mut definition = wire(&binding.input);
                    definition["localId"] = binding.import_id().into();
                    let descriptor = json!({"reference":reference.to_string(),"definition":definition,
                        "origin":{"uri":binding.origin.source.stamp.uri,"namespace":binding.origin.namespace,"key":binding.origin.key}});
                    let bytes = serde_json::to_vec(&descriptor).map_err(|e|e.to_string())?.len();
                    if bytes.saturating_add(descriptor_bytes) > MAX_OPTION_BYTES {
                        complete = false; break;
                    }
                    descriptor_bytes = descriptor_bytes.saturating_add(bytes);
                    descriptors.push(descriptor);
                    body_bindings.push(format!("{}|{}|{}", binding.semantic_key(), binding.origin.source.lifetime, frame.provider.semantic_key()));
                    imports.insert(reference, Arc::clone(binding));
                }
            }
        }
        let encode = |complete: bool, descriptors: &[Value]| serde_json::to_string(&json!({"version":1,
            "project":frame.project_mode,"complete":complete,"declarations":descriptors})).map_err(|e|e.to_string());
        let mut value = encode(complete, &descriptors)?;
        if !complete || value.len() > MAX_OPTION_BYTES {
            complete = false; imports.clear(); descriptors.clear(); body_bindings.clear();
            value = encode(false, &descriptors)?;
        }
        frame.import_bytes = frame.import_bytes.max(value.len());
        frame.imports_complete &= complete;
        let body_key = format!("{:x}", Sha256::digest(format!("{}|{}|{:?}", frame.project_mode, complete, body_bindings).as_bytes()));
        frame.body_inputs.insert((owner.into(), hook.into()), body_key);
        frame.body_key = format!("{:x}", Sha256::digest(format!("{}|{:?}", frame.provider.semantic_key(), frame.body_inputs).as_bytes()));
        request.options.push(WitMetadataEntry { key: OPTION.into(), owner_component_id: None, value });
        let original = request.clone();
        drop(frame);
        Ok(Self { arena, original, owner: owner.into(), hook: hook.into(), tree, imports })
    }

    /// Called only after full hook-output accounting; no echoed bytes escape
    /// the existing output quota by being consumed as a native status response.
    pub(in crate::host) fn consume_status(&self, output: &mut HookOutput, update: &mut DynamicSyntaxUpdate) -> Result<(), String> {
        if let Some(complete) = self.status(output)? {
            update.report_native_completeness(&self.hook, complete).map_err(|e|e.to_string())?;
            output.replacement = None;
        }
        Ok(())
    }

    fn status(&self, output: &HookOutput) -> Result<Option<bool>, String> {
        if !matches!(output.decision, HookDecision::ContinueProcessing) { return Ok(None); }
        let Some(replacement) = output.replacement.as_ref() else { return Ok(None); };
        let HookPayload::Parser(reply) = replacement else { return Err("project status requires its original Parser".into()); };
        if reply.options.len() != self.original.options.len() + 1 {
            return Err("project status changed input option count".into());
        }
        let status = reply.options.last().unwrap();
        if status.key != STATUS || status.owner_component_id.is_some() || status.value.len() > 1024
            || !same_request(reply, &self.original) { return Err("project status changed immutable input".into()); }
        let status: Complete = serde_json::from_str(&status.value).map_err(|e|e.to_string())?;
        if status.version != 1 { return Err("unsupported project completeness version".into()); }
        Ok(Some(status.complete))
    }

    pub(in crate::host) fn invalidate(&self) {
        if let Ok(mut frame) = self.arena.lock() { frame.violated = true; }
    }

    pub(in crate::host) fn bind(&self, input: &DynamicSyntaxInput, entry: &WitMetadataEntry,
        lease: &mut tree_view::Lease) -> Result<NativeSyntaxBinding, String>
    {
        let result = self.bind_inner(input, entry, lease);
        if result.is_err() { if let Ok(mut frame) = lock(&self.arena) { frame.violated = true; } }
        result
    }

    fn bind_inner(&self, input: &DynamicSyntaxInput, entry: &WitMetadataEntry,
        lease: &mut tree_view::Lease) -> Result<NativeSyntaxBinding, String>
    {
        if entry.owner_component_id.as_ref().is_some_and(|owner| owner != &self.owner) {
            return Err("foreign project declaration owner".into());
        }
        if entry.value.len() > 4096 { return Err("project declaration draft byte limit".into()); }
        if lease.reference() != self.tree { return Err("project declaration belongs to another invocation/store".into()); }
        let mut frame = lock(&self.arena)?;
        if input_bytes(input) > MAX_ARENA_BYTES { return Err("project signature byte limit".into()); }
        if !frame.provider.epoch.load(Ordering::Acquire) || !frame.view.current.is_live()
            || !frame.view.view.live.load(Ordering::Acquire) { return Err("stale project declaration invocation".into()); }
        let binding = if entry.key == EXPORT {
            let draft: ExportDraft = serde_json::from_str(&entry.value).map_err(|e|e.to_string())?;
            if draft.version != 1 || draft.tree.reference()? != self.tree || draft.namespace.is_empty()
                || draft.namespace.len() > 128 || draft.namespace.chars().any(char::is_control)
                || draft.key.is_empty() || draft.key.len() > 1024 || draft.key.chars().any(char::is_control)
                || !matches!(draft.visibility.as_str(), "document" | "project")
            { return Err("invalid current project export draft".into()); }
            let root = number(&draft.root)?;
            let tree = lease.read(self.tree).map_err(|e|e.to_string())?;
            if !tree.roots.iter().any(|id| id.get() == root) { return Err("project declaration is not a native root".into()); }
            let node = tree.nodes.iter().find(|node| node.id.get() == root).ok_or("missing native declaration root")?;
            if matches!(node.kind, skript_parser::RawNodeKind::Blank | skript_parser::RawNodeKind::Comment | skript_parser::RawNodeKind::Invalid) {
                return Err("invalid native declaration root".into());
            }
            validate_ordering(input, frame.provider.catalog.as_deref())?;
            if input_bytes(input).saturating_add(frame.view.current.stamp.bytes())
                .saturating_add(node.line.content_span.origins.len().saturating_mul(64))
                .saturating_add(draft.namespace.len()).saturating_add(draft.key.len()) > MAX_ARENA_BYTES.saturating_sub(frame.bytes)
            { return Err("project declaration staging byte limit".into()); }
            let source = frame.view.current.clone();
            let mut binding = frame.allocate(Binding { owner: self.owner.clone(), input: input.clone(),
                origin: Origin { source, root, header: mapped_span_to_wit(node.line.content_span.clone()),
                    namespace: draft.namespace, key: draft.key, local: draft.visibility == "document" }, imported: false })?;
            binding.callable = draft.callable;
            return Ok(binding);
        } else if entry.key == IMPORT {
            let draft: ImportDraft = serde_json::from_str(&entry.value).map_err(|e|e.to_string())?;
            if draft.version != 1 { return Err("unsupported project import version".into()); }
            let imported = self.imports.get(&number(&draft.reference)?).ok_or("stale or foreign project import reference")?;
            if input_bytes(input).saturating_add(imported.origin.source.stamp.bytes())
                .saturating_add(imported.origin.header.origins.len().saturating_mul(64))
                .saturating_add(imported.origin.namespace.len()).saturating_add(imported.origin.key.len()) > MAX_ARENA_BYTES.saturating_sub(frame.bytes)
            { return Err("project declaration import staging byte limit".into()); }
            if *input != imported.imported_input() { return Err("project import changed its immutable signature".into()); }
            Binding { owner: self.owner.clone(), input: input.clone(), origin: imported.origin.clone(), imported: true }
        } else { return Err("unknown project declaration draft".into()); };
        frame.allocate(binding)
    }
}

fn input_bytes(input: &DynamicSyntaxInput) -> usize {
    let mut bytes = input.local_id.len().saturating_add(input.handler.len()).saturating_add(128);
    for pattern in &input.patterns { bytes = bytes.saturating_add(pattern.len()); }
    for (key, value) in &input.metadata { bytes = bytes.saturating_add(key.len()).saturating_add(value.len()); }
    bytes = bytes.saturating_add(input.return_type.as_ref().map_or(0, String::len));
    for reference in input.before.iter().chain(&input.after) {
        bytes = bytes.saturating_add(match reference {
            SyntaxReference::Definition(id) => id.as_str().len(),
            SyntaxReference::Registration(id) => id.as_str().len(),
            SyntaxReference::Dynamic(id) => id.component_id.len().saturating_add(id.local_id.len()),
        });
    }
    if let Some(validator) = &input.entry_validator { bytes = bytes.saturating_add(validator_bytes(validator)); }
    bytes
}
fn validator_bytes(validator: &syntaxes::EntryValidator) -> usize {
    let mut bytes = 0usize;
    let mut validators = vec![validator];
    while let Some(validator) = validators.pop() {
      for entry in &validator.entry_data {
        bytes = bytes.saturating_add(128).saturating_add(entry.key.len()).saturating_add(entry.entry_data_class.as_str().len())
            .saturating_add(entry.separator.as_ref().map_or(0, String::len))
            .saturating_add(entry.string_mode.as_ref().map_or(0, String::len))
            .saturating_add(entry.value_type.as_ref().map_or(0, |v|v.as_str().len()));
        for ty in &entry.return_types { bytes = bytes.saturating_add(ty.as_str().len()); }
        let mut values = entry.default_value.iter().collect::<Vec<_>>();
        while let Some(value) = values.pop() {
            bytes = bytes.saturating_add(16);
            match value {
                Value::String(value) => bytes = bytes.saturating_add(value.len()),
                Value::Array(value) => {
                    if values.len().saturating_add(value.len()) > MAX_ARENA_BYTES / 16 { return MAX_ARENA_BYTES + 1; }
                    values.extend(value);
                },
                Value::Object(value) => {
                    if values.len().saturating_add(value.len()) > MAX_ARENA_BYTES / 16 { return MAX_ARENA_BYTES + 1; }
                    for (key, value) in value { bytes = bytes.saturating_add(key.len()); values.push(value); }
                },
                _ => {},
            }
            if bytes > MAX_ARENA_BYTES { return bytes; }
        }
        if let Some(nested) = &entry.nested_validator { validators.push(nested); }
        if bytes > MAX_ARENA_BYTES { return bytes; }
      }
    }
    bytes
}

fn same_request(reply: &ParseRequest, original: &ParseRequest) -> bool {
    reply.request_id == original.request_id && reply.parser_id == original.parser_id && reply.input == original.input
        && reply.expected_types.len() == original.expected_types.len()
        && reply.expected_types.iter().zip(&original.expected_types).all(|(a,b)| a.class_name == b.class_name && a.plural == b.plural)
        && reply.span.virtual_range.start == original.span.virtual_range.start && reply.span.virtual_range.end == original.span.virtual_range.end
        && reply.span.origins.len() == original.span.origins.len()
        && reply.span.origins.iter().zip(&original.span.origins).all(|(a,b)| a.original_range.start == b.original_range.start
            && a.original_range.end == b.original_range.end && a.kind == b.kind && a.expansion == b.expansion)
        && reply.options.iter().take(original.options.len()).zip(&original.options).all(|(a,b)|
            a.key == b.key && a.value == b.value && a.owner_component_id == b.owner_component_id)
}

fn validate_ordering(input: &DynamicSyntaxInput, catalog: Option<&Catalog>) -> Result<(), String> {
    for reference in input.before.iter().chain(&input.after) {
        let Some(catalog) = catalog else { return Err("project ordering requires a static catalog".into()); };
        let targets = match reference {
            SyntaxReference::Definition(id) => catalog.syntaxes().iter().filter(|s|s.definition_id() == id).collect::<Vec<_>>(),
            SyntaxReference::Registration(id) => catalog.syntax_by_registration_id(id.as_str()),
            SyntaxReference::Dynamic(_) => return Err("project prototypes cannot export local dynamic ordering dependencies".into()),
        };
        if targets.is_empty() || targets.iter().any(|syntax| syntax.kind() != input.kind) {
            return Err("invalid static ordering reference in project prototype".into());
        }
    }
    Ok(())
}

fn reference_wire(reference: &SyntaxReference) -> Value {
    match reference {
        SyntaxReference::Definition(id) => json!({"definitionId":id.as_str()}),
        SyntaxReference::Registration(id) => json!({"registrationId":id.as_str()}),
        SyntaxReference::Dynamic(id) => json!({"dynamic":{"componentId":id.component_id,"localId":id.local_id}}),
    }
}
fn validator_wire(validator: &syntaxes::EntryValidator) -> Value {
    json!({"entryData":validator.entry_data.iter().map(|entry| json!({"key":entry.key,"defaultValue":entry.default_value,
        "optional":entry.optional,"multiple":entry.multiple,"entryDataClass":entry.entry_data_class.as_str(),
        "kind":match entry.kind {syntaxes::EntryKind::Literal=>"literal",syntaxes::EntryKind::VariableString=>"variable-string",
            syntaxes::EntryKind::Expression=>"expression",syntaxes::EntryKind::Trigger=>"trigger",syntaxes::EntryKind::Container=>"container",
            syntaxes::EntryKind::Section=>"section",syntaxes::EntryKind::KeyValue=>"key-value",syntaxes::EntryKind::Unknown=>"unknown"},
        "separator":entry.separator,"valueType":entry.value_type.as_ref().map(|t|t.as_str()),"stringMode":entry.string_mode,
        "returnTypes":entry.return_types.iter().map(|t|t.as_str()).collect::<Vec<_>>(),"flags":entry.flags,
        "nestedValidator":entry.nested_validator.as_ref().map(validator_wire)})).collect::<Vec<_>>()})
}
fn wire(input: &DynamicSyntaxInput) -> Value {
    json!({"localId":input.local_id,"kind":match input.kind {CatalogSyntaxKind::Event=>"event",CatalogSyntaxKind::Condition=>"condition",
        CatalogSyntaxKind::Effect=>"effect",CatalogSyntaxKind::Expression=>"expression",CatalogSyntaxKind::Type=>"type",CatalogSyntaxKind::Function=>"function",
        CatalogSyntaxKind::Section=>"section",CatalogSyntaxKind::Structure=>"structure"},
        "patterns":input.patterns,"priority":input.priority,"before":input.before.iter().map(reference_wire).collect::<Vec<_>>(),
        "after":input.after.iter().map(reference_wire).collect::<Vec<_>>(),"returnType":input.return_type,
        "returnMultiplicity":input.return_multiplicity.map(|v|match v{DynamicMultiplicity::Single=>"single",DynamicMultiplicity::Multiple=>"multiple",DynamicMultiplicity::Both=>"both"}),
        "structureNodeType":input.structure_node_type.map(|v|match v{syntaxes::NodeType::Simple=>"simple",syntaxes::NodeType::Section=>"section",syntaxes::NodeType::Both=>"both"}),
        "structureBodyMode":input.structure_body_mode.map(|v|match v{syntaxes::DynamicStructureBodyMode::None=>"none",syntaxes::DynamicStructureBodyMode::Raw=>"raw",
            syntaxes::DynamicStructureBodyMode::Entries=>"entries",syntaxes::DynamicStructureBodyMode::Trigger=>"trigger"}),
        "entryValidator":input.entry_validator.as_ref().map(validator_wire),"handler":input.handler,"metadata":input.metadata})
}

#[cfg(test)]
mod tests;

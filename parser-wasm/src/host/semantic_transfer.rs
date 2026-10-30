//! Current-revision, owner-authenticated transfers of frontend type bounds.
//! This is independent of editing suggestions and does not import old scopes.
use super::*;
use crate::bindings::exports::nlaocs::skript_parser_addon::semantic_transfer as wire;
use serde::Deserialize;
use std::time::Instant;

pub use wire::{BindingEffects, Output, SourceUpdate, TypeBound};
pub const METADATA: &str = "parser.semantic-transfer.v1";
const MAX_CALLS: usize = 2048;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_CALL_BYTES: usize = 128 * 1024;
const MAX_OPERATIONS: usize = 2048;
const MAX_OPERATION_BYTES: usize = 2 * 1024 * 1024;
const MAX_TIME: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub(super) struct Fence {
    document: Arc<editor_completion::typed::Fence>,
    source_digest: String,
    snapshot_id: String,
    owners: Vec<(String, String, u64, bool, bool)>,
    epoch:Arc<AtomicBool>,
}

pub(super) struct Authority(Arc<AtomicBool>);
impl Default for Authority {fn default()->Self {Self(Arc::new(AtomicBool::new(true)))}}
impl Authority {pub(super) fn revoke(&mut self){self.0.store(false,Ordering::Release);self.0=Arc::new(AtomicBool::new(true));}}
impl Drop for Authority {fn drop(&mut self){self.0.store(false,Ordering::Release);}}
/// Revocable lifetime for frontend facts computed by these data-only queries.
#[derive(Clone,Debug)]
pub struct Certificate {document:Arc<editor_completion::typed::Fence>,epoch:Arc<AtomicBool>}
impl Certificate {
    pub fn is_current(&self)->bool {self.epoch.load(Ordering::Acquire)&&self.document.current.load(Ordering::Acquire)&&self.document.providers.is_current()}
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Operation {
    version: u32,
    operation: String,
    parameters: String,
    #[serde(default)]
    output_use: Option<String>,
}

/// A synchronous flow pass over this exact result. Upper bounds are supplied by
/// the frontend's current semantic evaluator, never a guest editing proposal.
pub struct Session<'a> {
    host: &'a mut ParserHost,
    result: &'a DocumentParseResult,
    operations: HashMap<u64, (String, Operation, u64)>,
    cancellation: Option<DocumentCancellationToken>,
    calls: usize,
    bytes: usize,
    used: Duration,
}

fn valid_bound(bound: &TypeBound) -> bool {
    bound.complete && !bound.multiple && bound.types.len() <= 64
        && (!bound.types.is_empty() || bound.maybe_unset)
        && bound.types.iter().all(|c| !c.is_empty() && c.len() <= 1024 && c != "java.lang.Object" && !c.chars().any(char::is_control))
        && bound.types.iter().collect::<BTreeSet<_>>().len() == bound.types.len()
}
fn bound_bytes(bound: &TypeBound) -> usize { bound.types.iter().map(String::len).sum::<usize>() + 16 }

impl ParserHost {
    pub(super) fn transfer_fence(&self, document: Arc<editor_completion::typed::Fence>) -> Option<Fence> {
        let source = self.config.syntax_catalog.as_deref()?.source()?;
        Some(Fence { document, source_digest: source.source_digest.clone(), snapshot_id: source.snapshot_id.clone(),
            owners: self.components.iter().map(|c| (c.manifest.component_id.clone(), c.component_sha256.clone(), c.preparation_generation,c.disabled,c.unloaded)).collect(),
            epoch:Arc::clone(&self.transfer_authority.0) })
    }

    pub fn semantic_transfer_session<'a>(&'a mut self, result: &'a DocumentParseResult,
        cancellation: Option<DocumentCancellationToken>) -> Session<'a> {
        let witnesses = result.semantic_witnesses();
        let mut operations = HashMap::new();
        let mut operation_bytes = 0usize;
        for node in &result.ast.nodes {
            let selected = (|| {
            let mut entries = node.metadata.iter().filter(|e| e.key == METADATA);
            let entry = entries.next()?;
            if entries.next().is_some() || entry.value.len() > 32768 { return None; }
            let owner = entry.owner_component_id.as_deref()?;
            if !witnesses.provider_valid(node, owner) && !witnesses.composed_metadata_valid(node, entry) { return None; }
            let operation: Operation = serde_json::from_str(&entry.value).ok()?;
            if !matches!((operation.version,operation.output_use.as_deref()),(1,None)|(2,Some("flow"|"completion-candidate"))) || operation.operation.is_empty() || operation.operation.len() > 256
                || operation.parameters.len() > 8192 || operation.operation.chars().any(char::is_control) { return None; }
            Some((owner.to_owned(), operation, entry.value.len()))
            })();
            if let Some((owner, operation, bytes)) = selected {
                if operations.len() == MAX_OPERATIONS || operation_bytes.saturating_add(bytes) > MAX_OPERATION_BYTES {
                    operations.clear();
                    break;
                }
                operation_bytes += bytes;
                operations.insert(node.id, (owner, operation,node.syntax_context));
            }
        }
        Session { host: self, result, operations, cancellation, calls: 0, bytes: 0, used: Duration::ZERO }
    }
}

impl Session<'_> {
    pub fn certificate(&self)->Option<Certificate> {
        self.current().then(|| {let fence=self.result.transfer_fence.as_ref().expect("current fence");
            Certificate {document:Arc::clone(&fence.document),epoch:Arc::clone(&fence.epoch)}})
    }
    fn current(&self) -> bool {
        let Some(fence) = &self.result.transfer_fence else { return false; };
        let document = &fence.document;
        fence.epoch.load(Ordering::Acquire) && document.current.load(Ordering::Acquire) && document.providers.is_current()
            && self.host.editor_documents.get(&(document.project.clone(), document.document.clone()))
                .is_some_and(|current| Arc::ptr_eq(current, document))
            && self.host.config.syntax_catalog.as_deref().and_then(Catalog::source)
                .is_some_and(|s| s.source_digest == fence.source_digest && s.snapshot_id == fence.snapshot_id)
            && fence.owners.len() == self.host.components.len()
            && fence.owners.iter().zip(&self.host.components).all(|((id, hash, generation, disabled, unloaded), c)|
                id == &c.manifest.component_id && hash == &c.component_sha256 && *generation == c.preparation_generation && *disabled==c.disabled && *unloaded==c.unloaded)
    }

    /// None means unsupported/stale/bounded-out, never an empty-value result.
    pub fn evaluate(&mut self, node_id: u64, input: TypeBound) -> Result<Option<Output>, HostError> {
        if self.cancellation.as_ref().is_some_and(DocumentCancellationToken::is_cancelled) {
            return Err(skript_parser::ParseCancelled.into());
        }
        if !valid_bound(&input) || !self.current() || self.calls >= MAX_CALLS || self.used >= MAX_TIME
            || self.bytes.saturating_add(bound_bytes(&input)) > MAX_BYTES { return Ok(None); }
        let Some((owner, operation, syntax_context)) = self.operations.get(&node_id) else { return Ok(None); };
        if self.bytes.saturating_add(bound_bytes(&input)).saturating_add(operation.parameters.len()) > MAX_BYTES { return Ok(None); }
        let Some(index) = self.host.components.iter().position(|c| &c.manifest.component_id == owner) else { return Ok(None); };
        if !component_promises(&self.host.components[index].manifest.capabilities, crate::CAPABILITY_SEMANTIC_TRANSFER) { return Ok(None); }
        let owner = owner.clone();
        let syntax_context=*syntax_context;
        let operation = (operation.operation.clone(), operation.parameters.clone());
        // A restart changes preparation authority; do not silently rebase an old result.
        self.host.ensure_component_ready(index)?;
        if !self.current() || self.host.components[index].disabled || self.host.components[index].unloaded { return Ok(None); }
        let fence = self.result.transfer_fence.as_ref().expect("current fence");
        let request = wire::Request { subject: wire::Context {
            document_id: fence.document.document.clone(), document_revision: fence.document.revision,
            node_id, syntax_context, source_digest: fence.source_digest.clone(), snapshot_id: fence.snapshot_id.clone(),
            preparation_generation: self.host.components[index].preparation_generation,
        }, operation: operation.0, parameters: operation.1, input };
        self.calls += 1;
        self.bytes += bound_bytes(&request.input) + request.parameters.len();
        let started = Instant::now();
        let entry = &mut self.host.components[index];
        if entry.store.data().invocation.is_some() || entry.store.data().dynamic_syntax_update.is_some() || entry.store.data().tree_view.is_some() {
            return Err(HostError::InvalidHookOutput { component_id: owner, subscription_id: METADATA.into(), message: "semantic transfer requires a completed parse".into() });
        }
        let old_readonly = entry.store.data().editor_readonly;
        let old_input_only = entry.store.data().editor_input_only;
        let old_attempted = entry.store.data().editor_mutation_attempted;
        let old_cancel = mem::replace(&mut entry.store.data_mut().cancellation, self.cancellation.clone());
        entry.store.data_mut().editor_readonly = true;
        entry.store.data_mut().editor_input_only = true;
        entry.store.data_mut().editor_mutation_attempted = false;
        let remaining=MAX_TIME.saturating_sub(self.used);
        let ticks=remaining.as_nanos().div_ceil(self.host.config.epoch_tick.as_nanos()).min(u64::MAX as u128) as u64;
        let deadline = self.host.config.deadline_ticks(&owner).min(ticks.max(1));
        let called = prepare_store(&mut entry.store, self.host.config.fuel_per_call, deadline, &owner, "semantic transfer")
            .and_then(|()| entry.bindings.nlaocs_skript_parser_addon_semantic_transfer().call_evaluate(&mut entry.store, &request)
                .map_err(|error| classify_wasmtime_error(owner.clone(), "semantic transfer", error)))
            .and_then(|answer| answer.map_err(|error| HostError::AddonFailure { component_id: owner.clone(), kind: error.kind, message: error.message }));
        let forbidden = entry.store.data().editor_mutation_attempted;
        entry.store.data_mut().editor_readonly = old_readonly;
        entry.store.data_mut().editor_input_only = old_input_only;
        entry.store.data_mut().editor_mutation_attempted = old_attempted;
        entry.store.data_mut().cancellation = old_cancel;
        self.used += started.elapsed();
        let output = match called {
            Ok(output) => output,
            Err(error) => { if error.disables_component() { self.host.disable_component(index, &error); } return Err(error); }
        };
        if self.cancellation.as_ref().is_some_and(DocumentCancellationToken::is_cancelled) { return Err(skript_parser::ParseCancelled.into()); }
        if !self.current() || self.used > MAX_TIME { return Ok(None); }
        let invalid = |message: &str| HostError::InvalidHookOutput { component_id: owner.clone(), subscription_id: METADATA.into(), message: message.into() };
        if forbidden { return Err(invalid("semantic transfer attempted a state/scope/parse import")); }
        let Some(output) = output else { return Ok(None); };
        if !valid_bound(&output.value) || output.source_writeback.as_ref().is_some_and(|b| !valid_bound(b))
            || matches!(&output.source, SourceUpdate::Known(bound) if !valid_bound(bound))
            || matches!((&output.source, output.effects), (SourceUpdate::Unchanged, BindingEffects::Invalidate)) {
            return Err(invalid("invalid or incomplete semantic type transfer"));
        }
        let bytes = bound_bytes(&output.value) + output.source_writeback.as_ref().map_or(0, bound_bytes)
            + if let SourceUpdate::Known(bound) = &output.source { bound_bytes(bound) } else { 0 };
        if bytes > MAX_CALL_BYTES || self.bytes.saturating_add(bytes) > MAX_BYTES { return Ok(None); }
        self.bytes += bytes;
        Ok(Some(output))
    }
}

#[cfg(test)]
mod tests;

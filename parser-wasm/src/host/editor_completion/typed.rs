//! Frozen native editing observations. No parse receipt or scope is imported.
use super::*;
use std::fmt::{self, Write as _};
use crate::host::semantic_proof::SemanticWitnesses;

pub const CAPABILITY: &str = "parser.editor-typed-inputs";
pub const ROUTE_PREFIX: &str = "editor.refine/";
pub const FACTS: &str = "editor.typed-inputs.v2";
const MAX_OBSERVATIONS: usize = 64;
const MAX_NODES: usize = 128;
const MAX_OBSERVATION_BYTES: usize = 128 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Reference { pub host_token: u64, pub root_id: u64 }

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Draft {
    pub id: String,
    pub subject: Reference,
    pub anchor: Option<Reference>,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug)]
pub(in crate::host) struct Fence {
    pub project: String,
    pub document: String,
    pub revision: u64,
    pub current: AtomicBool,
    pub providers: CompletionTypeCertificate,
}

#[derive(Debug)]
struct Observation {
    tree: AstTree,
    proof: semantic_proof::SemanticProofRegistry,
    bytes: usize,
    fence: Arc<Fence>,
}

/// An immutable subject chosen by a current live query. The private observations
/// are created only from native selected results, never addon-returned node data.
#[derive(Clone, Debug)]
pub struct Input {
    pub id: String,
    pub start: usize,
    pub end: usize,
    owner: String,
    raw_owner: u64,
    line_span: MappedSpan,
    line_input: String,
    subject: Arc<Observation>,
    anchor: Option<Arc<Observation>>,
}

impl Input {
    pub fn owner_raw_node(&self) -> u64 { self.raw_owner }
    pub fn line_span(&self) -> &MappedSpan { &self.line_span }
    pub fn line_input(&self) -> &str { &self.line_input }
    pub fn subject(&self) -> &AstTree { &self.subject.tree }
    pub fn anchor(&self) -> Option<&AstTree> { self.anchor.as_ref().map(|o| &o.tree) }
    pub fn subject_witnesses<'a>(&'a self, source: &MappedSource) -> SemanticWitnesses<'a> {
        self.subject.proof.view_for_source(&self.subject.tree, source)
    }
    pub fn anchor_witnesses<'a>(&'a self, source: &MappedSource) -> Option<SemanticWitnesses<'a>> {
        let anchor = self.anchor.as_ref()?;
        Some(anchor.proof.view_for_source(&anchor.tree, source))
    }
    pub fn is_current(&self) -> bool {
        self.subject.fence.current.load(Ordering::Acquire) && self.subject.fence.providers.is_current()
            && self.anchor.as_ref().is_none_or(|anchor| Arc::ptr_eq(&anchor.fence, &self.subject.fence))
    }
    pub(in crate::host) fn bytes(&self) -> usize {
        self.subject.bytes.saturating_add(self.anchor.as_ref().map_or(0, |o| o.bytes))
    }
}

/// A frontend's current semantic-flow observation for one sealed input. This is
/// editing-only data; the host never turns it into an Expression/type proof.
#[derive(Clone, Debug)]
pub struct TypeFact {
    pub input: Input,
    pub classes: Vec<String>,
    pub complete: bool,
    pub multiple: bool,
    pub maybe_unset: bool,
    pub read_closed: bool,
    /// Incomplete same-flow candidates, usable only as nonsemantic suggestions.
    pub inferred: bool,
}

pub(in crate::host) struct Capture {
    observations: BTreeMap<Reference, Arc<Observation>>,
    bytes: usize,
}
impl Capture {
    pub(in crate::host) fn new() -> Self { Self { observations: BTreeMap::new(), bytes: 0 } }
}

fn tree_bytes(tree: &AstTree) -> Option<usize> {
    struct Size(usize);
    impl fmt::Write for Size {
        fn write_str(&mut self, value: &str) -> fmt::Result {
            self.0 = self.0.saturating_add(value.len());
            if self.0 > MAX_OBSERVATION_BYTES { Err(fmt::Error) } else { Ok(()) }
        }
    }
    let mut size = Size(0);
    write!(&mut size, "{tree:?}").ok()?;
    Some(size.0)
}

impl ParserHost {
    pub(in crate::host) fn record_editor_observation(&mut self, result: &mut ExecutedParseResult) {
        let Some(capture) = self.editor_capture.as_ref() else { return; };
        if capture.observations.len() >= MAX_OBSERVATIONS { return; }
        let Some(session) = self.editor_session.as_ref() else { return; };
        let Some(tree) = result.editor_tree.take() else { return; };
        let [wire_root] = result.wire.roots.as_slice() else { return; };
        let [root] = tree.roots.as_slice() else { return; };
        if tree.nodes.len() > MAX_NODES { return; }
        let Some(bytes) = tree_bytes(&tree) else { return; };
        if capture.bytes.saturating_add(bytes) > MAX_BYTES { return; }
        let proof = semantic_proof::SemanticProofRegistry::capture(&tree, self.semantic_receipts.as_ref(), &result.calls)
            .bind_source(&session.mapped);
        if !tree.nodes.iter().find(|node| node.id == *root)
            .is_some_and(|node| proof.view_for_source(&tree, &session.mapped).native_valid(node)) { return; }
        let reference = Reference { host_token: result.wire.host_token, root_id: *wire_root };
        let observation = Arc::new(Observation { tree, proof, bytes, fence: Arc::clone(&session.fence) });
        let capture = self.editor_capture.as_mut().expect("capture remains active");
        capture.bytes += bytes;
        capture.observations.insert(reference, observation);
    }
}

pub(in crate::host) fn select_inputs(output: &Output, capture: Option<&Capture>, owner: &str, raw_owner: u64,
    span: &MappedSpan, input: &str) -> Option<Vec<Input>> {
    if output.typed_inputs.is_empty() { return Some(Vec::new()); }
    let capture = capture?;
    if output.typed_inputs.len() > 32 { return None; }
    let mut ids = BTreeSet::new();
    let mut selected = Vec::new();
    for draft in &output.typed_inputs {
        if draft.id.is_empty() || draft.id.len() > 128 || draft.id.chars().any(char::is_control)
            || !ids.insert(&draft.id) || draft.start > draft.end || input.get(draft.start..draft.end).is_none() { return None; }
        let subject = capture.observations.get(&draft.subject)?.clone();
        let anchor = match draft.anchor { Some(r) => Some(capture.observations.get(&r)?.clone()), None => None };
        let root = subject.tree.nodes.iter().find(|n| subject.tree.roots.contains(&n.id))?;
        if root.span.virtual_range.start < span.virtual_range.start
            || root.span.virtual_range.end > span.virtual_range.start.checked_add(draft.start as u64)? { return None; }
        if anchor.as_ref().is_some_and(|a| !Arc::ptr_eq(&a.fence, &subject.fence)) { return None; }
        selected.push(Input { id: draft.id.clone(), start: draft.start, end: draft.end,
            owner: owner.into(), raw_owner, line_span: span.clone(), line_input: input.into(), subject, anchor });
    }
    Some(selected)
}

/// Rebase only a fresh native fragment projection. Never reseal mutable AST output.
pub(in crate::host) fn rebase_projection(mut tree: AstTree, request: &ParseRequest) -> Option<AstTree> {
    let [base] = request.span.origins.as_slice() else { return None; };
    if base.kind != WitOriginKind::Exact || base.expansion.is_some()
        || base.original_range.end.checked_sub(base.original_range.start)? != request.input.len() as u64
        || request.span.virtual_range.end.checked_sub(request.span.virtual_range.start)? != request.input.len() as u64 { return None; }
    let map = |span: &MappedSpan| -> Option<MappedSpan> {
        let [origin] = span.origins.as_slice() else { return None; };
        let start = usize::try_from(span.virtual_range.start).ok()?;
        let end = usize::try_from(span.virtual_range.end).ok()?;
        if origin.kind != WitOriginKind::Exact || origin.expansion.is_some()
            || origin.original_range.start != span.virtual_range.start || origin.original_range.end != span.virtual_range.end
            || request.input.get(start..end).is_none() { return None; }
        Some(MappedSpan { virtual_range: WitTextRange { start: request.span.virtual_range.start.checked_add(start as u64)?, end: request.span.virtual_range.start.checked_add(end as u64)? },
            origins: vec![WitSourceOrigin { kind: WitOriginKind::Exact, expansion: None,
                original_range: WitTextRange { start: base.original_range.start.checked_add(start as u64)?, end: base.original_range.start.checked_add(end as u64)? } }] })
    };
    for node in &mut tree.nodes {
        if !source_geometry::map_native_projection(node, &map) { return None; }
    }
    Some(tree)
}

fn same_input(left: &Input, right: &Input) -> bool {
    left.id == right.id && left.start == right.start && left.end == right.end
        && left.owner == right.owner && left.raw_owner == right.raw_owner
        && left.line_input == right.line_input && Arc::ptr_eq(&left.subject, &right.subject)
        && match (&left.anchor, &right.anchor) { (None, None) => true, (Some(a), Some(b)) => Arc::ptr_eq(a,b), _ => false }
}

fn no_effects(effects: &HookEffects) -> bool {
    effects.diagnostics.is_empty() && effects.context_updates.is_empty()
        && effects.parse_requests.is_empty() && effects.parse_results.is_empty()
}

impl ParserHost {
    /// A bounded, editing-only refinement after the frontend's current flow pass.
    /// No old parsing scope/state is replayed. Facts never change the semantic AST.
    pub fn refine_editor_completions(&mut self, result: &DocumentParseResult,
        facts: &[TypeFact], cancellation: Option<DocumentCancellationToken>) -> Result<Vec<Batch>, HostError> {
        if self.editor_context.is_some() || self.editor_capture.is_some() || self.editor_session.is_some() {
            return Err(HostError::InvalidParseResult {parser_id:ROUTE_PREFIX.into(),message:"typed refinement requires a completed parse".into()});
        }
        let mut output=Vec::new();let mut bytes=0usize;
        for batch in &result.editor_completions {
            if cancellation.as_ref().is_some_and(DocumentCancellationToken::is_cancelled) {return Err(skript_parser::ParseCancelled.into());}
            if batch.inputs.is_empty() {continue;}
            let mut selected=Vec::new();let mut allowed=Vec::new();
            for input in &batch.inputs {
                if !input.is_current() {continue;}
                let fence=&input.subject.fence;
                if !self.editor_documents.get(&(fence.project.clone(),fence.document.clone()))
                    .is_some_and(|current|Arc::ptr_eq(current,fence)) {continue;}
                let witnesses=input.subject_witnesses(&result.source);
                if !input.subject.tree.nodes.iter().find(|n|input.subject.tree.roots.contains(&n.id))
                    .is_some_and(|n|witnesses.native_valid(n)) {continue;}
                let matching=facts.iter().filter(|fact|same_input(&fact.input,input)).collect::<Vec<_>>();
                let [fact]=matching.as_slice() else{continue;};
                if fact.complete==fact.inferred || fact.inferred&&fact.read_closed || fact.multiple || fact.classes.is_empty() || fact.classes.len()>16
                    || fact.classes.iter().any(|class|class.is_empty()||class.len()>1024||class=="java.lang.Object"||class.chars().any(char::is_control))
                    || fact.classes.iter().collect::<BTreeSet<_>>().len()!=fact.classes.len() {continue;}
                selected.push(serde_json::json!({"id":input.id,"start":input.start,"end":input.end,
                    "storedTypes":fact.classes,"complete":fact.complete,"multiple":fact.multiple,"maybeUnset":fact.maybe_unset,
                    "readClosed":fact.read_closed,"certainty":if fact.inferred {"inferred"}else{"current"},
                    "evidence":if fact.inferred {"same-flow-prior-candidate"}else{"current-native-semantic-flow"},"boundary":"parsed-prefix-first-operand",
                    "semanticGuarantee":false}));
                allowed.push((input.start,input.end));
            }
            if selected.is_empty(){continue;}
            let fence=&batch.inputs[0].subject.fence;
            let json=serde_json::json!({"version":2,"documentRevision":fence.revision,"inputs":selected}).to_string();
            if json.len()>128*1024 {continue;}
            let route=format!("{ROUTE_PREFIX}{}",batch.owner);
            let index=self.components.iter().position(|component|component.manifest.component_id==batch.owner)
                .ok_or_else(||HostError::UnavailableComponent {component_id:batch.owner.clone(),reason:"editing provider is no longer loaded".into()})?;
            self.ensure_component_ready(index)?;
            let component=&self.components[index];
            if component.disabled || component.unloaded {return Err(HostError::UnavailableComponent {component_id:batch.owner.clone(),reason:"editing provider is unavailable".into()});}
            let subscriptions=component.manifest.subscriptions.iter().filter(|subscription|
                matches!(&subscription.target,HookTarget::Parser(id)if id==&route)).collect::<Vec<_>>();
            let [subscription]=subscriptions.as_slice() else{return Err(HostError::InvalidManifest {message:"typed editing requires one owner input-only refinement route".into()});};
            let subscription_id=subscription.id.clone();
            let request=ParseRequest {request_id:0,parser_id:route.clone(),input:batch.input.clone(),span:batch.span.clone(),expected_types:Vec::new(),
                options:vec![WitMetadataEntry {key:FACTS.into(),value:json,owner_component_id:None}]};
            let context=InvocationContext {invocation_id:self.next_parse_result_token,subscription_id,document_id:fence.document.clone(),document_revision:fence.revision,
                expansion:None,syntax_context:0};
            self.next_parse_result_token=self.next_parse_result_token.checked_add(1).ok_or(HostError::ParseResultTokenExhausted)?;
            let invocation=crate::bindings::nlaocs::skript_parser_addon::types::HookInvocation {context,target:HookTarget::Parser(route.clone()),phase:HookPhase::Parser,
                payload:HookPayload::Parser(request.clone()),parse_results:Vec::new()};
            let deadline=self.config.deadline_ticks(&batch.owner);
            let entry=&mut self.components[index];
            if entry.store.data().invocation.is_some() || entry.store.data().dynamic_syntax_update.is_some() || entry.store.data().tree_view.is_some() {
                return Err(HostError::InvalidHookOutput {component_id:batch.owner.clone(),subscription_id:"input-only".into(),message:"a parsing resource is still active".into()});
            }
            let old_readonly=entry.store.data().editor_readonly;
            let old_input_only=entry.store.data().editor_input_only;
            let old_attempted=entry.store.data().editor_mutation_attempted;
            let old_cancellation=mem::replace(&mut entry.store.data_mut().cancellation,cancellation.clone());
            entry.store.data_mut().editor_readonly=true;entry.store.data_mut().editor_input_only=true;entry.store.data_mut().editor_mutation_attempted=false;
            let prepared=prepare_store(&mut entry.store,self.config.fuel_per_call,deadline,&batch.owner,"typed editing");
            let called=prepared.and_then(|()|entry.bindings.nlaocs_skript_parser_addon_hooks().call_invoke(&mut entry.store,&invocation)
                .map_err(|error|classify_wasmtime_error(batch.owner.clone(),"typed editing",error)))
                .and_then(|result|result.map_err(|error|HostError::AddonFailure {component_id:batch.owner.clone(),kind:error.kind,message:error.message}));
            let forbidden=entry.store.data().editor_mutation_attempted;
            entry.store.data_mut().editor_readonly=old_readonly;entry.store.data_mut().editor_input_only=old_input_only;
            entry.store.data_mut().editor_mutation_attempted=old_attempted;entry.store.data_mut().cancellation=old_cancellation;
            let mut called=match called {
                Ok(output)=>output,
                Err(error)=>{if error.disables_component() {self.disable_component(index,&error);}return Err(error);}
            };
            if cancellation.as_ref().is_some_and(DocumentCancellationToken::is_cancelled) {return Err(skript_parser::ParseCancelled.into());}
            if hook_output_size(&called)>self.config.max_generated_output_bytes {return Err(HostError::GeneratedOutputQuotaExceeded {limit:self.config.max_generated_output_bytes});}
            let invalid=|message:&str|HostError::InvalidHookOutput {component_id:batch.owner.clone(),subscription_id:route.clone(),message:message.into()};
            if forbidden {return Err(invalid("input-only editing attempted a state/scope import"));}
            if matches!(called.decision,HookDecision::NotApplicable) && called.replacement.is_none() && no_effects(&called.effects){continue;}
            if !matches!(called.decision,HookDecision::Handled) || called.replacement.is_some()
                || !called.effects.diagnostics.is_empty() || !called.effects.context_updates.is_empty() || !called.effects.parse_requests.is_empty()
                || called.effects.parse_results.len()!=1 {return Err(invalid("input-only editing may return only one proposal result"));}
            let parsed=called.effects.parse_results.pop().expect("one result");
            validate_parse_result(&request,&parsed)?;
            let [root]=parsed.roots.as_slice()else{return Err(invalid("one proposal root is required"));};
            let [node]=parsed.nodes.as_slice()else{return Err(invalid("proposal result must not contain a semantic graph"));};
            if node.node_id!=*root||node.kind!="editor-query-result"||node.summary.is_some()||node.function.is_some()||!node.children.is_empty()
                ||!node.attachments.is_empty()||!node.diagnostics.is_empty()||!parsed.diagnostics.is_empty(){return Err(invalid("typed editing returned semantic data"));}
            let entries=node.metadata.iter().filter(|metadata|metadata.key==RESULT).collect::<Vec<_>>();
            let [metadata]=entries.as_slice()else{return Err(invalid("one bounded proposal record is required"));};
            if metadata.value.len()>128*1024 {return Err(invalid("typed proposal record exceeds its limit"));}
            let mut proposed:Output=serde_json::from_str(&metadata.value).map_err(|_|invalid("invalid typed proposal JSON"))?;
            if !valid_output(&proposed,&batch.input)||!proposed.typed_inputs.is_empty()
                ||proposed.proposals.iter().any(|p|!allowed.contains(&(p.start,p.end))){return Err(invalid("typed proposals escaped their selected input ranges"));}
            let inferred=selected.iter().any(|v|v["certainty"]=="inferred");
            stamp_uncertainty(&mut proposed,inferred);
            bytes=bytes.saturating_add(metadata.value.len());if bytes>MAX_BYTES{break;}
            output.push(Batch {owner:batch.owner.clone(),attempt:batch.attempt,input:batch.input.clone(),span:batch.span.clone(),output:proposed,inputs:Vec::new()});
        }
        Ok(output)
    }
}

fn stamp_uncertainty(output:&mut Output,inferred:bool) {
    output.inferred=inferred;
    if inferred {output.complete=false;output.exclusive=false;}
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_provider_cannot_erase_inference_or_make_it_an_exclusive_complete_menu() {
        let mut output:super::Output=serde_json::from_str(r#"{"version":1,"complete":true,"exclusive":true,"proposals":[]}"#).unwrap();
        super::stamp_uncertainty(&mut output,true);
        assert!(output.inferred&&!output.complete&&!output.exclusive);
        assert!(super::valid_output(&output,""));
    }
    use super::*;
    #[test]
    fn drafts_cannot_supply_foreign_or_unobserved_native_subjects() {
        let output=Output {version:1,complete:false,exclusive:true,inferred:false,proposals:Vec::new(),typed_inputs:vec![Draft {id:"one".into(),
            subject:Reference {host_token:5,root_id:0},anchor:None,start:5,end:8}]};
        let span=MappedSpan {virtual_range:WitTextRange {start:0,end:8},origins:Vec::new()};
        assert!(select_inputs(&output,None,"a",1,&span,"{_a}.sub").is_none());
        assert!(select_inputs(&output,Some(&Capture::new()),"a",1,&span,"{_a}.sub").is_none());
        let mut old=serde_json::json!({"version":1,"complete":false,"exclusive":true,"proposals":[]});
        assert!(serde_json::from_value::<Output>(old.clone()).unwrap().typed_inputs.is_empty());
        old["typedInputs"]=serde_json::json!([{"id":"one","subject":{"hostToken":5,"rootId":0,"type":"spoof"},"start":5,"end":8}]);
        assert!(serde_json::from_value::<Output>(old).is_err());
    }
}

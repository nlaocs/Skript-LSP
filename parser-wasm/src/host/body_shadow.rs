//! Bounded test-only decision gate. Never imports, adopts, replays or stores a body.
//! Only data/weak identities survive an event; opaque resources stay live
//! exclusively in the ordinary parser. Every temporary certificate is dropped.
use super::*;
use sha2::{Digest, Sha256};
use serde_json::{Value, json};
use skript_parser::{StructureBodyReuseRequest, StructureBodyWork, StructurePreparationEvent,
    PortableStructureBody, ConsumedBodyResource, RawNodeId, SectionBodyNode};

const MAX_ROWS: usize = 8;
const MAX_OWNERS: usize = 16;
const MAX_CALLS: usize = 4096;
const MAX_IDENTITIES: usize = 64;
const MAX_BYTES: usize = 256 * 1024;
const RECEIPT: &str = "parser.semantic-writer-receipt.v1";
const OVERFLOW: &str = "budget-exceeded";

fn digest(value: &impl std::fmt::Debug) -> String {
    use std::fmt::Write;
    struct BoundedHash {hash:Sha256,size:Size}
    impl std::fmt::Write for BoundedHash {
        fn write_str(&mut self,value:&str)->std::fmt::Result {self.size.write_str(value)?;self.hash.update(value);Ok(())}
    }
    let mut sink=BoundedHash {hash:Sha256::new(),size:Size(0)};
    if write!(&mut sink,"{value:?}").is_err() {return OVERFLOW.into();}
    format!("{:x}",sink.hash.finalize())
}
fn check(status: &str, reason: &str) -> Value { json!({"status":status,"reason":reason}) }

fn checked_calls(before:&HookCallLog, current:&HookCallLog) -> Option<Vec<HookCall>> {
    before.is_prefix_of(current).then(||current.suffix(before.len()))
}

#[derive(Clone)]
pub(super) struct Probe(Arc<Mutex<Data>>);
struct Data {
    enabled: bool,
    starts: BTreeSet<usize>,
    live: Option<Live>,
    rows: Vec<Value>,
    sequence: Sha256,
    calls: u64,
    overflow: bool,
    next: u64,
    previous: BTreeMap<usize,PreviousReads>,
    finished_resources: Vec<skript_parser::ExpressionEffectsIdentity>,
}
struct PreviousReads {document:String,revision:u64,key:String,world:String,certificate:crate::state::BodyReadCertificate}
struct Live {
    id: u64,
    start: usize,
    revision: u64,
    key: String,
    root: RawNodeId,
    parent: u64,
    owners: Vec<Owner>,
    receipts: Vec<(String,String)>,
    owner_ok: bool,
    preparation_freeze_ok: bool,
    preparation_resources: usize,
    invalidations: usize,
    calls: usize,
    unlisted: usize,
    guest_impure: usize,
    identities: BTreeMap<String,usize>,
    overflow: bool,
    slot: usize,
    document: String,
    world: String,
    prior_reads: Value,
    resources: Vec<ResourceWitness>,
}
struct ResourceWitness {owner:RawNodeId,kind:ConsumedBodyResource,identity:skript_parser::ExpressionEffectsIdentity}
struct Owner {
    raw: RawNodeId,
    attempt: u64,
    parent: Option<u64>,
    identity: String,
}
pub(super) struct Lease { probe: Probe, id: u64 }
impl Drop for Lease {
    fn drop(&mut self) {
        let mut data=self.probe.0.lock().unwrap();
        if data.live.as_ref().is_some_and(|live|live.id==self.id) {
            let live=data.live.take().unwrap();
            data.rows.push(json!({"start":live.start,"revision":live.revision,"partial":true,
                "status":"Blocked","reason":"body ended before live finish (cancel/rejection/error)",
                "owners":live.owners.len(),"calls":live.calls,"resourcesRetained":false,"freshHeaderReadCheck":live.prior_reads}));
            for witness in live.resources {
                if data.finished_resources.len()>=MAX_ROWS*64 {data.overflow=true;break;}
                data.finished_resources.push(witness.identity);
            }
        }
    }
}
impl Probe {
    pub(super) fn new(enabled: bool, starts: impl IntoIterator<Item=usize>) -> Self {
        let starts=starts.into_iter().collect::<BTreeSet<_>>();
        assert!(starts.len()<=2,"dedicated two-body probe only");
        Self(Arc::new(Mutex::new(Data { enabled, starts,live:None,rows:Vec::new(),
            sequence:Sha256::new(),calls:0,overflow:false,next:0,previous:BTreeMap::new(),finished_resources:Vec::new() })))
    }
    pub(super) fn begin(&self, request:&StructureBodyReuseRequest<'_>, key:&str, revision:u64,
        transaction:&ParseTransaction, world:&str) -> Option<Lease> {
        let mut data=self.0.lock().unwrap();
        if !data.enabled {return None;}
        let root=request.tree.get(request.candidate.raw_node_id)?;
        let extent=super::incremental::body::shadow_original_range(request.source,root.span.virtual_range)?;
        if !data.starts.contains(&extent.start) {return None;}
        if data.live.is_some() || data.rows.len()>=MAX_ROWS {data.overflow=true;return None;}
        data.next+=1;let id=data.next;
        let slot=data.starts.iter().position(|start|*start==extent.start)?;
        let document=transaction.document_id().ok()?;
        let world=format!("{:x}",Sha256::digest(world.as_bytes()));
        let prior_reads=match data.previous.get(&slot) {
            None=>check("NotRequired","no prior readonly sample"),
            Some(previous) if previous.document!=document || previous.revision>=revision
                || previous.key!=key || previous.world!=world=>check("Blocked","document/revision/key/world changed; ordinary parse required"),
            Some(previous)=>transaction.body_shadow_check_reads(&previous.certificate)
                .unwrap_or_else(|_|check("Blocked","current readonly predicate unavailable")),
        };
        data.live=Some(Live {id,start:extent.start,revision,key:key.into(),root:root.id,parent:request.parent_attempt,
            owners:Vec::new(),receipts:Vec::new(),owner_ok:true,preparation_freeze_ok:true,preparation_resources:0,invalidations:0,
            calls:0,unlisted:0,guest_impure:0,identities:BTreeMap::new(),overflow:false,
            slot,document,world,prior_reads,resources:Vec::new()});
        Some(Lease {probe:self.clone(),id})
    }
    pub(super) fn invocation(&self, component:&str, invocation:&crate::bindings::nlaocs::skript_parser_addon::types::HookInvocation,
        executed:bool, listed:bool, outcome:&str, guest_impure:bool) {
        let identity=match &invocation.payload {
            HookPayload::RegisteredExpression(p)=>digest(&(&p.definition_id,&p.registration_id)),
            _=>digest(&invocation.target),
        };
        let mut data=self.0.lock().unwrap();
        // Includes negative/losing attempts. No payload/source/State values.
        let call_digest=digest(&(invocation.phase,component,&invocation.context.subscription_id,&identity,executed,outcome));
        data.overflow |= identity==OVERFLOW || call_digest==OVERFLOW;
        data.sequence.update(&call_digest);
        data.calls+=u64::from(executed);
        let Some(live)=data.live.as_mut() else {return;};
        live.calls+=1;live.unlisted+=usize::from(!listed);live.guest_impure+=usize::from(guest_impure);
        if live.calls>MAX_CALLS {live.overflow=true;return;}
        let subscription=digest(&(component,&invocation.context.subscription_id));
        live.overflow |= identity==OVERFLOW || call_digest==OVERFLOW || subscription==OVERFLOW;
        let key=format!("{subscription}|{identity}|{listed}|{outcome}");
        if !live.identities.contains_key(&key) && live.identities.len()>=MAX_IDENTITIES {live.overflow=true;return;}
        *live.identities.entry(key).or_default()+=1;
    }
    pub(super) fn snapshot(&self) -> Value {
        let data=self.0.lock().unwrap();
        json!({"rows":data.rows,"physicalCalls":data.calls,"physicalAndNegativeOrderSha256":format!("{:x}",data.sequence.clone().finalize()),
            "live":data.live.is_some(),"overflow":data.overflow,
            "limits":{"rows":MAX_ROWS,"ownersPerBody":MAX_OWNERS,"callsPerBody":MAX_CALLS,
                "identitiesPerBody":MAX_IDENTITIES,"temporaryBodyBytes":MAX_BYTES},"retainsOpaqueResources":false,
            "finishedResourceWitnesses":data.finished_resources.len(),
            "finishedResourcesStillLive":data.finished_resources.iter().filter(|id|id.is_live()).count()})
    }
    pub(super) fn reset(&self) {
        let mut data=self.0.lock().unwrap();assert!(data.live.is_none());
        data.rows.clear();data.sequence=Sha256::new();data.calls=0;data.overflow=false;
    }
    pub(super) fn retarget(&self, starts:impl IntoIterator<Item=usize>) {
        let mut data=self.0.lock().unwrap();assert!(data.live.is_none());
        let starts=starts.into_iter().collect::<BTreeSet<_>>();assert!(starts.len()<=2);data.starts=starts;
    }
}

// Same consumed-resource predicates as production body_finish; no resource is
// moved, restored, applied or retained here. The probe is not reuse authority.
fn consumed(hooks:&WasmPatternHooks<'_>, before:&HookEffectsCheckpoint,
    kind:ConsumedBodyResource, value:&skript_parser::ExpressionEffects) -> bool {
    let Some(calls)=checked_calls(&before.calls,&hooks.calls) else {return false;};
    let Some(updates)=hooks.effects.context_updates.get(before.effects.context_updates.len()..) else {return false;};
    match kind {
        ConsumedBodyResource::Expression=>value.downcast_ref::<DeferredExpressionEffects>().is_some_and(|e|
            hooks.transaction.delta_is_currently_applied(&e.state).unwrap_or(false)
                && e.failures.is_empty() && e.effects.diagnostics.is_empty()
                && e.effects.parse_requests.is_empty() && e.effects.parse_results.is_empty()
                && (e.calls.is_empty() || calls.windows(e.calls.len()).any(|part|part==e.calls.as_slice()))
                && (e.effects.context_updates.is_empty() || updates.windows(e.effects.context_updates.len()).any(|part|
                    part.iter().zip(&e.effects.context_updates).all(|(a,b)|a.syntax_context==b.syntax_context && a.key==b.key && a.value==b.value)))),
        ConsumedBodyResource::Pattern=>value.downcast_ref::<SavedPatternCandidate>().is_some_and(|e|
            hooks.transaction.owns_savepoint(&e.state).unwrap_or(false)
                && e.restored.as_ref().is_some_and(|flag|flag.load(Ordering::Relaxed))
                && e.effects.calls.is_prefix_of(&hooks.calls)),
    }
}

/// Testable shared enter-freeze path; the returned summary has no resources.
pub(super) fn freeze_enter(candidate:&skript_parser::SectionCandidate,
    mut consumed:impl FnMut(ConsumedBodyResource,&skript_parser::ExpressionEffects)->bool) -> (bool,bool,usize,Vec<(String,String)>) {
    use std::fmt::Write;
    let mut size=Size(0);
    if write!(&mut size,"{candidate:?}").is_err() {return (false,true,0,Vec::new());}
    let mut resources=0;
    let frozen=PortableStructureBody::freeze_preparation(candidate,&mut |kind,value|{resources+=1;consumed(kind,value)});
    let success=frozen.is_some();
    let metadata=frozen.map(|(_,metadata)|metadata.into_iter().filter(|(key,_)|key==RECEIPT).collect()).unwrap_or_default();
    (success,false,resources,metadata)
}

pub(super) fn preparation(probe:&Probe, hooks:&WasmPatternHooks<'_>, before:&HookEffectsCheckpoint,
    event:&StructurePreparationEvent<'_>) {
    let mut data=probe.0.lock().unwrap();
    let Some(live)=data.live.as_mut() else {return;};
    match event {
        StructurePreparationEvent::SectionHeader {source,tree,candidate,attempt_id,parent_attempt_id}=>{
            if live.owners.len()>=MAX_OWNERS {live.overflow=true;return;}
            let mut ancestor=tree.get(candidate.raw_node_id).and_then(|n|n.parent);
            let mut expected=None;let mut steps=0;
            while let Some(id)=ancestor {
                steps+=1;if steps>4096 {live.overflow=true;break;}
                if id==live.root {expected=Some(live.parent);break;}
                if let Some(owner)=live.owners.iter().rev().find(|owner|owner.raw==id) {expected=Some(owner.attempt);break;}
                ancestor=tree.get(id).and_then(|n|n.parent);
            }
            live.owner_ok &= expected==*parent_attempt_id && expected.is_some()
                && !live.owners.iter().any(|owner|owner.raw==candidate.raw_node_id || owner.attempt==*attempt_id)
                && super::incremental::body::shadow_original_range(source,candidate.matched.matched.span.mapped.virtual_range).is_some();
            // Freeze a temporary clone now, then drop it before returning the
            // real enter event. Retain only host receipt references and hashes.
            let (frozen,overflow,resources,metadata)=freeze_enter(candidate,|kind,value| {
                if live.resources.len()>=64 {live.overflow=true;return false;}
                live.resources.push(ResourceWitness {owner:candidate.raw_node_id,kind,identity:value.identity()});
                consumed(hooks,before,kind,value)
            });
            live.preparation_freeze_ok &= frozen;live.overflow |= overflow;
            live.preparation_resources+=resources;
            for (key,value) in metadata {
                if live.receipts.len()>=256 {live.overflow=true;break;}
                live.receipts.push((key,value));
            }
            let identity=digest(&(&candidate.matched.definition_id,&candidate.matched.registration_id,candidate.matched.pattern_index,candidate.body_mode));
            live.overflow |= identity==OVERFLOW;
            live.owners.push(Owner {raw:candidate.raw_node_id,attempt:*attempt_id,parent:*parent_attempt_id,identity});
        },
        StructurePreparationEvent::Invalidated {..}=>live.invalidations+=1,
        _=>{},
    }
}

fn owners_match(body:&skript_parser::StructureBody, owners:&[Owner]) -> bool {
    let skript_parser::StructureBody::Trigger(nodes)=body else {return false;};
    let mut pending=nodes.iter().rev().collect::<Vec<_>>();let mut index=0;
    while let Some(node)=pending.pop() {
        if let SectionBodyNode::Section(matches)=node {
            let Some(candidate)=matches.selected.as_ref() else {return false;};
            let Some(owner)=owners.get(index) else {return false;};index+=1;
            if owner.raw!=candidate.raw_node_id || owner.identity!=digest(&(&candidate.matched.definition_id,
                &candidate.matched.registration_id,candidate.matched.pattern_index,candidate.body_mode)) {return false;}
            pending.extend(candidate.body.iter().rev());
        }
    }
    index==owners.len()
}

fn cover_current_resource(witnesses:&[ResourceWitness], covered:&mut [bool], owner:RawNodeId,
    kind:ConsumedBodyResource, value:&skript_parser::ExpressionEffects) -> bool {
    if let Some((index,_))=witnesses.iter().enumerate().find(|(index,witness)|
        !covered[*index] && witness.owner==owner && witness.kind==kind && witness.identity.matches(value)) {
        covered[index]=true;true
    } else {false}
}

struct Size(usize);
impl std::fmt::Write for Size {
    fn write_str(&mut self,value:&str)->std::fmt::Result {
        self.0=self.0.saturating_add(value.len());
        if self.0>MAX_BYTES {Err(std::fmt::Error)} else {Ok(())}
    }
}

pub(super) fn finish(lease:&Lease, hooks:&WasmPatternHooks<'_>, body:&skript_parser::StructureBody,
    context:&ExpressionParseContext, work:StructureBodyWork, before_work:StructureBodyWork,
    state:&crate::state::BodyObservationCheckpoint, before:&HookEffectsCheckpoint, pending:bool) {
    use std::fmt::Write;
    let Some(live)=lease.probe.0.lock().unwrap().live.take() else {return;};
    let mut size=Size(0);let bounded=write!(&mut size,"{body:?}|{context:?}").is_ok() && !live.overflow;
    let reads=hooks.transaction.body_read_certificate(state);
    let state_observation=hooks.transaction.body_shadow_status(state);
    let a4=match &reads {
        Ok(Some(_))=>check("Pass","current readonly certificate captured; not replayed"),
        Ok(None)=>check("Blocked","readonly/observation/prefix requirements not certified; see state observation"),
        Err(_)=>check("Blocked","current transaction certificate unavailable"),
    };
    let mut resource_checks=0;let mut resource_refusals=0;
    let prefix=before.calls.is_prefix_of(&hooks.calls);
    let portable=(bounded && prefix).then(||PortableStructureBody::freeze(body,&mut |kind,value| {
        resource_checks+=1;let ok=consumed(hooks,before,kind,value);resource_refusals+=usize::from(!ok);ok
    })).flatten();
    // Revisit only the adopted owner's current HEADER resources, not a same-
    // looking resource elsewhere in its body. Weak tokens grant no authority:
    // every matching current borrowed value must pass the original predicate.
    let mut covered=vec![false;live.resources.len()];let mut current_headers_ok=true;
    if bounded && prefix && portable.is_some() {
        let mut pending=match body {skript_parser::StructureBody::Trigger(nodes)=>nodes.iter().rev().collect::<Vec<_>>(),_=>Vec::new()};
        while let Some(node)=pending.pop() {
            if let SectionBodyNode::Section(matches)=node && let Some(candidate)=&matches.selected {
                pending.extend(candidate.body.iter().rev());
                let mut header=candidate.clone();header.body.clear();
                let (ok,overflow,_,_)=freeze_enter(&header,|kind,value| {
                    let current=consumed(hooks,before,kind,value);
                    if current {cover_current_resource(&live.resources,&mut covered,candidate.raw_node_id,kind,value);}
                    current
                });
                current_headers_ok &= ok && !overflow;
            }
        }
    }
    let a5=if !bounded {check("Blocked","temporary body/event budget exceeded")}
        else if !prefix {check("Blocked","current call prefix changed; consumed-resource checks not run")}
        else if portable.is_none() {check("Blocked","current native body/resource freeze refused")}
        else if !live.preparation_freeze_ok {check("Blocked","enter preparation resource freeze refused")}
        else if !current_headers_ok || covered.iter().any(|covered|!*covered) {check("Blocked","enter resource not certified in the same current adopted owner header")}
        else {check("Pass","same non-owning enter identities covered and revalidated in current owner headers; no future replay implied")};
    let calls=if bounded {checked_calls(&before.calls,&hooks.calls).unwrap_or_default()} else {Vec::new()};
    let mut receipt_report=json!({"status":"Blocked","reason":"body freeze unavailable"});
    if let (Some(portable),Some(registry))=(&portable,&hooks.host.semantic_receipts) {
        let body_only=registry.body_shadow_export(&calls,portable.metadata());
        let mut metadata=portable.metadata().to_vec();metadata.extend(live.receipts.iter().cloned());
        let with_preparation=registry.body_shadow_export(&calls,&metadata);
        metadata.extend(context.values.iter().map(|(k,v)|(k.clone(),v.clone())));
        for frame in &context.section_stack {
            metadata.extend(frame.metadata.iter().map(|(k,v)|(k.clone(),v.clone())));
            for capture in &frame.captures {metadata.extend(capture.metadata.iter().map(|(k,v)|(k.clone(),v.clone())));}
        }
        let full=registry.body_shadow_export(&calls,&metadata);
        receipt_report=json!({"status":if full["exported"]==true && live.preparation_freeze_ok {"Pass"} else {"Blocked"},
            "bodyOnly":body_only,"withPreparation":with_preparation,"withPostContext":full,
            "privateProviderScopePortability":"NotExercised"});
    }
    let effects_unchanged=hooks.failures.len()==before.failures_len
        && hooks.effects.diagnostics.len()==before.effects.diagnostics.len()
        && hooks.effects.parse_requests.len()==before.effects.parse_requests.len()
        && hooks.effects.parse_results.len()==before.effects.parse_results.len();
    let row=json!({"start":live.start,"revision":live.revision,"partial":false,"keySha256":live.key,
        "A1":{"status":"Pass","reason":"production key admitted current exact input/context/world/work","crossRevisionKey":"compare recorded keys; never normalize opaque IDs"},
        "A2":{"status":if live.unlisted==0 && live.guest_impure==0 && !live.overflow {"Pass"} else {"Blocked"},
            "uncertifiedInvocations":live.unlisted,"guestImpureFlags":live.guest_impure,"attemptedInvocations":live.calls,
            "identityOutcomeCounts":live.identities,"uncertifiedIsMutationEvidence":false},
        "A3":{"status":if !bounded {"Blocked"} else if live.owner_ok && live.invalidations==0 && owners_match(body,&live.owners) {"Pass"} else {"Fail"},
            "owners":live.owners.len(),"invalidations":live.invalidations,"parentChecks":live.owners.iter().map(|o|(o.attempt,o.parent)).collect::<Vec<_>>(),"replay":"NotExercised"},
        "A4":a4,"freshHeaderReadCheck":live.prior_reads,"stateObservation":state_observation.ok(),"readonlyCertificateBytes":reads.as_ref().ok().and_then(|c|c.as_ref()).map(|c|c.retained_bytes()),
        "A5":a5,"resourceChecks":resource_checks,"resourceRefusals":resource_refusals,"enterResourceChecks":live.preparation_resources,
        "enterResourcesCovered":covered.iter().filter(|covered|**covered).count(),
        "enterResourcesExpiredAtBodyEnd":live.resources.iter().filter(|resource|!resource.identity.is_live()).count(),
        "A6":receipt_report,
        "A7":{"current":{"status":if bounded && prefix && effects_unchanged && !pending {"Pass"} else {"Blocked"},
            "callPrefix":prefix,"noFailureDiagnosticOrAdditionalParse":effects_unchanged,"noPendingResources":!pending,
            "beforeWork":format!("{before_work:?}"),"afterWork":format!("{work:?}"),"postContextSha256":digest(context)},
            "import":"NotExercised","readReplay":"NotExercised","adoption":"NotExercised","futureResourceLifetimes":"NotExercised"},
        "temporaryBodyBytes":size.0,"overflow":live.overflow,"resourcesRetained":false});
    drop(portable);
    let mut data=lease.probe.0.lock().unwrap();
    if let Ok(Some(certificate))=reads {
        // Data-only diagnostic sample, not an adopted body or cache authority.
        data.previous.insert(live.slot,PreviousReads {document:live.document,revision:live.revision,key:live.key,world:live.world,certificate});
    }
    for witness in live.resources {
        if data.finished_resources.len()>=MAX_ROWS*64 {data.overflow=true;break;}
        data.finished_resources.push(witness.identity);
    }
    data.rows.push(row);
}

#[test]
fn shadow_bounds_drop_partial_lease_and_never_retain_opaque_resources() {
    let probe=Probe::new(true,[0]);
    let mut data=probe.0.lock().unwrap();
    data.live=Some(Live {id:1,start:0,revision:1,key:String::new(),root:RawNodeId::new(0),parent:1,
        owners:Vec::new(),receipts:Vec::new(),owner_ok:true,preparation_freeze_ok:true,preparation_resources:0,invalidations:0,
        calls:0,unlisted:0,guest_impure:0,identities:BTreeMap::new(),overflow:false,
        slot:0,document:String::new(),world:String::new(),prior_reads:Value::Null,resources:Vec::new()});
    drop(data);
    let drops=Arc::new(std::sync::atomic::AtomicUsize::new(0));
    struct Opaque(Arc<std::sync::atomic::AtomicUsize>);
    impl Drop for Opaque {fn drop(&mut self){self.0.fetch_add(1,Ordering::Relaxed);}}
    let opaque=skript_parser::ExpressionEffects::new(Opaque(drops.clone()));
    let lease=Lease {probe:probe.clone(),id:1};drop(opaque);
    assert_eq!(drops.load(Ordering::Relaxed),1,"probe/lease cannot keep resources alive");
    drop(lease);let snapshot=probe.snapshot();
    assert_eq!(snapshot["live"],false);assert_eq!(snapshot["rows"][0]["partial"],true);
    let mut size=Size(0);use std::fmt::Write;
    assert!(write!(&mut size,"{}","a".repeat(MAX_BYTES+1)).is_err());
    assert_eq!(digest(&"a".repeat(MAX_BYTES+1)),OVERFLOW);
}

#[test]
fn shadow_prefix_refusal_does_not_slice_shorter_or_equal_unrelated_logs() {
    let call=HookCall {origin:HookCallOrigin::Executed,component_id:"public.fixture".into(),subscription_id:"fixture".into(),semantic_receipt:None};
    let mut prefix=HookCallLog::default();prefix.extend(vec![call.clone()]);
    assert!(checked_calls(&prefix,&HookCallLog::default()).is_none());
    let mut unrelated=HookCallLog::default();unrelated.extend(vec![call.clone()]);
    assert!(checked_calls(&prefix,&unrelated).is_none());
    let mut current=prefix.clone();current.extend(vec![call]);
    assert_eq!(checked_calls(&prefix,&current).unwrap().len(),1);
}

#[test]
fn shadow_resource_coverage_is_owner_kind_identity_and_multiplicity_exact() {
    use ConsumedBodyResource::{Expression,Pattern};
    let value=skript_parser::ExpressionEffects::new(7u32);
    let witnesses=vec![ResourceWitness {owner:RawNodeId::new(1),kind:Expression,identity:value.identity()}];
    let mut covered=vec![false];
    assert!(!cover_current_resource(&witnesses,&mut covered,RawNodeId::new(2),Expression,&value));
    assert!(!cover_current_resource(&witnesses,&mut covered,RawNodeId::new(1),Pattern,&value));
    assert!(!cover_current_resource(&witnesses,&mut covered,RawNodeId::new(1),Expression,&skript_parser::ExpressionEffects::new(7u32)));
    assert!(cover_current_resource(&witnesses,&mut covered,RawNodeId::new(1),Expression,&value.clone()));
    assert!(!cover_current_resource(&witnesses,&mut covered,RawNodeId::new(1),Expression,&value));
    drop(value);assert!(!witnesses[0].identity.is_live());covered[0]=false;
    assert!(!cover_current_resource(&witnesses,&mut covered,RawNodeId::new(1),Expression,&skript_parser::ExpressionEffects::new(7u32)));
}

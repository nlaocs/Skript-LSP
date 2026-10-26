//! Test-only first-failing admission counters. A passed funnel stage is not a
//! rejection or proof of successful caching. No input text, clocks or State.
use super::*;

const CODES: &[&str] = &[
    "body_admission.native.total", "body_admission.native.passed",
    "body_admission.native.reject.mode", "body_admission.native.reject.initial_ambiguity",
    "body_admission.native.reject.retry", "body_admission.native.reject.remaining_candidates",
    "body_admission.native.reject.header_captures", "body_admission.native.reject.recovery",
    "body_admission.native.reject.environment", "body_admission.prepare.total",
    "body_admission.prepare.admitted", "body_admission.prepare.reject.document_context",
    "body_admission.prepare.reject.owner_missing", "body_admission.prepare.reject.body_span_missing",
    "body_admission.prepare.reject.source_budget", "body_admission.prepare.reject.mapping_error",
    "body_admission.prepare.reject.multiple_origins", "body_admission.prepare.reject.nonexact_origin",
    "body_admission.prepare.reject.original_range", "body_admission.prepare.reject.virtual_range",
    "body_admission.prepare.reject.receipt_registry_missing", "body_admission.prepare.reject.header_authority",
    "body_admission.prepare.reject.tree_shape_or_budget", "body_admission.prepare.reject.raw_node_missing",
    "body_admission.prepare.reject.active_document_missing", "body_admission.prepare.reject.digest",
    "body_admission.prepare.reject.mapping_range", "body_admission.prepare.reject.mapping_shape",
    "body_admission.prepare.reject.mapping_not_exact", "body_admission.prepare.reject.mapping_not_contiguous",
    "body_admission.prepare.reject.mapping_text_mismatch",
    "body_admission.prepare.observe", "body_admission.prepare.reuse_proposed",
    "body_admission.finish.total", "body_admission.finish.ready",
    "body_admission.finish.reject.invalid", "body_admission.finish.reject.impure",
    "body_admission.finish.reject.failures", "body_admission.finish.reject.diagnostics",
    "body_admission.finish.reject.parse_requests", "body_admission.finish.reject.parse_results",
    "body_admission.finish.reject.pending_leaf", "body_admission.finish.reject.pending_registered",
    "body_admission.finish.reject.expression_scopes", "body_admission.finish.reject.call_prefix",
    "body_admission.finish.reject.call_budget", "body_admission.finish.reject.serialized_budget",
    "body_admission.finish.reject.readonly", "body_admission.finish.reject.portable",
    "body_admission.finish.reject.receipts", "body_admission.finish.reject.retained_budget",
    "body_admission.adopt.total", "body_admission.adopt.stored",
    "body_admission.adopt.reject.not_accepted", "body_admission.adopt.reject.capacity",
    "body_admission.accept.total", "body_admission.accept.replayed",
    "body_admission.accept.reject.registry", "body_admission.accept.reject.receipts",
    "body_admission.accept.reject.reads",
];

#[derive(Clone, Default)]
pub(super) struct Probe(Arc<Mutex<Counters>>, bool);
#[derive(Default)]
struct Counters {
    codes: BTreeMap<&'static str, u64>,
    unknown: u64,
    overflow: bool,
    read_outcomes: BTreeMap<String,u64>,
    read_samples: Vec<serde_json::Value>,
    read_mismatches: u64,
    read_observation_errors: u64,
    read_detail_attempts: u64,
    read_namespace_changes: BTreeMap<String,(u64,u64,u64)>,
    read_finish_outcomes: BTreeMap<String,u64>,
    read_identity_profiled: u64,
    read_identity_blocked: u64,
    read_identity_groups_omitted: u64,
}
impl Probe {
    pub(super) fn with_read_refusals()->Self {Self(Arc::default(),true)}
    pub(super) fn read_refusals_enabled(&self)->bool {self.1}
    pub(super) fn take_read_detail(&self)->bool {
        if !self.1 {return false;}
        let mut data=self.0.lock().unwrap();
        if data.read_detail_attempts>=8 {return false;}
        data.read_detail_attempts+=1;true
    }
    pub(super) fn record_read(&self,key:&str,actual:Option<bool>,report:serde_json::Value) {
        let prefix=report["firstPrefixFailure"].as_str();
        let observation=report["observationFailure"].as_str();
        let predicted=if prefix.is_some()||observation.is_some() {Some(false)} else if report["allObservationsChecked"]==true {Some(true)} else {None};
        let reason=match actual {Some(true)=>"accepted",None=>"actual_error",Some(false)=>
            prefix.or(observation).unwrap_or("observations_uninspected")};
        let mut data=self.0.lock().unwrap();
        if report["observationError"]==true {data.read_observation_errors+=1;}
        else if report["prefixIdentityBudgetBlocked"]==true {data.read_identity_blocked+=1;}
        else {data.read_identity_profiled+=1;}
        if let Some(groups)=report["prefixChangedNamespaces"].as_object() {
            for (key,value) in groups {
                let key=if data.read_namespace_changes.contains_key(key)||data.read_namespace_changes.len()<32 {key.as_str()} else {
                    data.read_identity_groups_omitted+=1;"additional_identity_groups_omitted"
                };
                let counts=data.read_namespace_changes.entry(key.into()).or_default();counts.0+=1;
                counts.1+=value["historicalOnly"].as_u64().unwrap_or(0);counts.2+=value["currentOnly"].as_u64().unwrap_or(0);
            }
        }
        *data.read_outcomes.entry(reason.into()).or_default()+=1;
        if actual.is_some()&&predicted.is_some()&&actual!=predicted {data.read_mismatches+=1;}
        if actual!=Some(true)&&report["detailRequested"]==true&&data.read_samples.len()<8 {
            data.read_samples.push(serde_json::json!({"bodyKeySha256":key,"actual":actual,"profile":report}));
        }
    }
    pub(super) fn record_read_finish(&self,key:&str,report:serde_json::Value) {
        let outcome=if report["blocked"].is_string() {"unobserved_budget".into()} else {
            format!("prefix_equal={}|added_equal={}|namespace_delta_equal={}|old_overlap={}|fresh_overlap={}|observed_namespaces_equal={}",
                report["prefixReadsEqual"],report["addedReadsEqual"],report["addedAdoptedNamespacesEqual"],
                report["oldBodyObservationOverlap"],report["freshBodyObservationOverlap"],report["observedNamespaceIdentitiesEqual"])
        };
        let mut data=self.0.lock().unwrap();*data.read_finish_outcomes.entry(outcome).or_default()+=1;
        if let Some(sample)=data.read_samples.iter_mut().find(|s|s["bodyKeySha256"]==key) {sample["freshColdIdentity"]=report;}
    }
    pub(super) fn record(&self, code: &'static str) {
        let mut data = self.0.lock().unwrap();
        if !CODES.contains(&code) {
            data.overflow |= data.unknown == u64::MAX;
            data.unknown = data.unknown.saturating_add(1);
            return;
        }
        let count = data.codes.entry(code).or_default();
        if let Some(next) = count.checked_add(1) { *count = next; }
        else { data.overflow = true; }
    }
    pub(super) fn reset(&self) { *self.0.lock().unwrap() = Counters::default(); }
    pub(super) fn snapshot(&self) -> serde_json::Value {
        let data = self.0.lock().unwrap();
        let n = |code: &str| data.codes.get(code).copied().unwrap_or(0);
        let sum = |prefix: &str| data.codes.iter().filter(|(code,_)| code.starts_with(prefix))
            .fold(0u64, |sum,(_,value)| sum.saturating_add(*value));
        let total = n("body_admission.native.total");
        let passed = n("body_admission.native.passed");
        let rejected = sum("body_admission.native.reject.");
        let prepare = n("body_admission.prepare.total");
        let prepared = n("body_admission.prepare.admitted");
        let key_rejected = sum("body_admission.prepare.reject.");
        let partition = data.codes.iter().filter(|(code,_)|
            code.starts_with("body_admission.native.reject.") || code.starts_with("body_admission.prepare.reject."))
            .map(|(code,count)| (*code,*count)).collect::<BTreeMap<_,_>>();
        let read_refused=data.read_outcomes.iter().filter(|(key,_)|!matches!(key.as_str(),"accepted"|"actual_error")).map(|(_,n)|n).sum::<u64>();
        let fresh_observed=data.read_finish_outcomes.values().sum::<u64>();
        let phase = |prefix: &str,success: &str| {
            let total=n(&format!("body_admission.{prefix}.total"));
            let accepted=n(success);
            let rejection_prefix=format!("body_admission.{prefix}.reject.");
            let rejected=sum(&rejection_prefix);
            serde_json::json!({"total":total,"accepted":accepted,"rejected":rejected,
                "strictPartition":total==accepted.saturating_add(rejected),
                "reasons":data.codes.iter().filter(|(code,_)|code.starts_with(&rejection_prefix))
                    .map(|(code,count)|(*code,*count)).collect::<BTreeMap<_,_>>()})
        };
        serde_json::json!({"codes":data.codes,"rejectionPartition":partition,
            "readRefusals":self.1.then(||serde_json::json!({"outcomes":data.read_outcomes,"samples":data.read_samples,
                "sampleLimit":8,"predictionMismatches":data.read_mismatches,
                "observationErrors":data.read_observation_errors,
                "detailedChecks":data.read_detail_attempts,
                "prefixChangedNamespaceCounts":data.read_namespace_changes,"freshColdIdentityOutcomes":data.read_finish_outcomes,
                "identityProfiled":data.read_identity_profiled,"identityBudgetBlocked":data.read_identity_blocked,
                "identityGroupLimit":32,"identityGroupOverflowEvents":data.read_identity_groups_omitted,
                "overflowCountsGroupMembershipsNotUniqueRequests":true,
                "readRefused":read_refused,"freshReadonlyObserved":fresh_observed,
                "freshReadonlyUnobserved":read_refused.saturating_sub(fresh_observed),
                "freshReadonlyOvercount":fresh_observed.saturating_sub(read_refused),
                "freshIdentityComparisonBlocked":data.read_finish_outcomes.get("unobserved_budget").copied().unwrap_or(0),
                "detailOmitted":data.read_outcomes.iter().filter(|(k,_)|k.as_str()!="accepted").map(|(_,n)|n).sum::<u64>().saturating_sub(data.read_samples.len() as u64)})),
            "denominator":"physical Structure attempts reaching the gate after accepted EnterBody, across root/nested sessions; not retained AST structures",
            "nativeAttempts":total,"nativeRejected":rejected,"nativePassed":passed,
            "prepareAttempts":prepare,"prepareRejected":key_rejected,
            "prepareAdmitted":prepared,
            "finish":phase("finish","body_admission.finish.ready"),
            "adopt":phase("adopt","body_admission.adopt.stored"),
            "accept":phase("accept","body_admission.accept.replayed"),
            "unfinalizedObservations":prepared.saturating_sub(n("body_admission.finish.total")),
            "nativeReplayRefusals":n("body_admission.prepare.reuse_proposed").saturating_sub(n("body_admission.accept.total")),
            "unreachedPrepare":passed.saturating_sub(prepare),"extraPrepare":prepare.saturating_sub(passed),
            "unknownCodes":data.unknown,"overflow":data.overflow,
            "strictPartition":!data.overflow && data.unknown==0
                && total==rejected.saturating_add(passed) && prepare==key_rejected.saturating_add(prepared)
                && passed==prepare})
    }
}

#[test]
fn read_refusal_samples_are_bounded_and_prediction_disagreements_explicit() {
    let probe=Probe::with_read_refusals();
    for _ in 0..12 {let detail=probe.take_read_detail();probe.record_read("hashed-body",Some(false),serde_json::json!({"firstPrefixFailure":"prefix_reads","detailRequested":detail}));}
    let snapshot=probe.snapshot();let reads=&snapshot["readRefusals"];
    assert_eq!(reads["outcomes"]["prefix_reads"],12);assert_eq!(reads["samples"].as_array().unwrap().len(),8);
    assert_eq!(reads["detailOmitted"],4);assert!(!probe.take_read_detail());assert_eq!(reads["detailedChecks"],8);
    probe.record_read("hashed-body",Some(true),serde_json::json!({"firstPrefixFailure":"prefix_reads"}));
    probe.record_read("hashed-body",None,serde_json::json!({"observationError":true}));
    assert_eq!(probe.snapshot()["readRefusals"]["predictionMismatches"],1);
    assert_eq!(probe.snapshot()["readRefusals"]["observationErrors"],1);
    probe.record_read("hashed-body",Some(true),serde_json::json!({"observationFailure":"stored_bytes_changed","detailBlocked":"non_parse_scope","allObservationsChecked":false}));
    assert_eq!(probe.snapshot()["readRefusals"]["predictionMismatches"],2,"earlier value refusal remains decisive before a later unobserved scope");
    probe.reset();assert!(probe.read_refusals_enabled());assert_eq!(probe.snapshot()["readRefusals"]["samples"],serde_json::json!([]));
    for _ in 0..20 {let detail=probe.take_read_detail();probe.record_read("hashed-body",Some(true),serde_json::json!({"detailRequested":detail,"allObservationsChecked":detail}));}
    assert_eq!(probe.snapshot()["readRefusals"]["detailedChecks"],8,"successful observations consume the same bounded budget");
    assert_eq!(probe.snapshot()["readRefusals"]["outcomes"]["accepted"],20);
    probe.reset();probe.record_read("hashed-body",Some(false),serde_json::json!({"firstPrefixFailure":"prefix_reads","prefixIdentityBudgetBlocked":true}));
    assert_eq!(probe.snapshot()["readRefusals"]["identityBudgetBlocked"],1);
    assert_eq!(probe.snapshot()["readRefusals"]["freshReadonlyUnobserved"],1);
    probe.record_read_finish("hashed-body",serde_json::json!({"blocked":"identity_work_limit"}));
    assert_eq!(probe.snapshot()["readRefusals"]["freshReadonlyUnobserved"],0);
    assert_eq!(probe.snapshot()["readRefusals"]["freshIdentityComparisonBlocked"],1);
    assert!(Probe::default().snapshot()["readRefusals"].is_null());
}

#[test]
fn later_funnel_refusals_are_not_counted_as_another_admission_refusal() {
    let p=Probe::default();
    for code in ["body_admission.native.total","body_admission.native.passed",
        "body_admission.prepare.total","body_admission.prepare.admitted","body_admission.prepare.observe",
        "body_admission.finish.total","body_admission.finish.reject.impure"] {p.record(code);}
    let result=p.snapshot();
    assert_eq!(result["strictPartition"],true);
    assert_eq!(result["rejectionPartition"],serde_json::json!({}));
    assert_eq!(result["finish"]["total"],1);
    assert_eq!(result["finish"]["strictPartition"],true);
    assert_eq!(result["adopt"]["total"],0);
    assert_eq!(result["unfinalizedObservations"],0);
}

#[test]
fn counters_partition_first_failure_without_adding_funnel_passes() {
    assert!(CODES.len() <= 64);
    let p = Probe::default();
    for code in ["body_admission.native.total","body_admission.native.reject.mode",
        "body_admission.native.total","body_admission.native.passed","body_admission.prepare.total",
        "body_admission.prepare.reject.multiple_origins"] { p.record(code); }
    let result=p.snapshot();
    assert_eq!(result["strictPartition"],true);
    assert_eq!(result["nativeAttempts"],2);
    assert_eq!(result["rejectionPartition"].as_object().unwrap().values().map(|v|v.as_u64().unwrap()).sum::<u64>(),2);
    p.record("body_admission.native.passed");
    assert_eq!(p.snapshot()["strictPartition"],false,"unreached/cancelled attempts cannot silently disappear");
    p.reset();p.record("unknown");assert_eq!(p.snapshot()["strictPartition"],false);
    p.reset();p.0.lock().unwrap().codes.insert("body_admission.native.total",u64::MAX);
    p.record("body_admission.native.total");assert_eq!(p.snapshot()["overflow"],true);
}

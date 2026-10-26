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
pub(super) struct Probe(Arc<Mutex<Counters>>);
#[derive(Default)]
struct Counters {
    codes: BTreeMap<&'static str, u64>,
    unknown: u64,
    overflow: bool,
}
impl Probe {
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

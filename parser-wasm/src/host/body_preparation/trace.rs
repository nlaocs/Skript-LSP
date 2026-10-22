//! Optional per-host counters. No shared clock lock, key/source text or event log.
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BodyPhaseMetrics {
    pub started_at_ms: f64,
    pub finished_at_ms: f64,
    pub document_ms: f64,
    pub native_passes: u64,
    /// Outermost Structure native pass, including callbacks and validation.
    pub native_pass_ms: f64,
    /// Generic Wasm hook calls, including lowering and host imports.
    pub hook_call_ms: f64,
    pub hook_calls: u64,
    pub hook_ms_inside_native_pass: f64,
    /// Native pass minus measured hook calls: includes host/cache/State work.
    /// This is not pure pattern-matcher CPU time.
    pub native_and_host_ms: f64,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BodyWorkerTrace {
    pub partition: usize,
    pub outcome: &'static str,
    pub started_at_ms: Option<f64>,
    pub initialization_ms: Option<f64>,
    pub finished_at_ms: Option<f64>,
    pub received_at_ms: Option<f64>,
    pub phase: Option<BodyPhaseMetrics>,
    /// Pending cache insertion time, before complete worker validation.
    pub first_recorded_at_ms: Option<f64>,
    pub last_recorded_at_ms: Option<f64>,
    /// Response borrowed after receipt/header/read validation, before normal
    /// output validation or candidate rollback. Not final AST adoption.
    pub first_consumed_at_ms: Option<f64>,
    pub last_consumed_at_ms: Option<f64>,
    pub max_recorded_to_consumed_ms: Option<f64>,
    pub max_ready_to_consumed_ms: Option<f64>,
    pub hit_calls: u64,
    pub receipt_rejections: u64,
    pub header_rejections: u64,
    pub state_rejections: u64,
    pub state_errors: u64,
    /// Unique entries borrowed after cache validation, not adopted AST nodes.
    pub unique_consumed_entries: usize,
    /// Never looked up after receipt of the committed capsule. A query before
    /// readiness cannot be assigned to an entry without retaining source keys.
    pub unqueried_after_receipt: usize,
    /// Mutually exclusive last observed reason for entries never consumed.
    pub unused_receipt_rejected: usize,
    pub unused_header_rejected: usize,
    pub unused_state_rejected: usize,
    pub unused_state_error: usize,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// One host document-parse attempt, not a project/round aggregate. In project
/// analysis this may be a dependency document rather than the requested URI.
pub struct BodyPreparationTrace {
    pub main: BodyPhaseMetrics,
    pub helper_join_wait_ms: f64,
    pub workers: Vec<BodyWorkerTrace>,
    pub lookup_calls: u64,
    pub lookups_while_pending: u64,
    pub no_ready_cache: u64,
    pub no_matching_world: u64,
    pub key_not_in_ready_caches: u64,
    pub cancelled_lookups: u64,
    pub foreign_capsules: u64,
    pub cancelled_capsules: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum EntryUse {
    #[default]
    Unqueried,
    ReceiptRejected,
    HeaderRejected,
    StateRejected,
    StateError,
    Consumed,
}

#[derive(Clone, Copy)]
pub(crate) struct TraceToken {
    pub partition: usize,
    pub slot: usize,
    pub recorded_at_ms: Option<f64>,
}

#[derive(Default)]
pub(crate) struct BodyPhaseClock {
    origin: Option<Instant>,
    next_origin: Option<Instant>,
    started: Option<Instant>,
    native_depth: usize,
    native_passes: u64,
    native: Duration,
    hooks: Duration,
    hooks_in_native: Duration,
    hook_calls: u64,
}

pub(crate) struct NativePassStart {
    started: Instant,
    hooks: Duration,
}

impl BodyPhaseClock {
    pub(crate) fn align_next_document(&mut self, origin: Option<Instant>) {
        self.next_origin = origin;
    }
    pub(crate) fn begin_document(&mut self, enabled: bool) {
        let origin = self.next_origin.take();
        *self = Self::default();
        if enabled {
            let now = Instant::now();
            self.origin = Some(origin.unwrap_or(now));
            self.started = Some(now);
        }
    }
    pub(crate) fn origin(&self) -> Option<Instant> {
        self.origin
    }
    pub(crate) fn offset_ms(&self) -> Option<f64> {
        self.origin
            .map(|origin| origin.elapsed().as_secs_f64() * 1000.0)
    }
    pub(crate) fn begin_native(&mut self) -> Option<NativePassStart> {
        self.origin?;
        self.native_passes += 1;
        self.native_depth += 1;
        (self.native_depth == 1).then(|| NativePassStart {
            started: Instant::now(),
            hooks: self.hooks,
        })
    }
    pub(crate) fn end_native(&mut self, start: Option<NativePassStart>) {
        if self.origin.is_none() {
            return;
        }
        self.native_depth = self.native_depth.saturating_sub(1);
        if let Some(start) = start {
            self.native += start.started.elapsed();
            self.hooks_in_native += self.hooks.saturating_sub(start.hooks);
        }
    }
    pub(crate) fn begin_hook(&self) -> Option<Instant> {
        self.origin.map(|_| Instant::now())
    }
    pub(crate) fn end_hook(&mut self, start: Option<Instant>) {
        if let Some(start) = start {
            self.hooks += start.elapsed();
            self.hook_calls += 1;
        }
    }
    pub(crate) fn finish_document(&mut self) -> Option<BodyPhaseMetrics> {
        let origin = self.origin.take()?;
        let started = self.started.take()?;
        let finished = Instant::now();
        Some(BodyPhaseMetrics {
            started_at_ms: started.duration_since(origin).as_secs_f64() * 1000.0,
            finished_at_ms: finished.duration_since(origin).as_secs_f64() * 1000.0,
            document_ms: finished.duration_since(started).as_secs_f64() * 1000.0,
            native_passes: self.native_passes,
            native_pass_ms: self.native.as_secs_f64() * 1000.0,
            hook_call_ms: self.hooks.as_secs_f64() * 1000.0,
            hook_calls: self.hook_calls,
            hook_ms_inside_native_pass: self.hooks_in_native.as_secs_f64() * 1000.0,
            native_and_host_ms: self
                .native
                .saturating_sub(self.hooks_in_native)
                .as_secs_f64()
                * 1000.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_clock_never_starts_a_timer_or_returns_old_metrics() {
        let mut clock = BodyPhaseClock::default();
        clock.begin_document(false);
        assert!(clock.begin_native().is_none());
        assert!(clock.begin_hook().is_none());
        assert!(clock.offset_ms().is_none());
        assert!(clock.finish_document().is_none());
        clock.begin_document(true);
        assert!(clock.finish_document().is_some());
        clock.begin_document(false);
        assert!(clock.finish_document().is_none());
    }
    #[test]
    fn nested_native_passes_are_counted_without_double_counting_wall_time() {
        let mut clock = BodyPhaseClock::default();
        clock.begin_document(true);
        let outer = clock.begin_native();
        let inner = clock.begin_native();
        assert!(inner.is_none());
        let hook = clock.begin_hook();
        clock.end_hook(hook);
        clock.end_native(inner);
        clock.end_native(outer);
        assert_eq!(clock.native_depth, 0);
        let metrics = clock.finish_document().unwrap();
        assert_eq!(metrics.native_passes, 2);
        assert_eq!(metrics.hook_calls, 1);
        assert!(metrics.native_pass_ms >= metrics.hook_ms_inside_native_pass);
    }
}

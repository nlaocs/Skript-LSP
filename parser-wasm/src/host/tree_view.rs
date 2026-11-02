//! Invocation-scoped read-only ownership for Tree/declaration queries. No guest state
//! or accepted edits are stored here; dropping the lease revokes all access.
use super::*;
use std::{collections::hash_map::RandomState, hash::{BuildHasher, Hasher}, sync::atomic::{AtomicU64, Ordering}};

pub(super) const MAX_PAGE: usize = 128;
pub(super) const MAX_REPLY_BYTES: usize = 64 * 1024;
const MAX_QUERY_CALLS: usize = 4096;
const MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);
// A random process namespace prevents persisted old handles from aliasing the
// counter of another process. This is opaque identity, never semantic evidence.
static PROCESS_EPOCH: LazyLock<(u64, u64)> = LazyLock::new(|| {
    let state = RandomState::new();
    let mut high = state.build_hasher(); high.write_u8(0);
    let mut low = state.build_hasher(); low.write_u8(1);
    (high.finish(), low.finish())
});

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Reference {
    pub epoch_high: u64,
    pub epoch_low: u64,
    pub token: u64,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum AccessError {
    #[error("tree view is stale or belongs to another invocation")]
    Stale,
    #[error("tree access was cancelled")]
    Cancelled,
    #[error("tree query count quota exceeded")]
    QueryQuota,
    #[error("tree response byte quota exceeded")]
    ByteQuota,
    #[error("invalid tree page cursor or limit")]
    InvalidPage,
    #[error("tree view token space exhausted")]
    TokenExhausted,
}

pub(super) struct Lease {
    reference: Reference,
    tree: Arc<ParserRawTree>,
    cancellation: Option<DocumentCancellationToken>,
    calls: usize,
    bytes: usize,
    macro_snapshot: Option<Arc<super::macro_dependencies::Snapshot>>,
    macro_reads: super::macro_dependencies::Reads,
    macro_headers_only: bool,
}

impl Lease {
    pub fn new(
        tree: Arc<ParserRawTree>,
        generation: u64,
        cancellation: Option<DocumentCancellationToken>,
    ) -> Result<Self, AccessError> {
        let mut token = NEXT_TOKEN.load(Ordering::Relaxed);
        loop {
            let next = token.checked_add(1).ok_or(AccessError::TokenExhausted)?;
            match NEXT_TOKEN.compare_exchange_weak(token, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(current) => token = current,
            }
        }
        Ok(Self { reference: Reference { epoch_high: PROCESS_EPOCH.0, epoch_low: PROCESS_EPOCH.1, token, generation }, tree, cancellation,
            calls: 0, bytes: 0, macro_snapshot:None, macro_reads:Default::default(), macro_headers_only:false })
    }

    pub fn for_macro_headers(mut self)->Self {self.macro_headers_only=true;self}
    pub fn macro_headers_only(&self)->bool {self.macro_headers_only}
    pub fn with_macro_snapshot(mut self,snapshot:Arc<super::macro_dependencies::Snapshot>)->Self {self.macro_snapshot=Some(snapshot);self}
    pub fn macro_snapshot(&self)->Option<Arc<super::macro_dependencies::Snapshot>> {self.macro_snapshot.clone()}
    pub fn macro_reads(&self)->super::macro_dependencies::Reads {self.macro_reads.clone()}
    pub fn observe_macro_page(&mut self,namespace:&str,complete:bool,ids:impl Iterator<Item=u64>) {
        self.macro_reads.namespaces.insert(namespace.into(),complete);self.macro_reads.bindings.extend(ids);
    }

    #[cfg(test)]
    pub(super) fn measurement(&self) -> (usize, usize) {
        (self.calls, self.bytes)
    }

    pub fn reference(&self) -> Reference {
        self.reference
    }

    /// Charge even rejected requests, and check cancellation before native work.
    /// The tree is the exact immutable revision/generation supplied by the
    /// coordinator; only the currently installed Store lease can authorize it.
    pub fn read(&mut self, reference: Reference) -> Result<Arc<ParserRawTree>, AccessError> {
        self.calls = self.calls.saturating_add(1);
        if self.calls > MAX_QUERY_CALLS {
            return Err(AccessError::QueryQuota);
        }
        if self.cancellation.as_ref().is_some_and(DocumentCancellationToken::is_cancelled) {
            return Err(AccessError::Cancelled);
        }
        if reference != self.reference {
            return Err(AccessError::Stale);
        }
        Ok(Arc::clone(&self.tree))
    }

    /// Called before constructing an owned WIT response. Both per-response and
    /// cumulative accounting use a conservative serialized-size upper bound.
    pub fn reserve_response(&mut self, bytes: usize) -> Result<(), AccessError> {
        let total = self.bytes.saturating_add(bytes);
        if bytes > MAX_REPLY_BYTES || total > MAX_TOTAL_BYTES {
            return Err(AccessError::ByteQuota);
        }
        self.bytes = total;
        Ok(())
    }
}

pub(super) fn page(
    ids: &[ParserRawNodeId], cursor: u64, limit: u32,
) -> Result<(&[ParserRawNodeId], Option<u64>), AccessError> {
    let start = usize::try_from(cursor).map_err(|_| AccessError::InvalidPage)?;
    let limit = usize::try_from(limit).map_err(|_| AccessError::InvalidPage)?;
    if start > ids.len() || limit == 0 || limit > MAX_PAGE {
        return Err(AccessError::InvalidPage);
    }
    let end = start.saturating_add(limit).min(ids.len());
    Ok((&ids[start..end], (end < ids.len()).then_some(end as u64)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(source: &str) -> Arc<ParserRawTree> {
        Arc::new(skript_parser::parse_raw_tree(&MappedSource::identity(source),
            skript_parser::RawTreeOptions::for_skript_version(2, 15)))
    }

    #[test]
    fn leases_are_fresh_generation_bound_and_share_immutable_native_storage() {
        let original = tree("on load:\n    old\n");
        let mut first = Lease::new(original.clone(), 7, None).unwrap();
        let mut next_call = Lease::new(original.clone(), 7, None).unwrap();
        assert_ne!(first.reference(), next_call.reference());
        assert!(Arc::ptr_eq(&first.read(first.reference()).unwrap(), &original));
        assert_eq!(next_call.read(first.reference()).unwrap_err(), AccessError::Stale);
        let mut foreign_epoch = first.reference(); foreign_epoch.epoch_high ^= 1;
        assert_eq!(first.read(foreign_epoch).unwrap_err(), AccessError::Stale);
        let mut wrong_generation = first.reference();
        wrong_generation.generation += 1;
        assert_eq!(first.read(wrong_generation).unwrap_err(), AccessError::Stale);
        let replacement = tree("on load:\n    new\n");
        let mut after_edit = Lease::new(replacement.clone(), 8, None).unwrap();
        assert_eq!(after_edit.read(first.reference()).unwrap_err(), AccessError::Stale);
        assert_eq!(original.nodes[1].text, "old");
        assert_eq!(after_edit.read(after_edit.reference()).unwrap().nodes[1].text, "new");
        // A rollback/reopened document may reuse a generation, never the token.
        let mut rollback = Lease::new(original.clone(), 7, None).unwrap();
        assert_eq!(rollback.read(first.reference()).unwrap_err(), AccessError::Stale);
    }

    #[test]
    fn queries_are_cancelled_and_bounded_including_invalid_handle_attempts() {
        let cancel = DocumentCancellationToken::new();
        let mut lease = Lease::new(tree("# public\n"), 0, Some(cancel.clone())).unwrap();
        let reference = lease.reference();
        cancel.cancel();
        assert_eq!(lease.read(reference).unwrap_err(), AccessError::Cancelled);
        let mut lease = Lease::new(tree("# public\n"), 0, None).unwrap();
        let mut wrong = lease.reference(); wrong.token = 0;
        for _ in 0..MAX_QUERY_CALLS {
            assert_eq!(lease.read(wrong).unwrap_err(), AccessError::Stale);
        }
        assert_eq!(lease.read(lease.reference()).unwrap_err(), AccessError::QueryQuota);
        let mut lease = Lease::new(tree("# public\n"), 0, None).unwrap();
        assert_eq!(lease.reserve_response(MAX_REPLY_BYTES + 1), Err(AccessError::ByteQuota));
        for _ in 0..MAX_TOTAL_BYTES / MAX_REPLY_BYTES {
            lease.reserve_response(MAX_REPLY_BYTES).unwrap();
        }
        assert_eq!(lease.reserve_response(1), Err(AccessError::ByteQuota));
    }

    #[test]
    fn pages_preserve_order_without_returning_a_large_child_vector() {
        let ids = (0..300).map(ParserRawNodeId::new).collect::<Vec<_>>();
        let mut cursor = 0;
        let mut seen = Vec::new();
        loop {
            let (slice, next) = page(&ids, cursor, MAX_PAGE as u32).unwrap();
            assert!(slice.len() <= MAX_PAGE);
            seen.extend_from_slice(slice);
            match next { Some(next) => cursor = next, None => break }
        }
        assert_eq!(seen, ids);
        for (cursor, limit) in [(301, 1), (u64::MAX, 1), (0, 0), (0, MAX_PAGE as u32 + 1)] {
            assert_eq!(page(&ids, cursor, limit), Err(AccessError::InvalidPage));
        }
    }
}

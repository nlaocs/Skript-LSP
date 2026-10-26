//! Persistent call trace: checkpoints share the accepted prefix instead of
//! cloning every earlier component call when a speculative branch appends.
use super::HookCall;
use std::{fmt, sync::Arc};

#[derive(Clone, Default)]
pub(super) struct HookCallLog {
    tail: Option<Arc<Chunk>>,
    len: usize,
}

pub(super) struct Chunk {
    previous: HookCallLog,
    calls: Vec<HookCall>,
}

impl fmt::Debug for HookCallLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A document can have many thousands of chunks. Debugging must not
        // recursively walk the chain any more than dropping it may do so.
        f.debug_struct("HookCallLog")
            .field("len", &self.len)
            .finish()
    }
}

impl From<Vec<HookCall>> for HookCallLog {
    fn from(calls: Vec<HookCall>) -> Self {
        let mut log = Self::default();
        log.extend(calls);
        log
    }
}

impl HookCallLog {
    pub(super) fn len(&self) -> usize {
        self.len
    }

    /// Exact retained prefix, including order and duplicate calls. Equal values
    /// from a discarded branch do not authenticate another branch's resource.
    pub(super) fn is_prefix_of(&self, later: &Self) -> bool {
        let mut cursor = later;
        while cursor.len > self.len {
            let Some(chunk) = &cursor.tail else { return false; };
            cursor = &chunk.previous;
        }
        cursor.len == self.len && match (&self.tail, &cursor.tail) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a,b),
            _ => false,
        }
    }

    pub(super) fn last(&self) -> Option<&HookCall> {
        self.tail.as_ref()?.calls.last()
    }

    pub(super) fn extend(&mut self, calls: Vec<HookCall>) {
        if calls.is_empty() {
            return;
        }
        let len = self
            .len
            .checked_add(calls.len())
            .expect("hook call count overflow");
        let previous = std::mem::take(self);
        self.tail = Some(Arc::new(Chunk { previous, calls }));
        self.len = len;
    }

    pub(super) fn suffix(&self, start: usize) -> Vec<HookCall> {
        assert!(start <= self.len, "hook call suffix outside checkpoint");
        let mut pieces = Vec::new();
        let mut cursor = self;
        while cursor.len > start {
            let chunk = cursor.tail.as_ref().expect("nonempty hook call log");
            let offset = start.saturating_sub(chunk.previous.len);
            pieces.push(&chunk.calls[offset..]);
            cursor = &chunk.previous;
        }
        let mut calls = Vec::with_capacity(self.len - start);
        for piece in pieces.into_iter().rev() {
            calls.extend_from_slice(piece);
        }
        calls
    }

    pub(super) fn into_vec(self) -> Vec<HookCall> {
        self.suffix(0)
    }

    #[cfg(test)]
    pub(super) fn observer(&self) -> std::sync::Weak<Chunk> {
        Arc::downgrade(self.tail.as_ref().expect("nonempty observed log"))
    }

    #[cfg(test)]
    pub(super) fn shares_tail(&self, other: &Self) -> bool {
        match (&self.tail, &other.tail) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            (None, None) => true,
            _ => false,
        }
    }
}

impl Drop for HookCallLog {
    fn drop(&mut self) {
        let mut tail = self.tail.take();
        while let Some(chunk) = tail {
            match Arc::try_unwrap(chunk) {
                Ok(mut chunk) => tail = chunk.previous.tail.take(),
                // Another branch or checkpoint still owns this whole prefix.
                Err(_) => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(index: u64) -> HookCall {
        HookCall {
            component_id: "test".into(),
            subscription_id: index.to_string(),
            semantic_receipt: Some(index),
        }
    }

    #[test]
    fn exact_prefix_preserves_branch_identity_not_only_equal_call_values() {
        let mut prefix = HookCallLog::default();
        prefix.extend(vec![call(1)]);
        let mut accepted = prefix.clone();
        accepted.extend(vec![call(2),call(2)]);
        let mut rejected = prefix.clone();
        rejected.extend(vec![call(2),call(2)]);
        assert!(prefix.is_prefix_of(&accepted));
        assert!(!rejected.is_prefix_of(&accepted));
        assert!(!accepted.is_prefix_of(&prefix));
        assert!(HookCallLog::default().is_prefix_of(&accepted));
    }

    #[test]
    fn suffix_preserves_chunk_order_offsets_and_semantic_receipts() {
        assert_eq!(HookCallLog::default().last(), None);
        let expected = (0..9).map(call).collect::<Vec<_>>();
        let mut log = HookCallLog::default();
        log.extend(expected[..3].to_vec());
        let prefix = log.clone();
        assert_eq!(prefix.last(), Some(&call(2)));
        log.extend(Vec::new());
        log.extend(expected[3..5].to_vec());
        log.extend(expected[5..].to_vec());
        assert_eq!(log.last(), Some(&call(8)));
        for start in 0..=expected.len() {
            assert_eq!(log.suffix(start), expected[start..]);
        }
        assert_eq!(prefix.into_vec(), expected[..3]);
        assert_eq!(log.into_vec(), expected);
    }

    #[test]
    #[should_panic(expected = "hook call suffix outside checkpoint")]
    fn invalid_suffix_does_not_silently_truncate() {
        HookCallLog::default().suffix(1);
    }

    #[test]
    fn deep_logs_debug_and_drop_iteratively_and_preserve_shared_prefix() {
        // A small stack makes accidental recursive Debug/Drop fail reliably.
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let mut log = HookCallLog::from(vec![call(0)]);
                let prefix = log.clone();
                let observed = prefix.observer();
                for index in 1..30_000 {
                    log.extend(vec![call(index)]);
                }
                assert_eq!(log.len(), 30_000);
                assert!(format!("{log:?}").contains("30000"));
                assert_eq!(log.suffix(29_998), vec![call(29_998), call(29_999)]);
                drop(log);
                assert!(observed.upgrade().is_some());
                assert_eq!(prefix.suffix(0), vec![call(0)]);
                drop(prefix);
                assert!(observed.upgrade().is_none());
            })
            .unwrap()
            .join()
            .unwrap();
    }
}

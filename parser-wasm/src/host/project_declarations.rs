//! Native authority for bounded project header data. No AST, scopes, receipts,
//! runtime values or callback purity are imported through this protocol.
use super::*;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};

mod protocol;
pub use protocol::DeclarationExports;
pub(super) use protocol::{Arena, Invocation};

pub(super) const MAX_SOURCES: usize = 512;
const MAX_SOURCE_ID_BYTES: usize = 64 * 1024;
const MAX_SOURCE_SET_BYTES: usize = 2 * 1024 * 1024;
const MAX_TRACKED_SOURCES: usize = 4096;
const MAX_TRACKED_SOURCE_BYTES: usize = 8 * 1024 * 1024;

/// Native coordinator identity for one observed source lifetime. Parse-attempt
/// revisions are deliberately absent: identical header discovery must converge.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DeclarationSource {
    uri: String,
    document_id: String,
    generation: u64,
    source_epoch: u64,
    digest: String,
}

impl DeclarationSource {
    /// Hashes the actual source bytes; a caller-supplied digest is not accepted.
    pub fn new(uri: impl Into<String>, document_id: impl Into<String>, generation: u64,
        source_epoch: u64, source: &str) -> Self
    {
        Self { uri: uri.into(), document_id: document_id.into(), generation, source_epoch,
            digest: format!("{:x}", Sha256::digest(source.as_bytes())) }
    }

    /// Original source URI, supplied by the native project coordinator.
    pub fn uri(&self) -> &str { &self.uri }
    /// Document identity used by the native document transaction.
    pub fn document_id(&self) -> &str { &self.document_id }

    fn bytes(&self) -> usize { self.uri.len().saturating_add(self.document_id.len()).saturating_add(self.digest.len()) }
    fn valid(&self) -> bool {
        !self.uri.is_empty() && !self.document_id.is_empty()
            && self.uri.len() <= MAX_SOURCE_ID_BYTES && self.document_id.len() <= MAX_SOURCE_ID_BYTES
            && !self.uri.chars().any(char::is_control) && !self.document_id.chars().any(char::is_control)
    }
}

/// Explicit native source membership. An unconfigured standalone parse has an
/// unknown external namespace; it does not certify that other files are absent.
#[derive(Clone, Debug, Default)]
pub struct DeclarationInputs {
    current: Option<DeclarationSource>,
    sources: Vec<DeclarationSource>,
    membership_complete: bool,
    batch_epoch: Option<u64>,
    exports: Vec<DeclarationExports>,
}

impl DeclarationInputs {
    /// Describes the actual scanner/open-buffer view for this immutable batch.
    pub fn new(current: DeclarationSource, sources: Vec<DeclarationSource>, membership_complete: bool) -> Self {
        Self { current: Some(current), sources, membership_complete, batch_epoch: Some(0), exports: Vec::new() }
    }
    /// Monotone native project-view epoch. Increment it when the scanner/open
    /// source view changes; it is an authority gate, not a parse-attempt counter.
    pub fn with_epoch(mut self, epoch: u64) -> Self { self.batch_epoch = Some(epoch); self }
    /// Supplies immutable results from other sources; the host revalidates them.
    pub fn with_exports(mut self, exports: Vec<DeclarationExports>) -> Self { self.exports = exports; self }
    /// Whole-namespace dependency key, separate from per-attempt authority.
    pub fn fingerprint(&self) -> String {
        let current = self.current.as_ref().map(|source| source.document_id.as_str());
        let mut sources = self.sources.iter().filter(|source| Some(source.document_id.as_str()) != current).collect::<Vec<_>>();
        sources.sort();
        let mut exports = self.exports.iter().filter(|record| Some(record.source().document_id()) != current)
            .map(|record| (record.source().document_id(), record.fingerprint())).collect::<Vec<_>>();
        exports.sort();
        format!("{:x}", Sha256::digest(format!("{}|{:?}|{:?}", self.membership_complete, sources, exports).as_bytes()))
    }
}

#[derive(Clone, Debug)]
pub(super) struct SourceLease {
    stamp: DeclarationSource,
    live: Arc<AtomicBool>,
    lifetime: u64,
}
impl SourceLease {
    fn is_live(&self) -> bool { self.live.load(Ordering::Acquire) }
}

#[derive(Clone, Debug)]
struct ViewLease { key: String, live: Arc<AtomicBool> }

#[derive(Debug)]
pub(super) struct SourceView {
    view: ViewLease,
    current: SourceLease,
    membership: BTreeMap<String, SourceLease>,
    complete: bool,
}

impl SourceView {
    fn permits(&self, authority: &SourceAuthority, project: &str, source: &SourceLease) -> bool {
        self.complete && self.view_is_current(authority, project) && authority.is_current(project, source)
            && self.membership.get(&source.stamp.document_id)
                .is_some_and(|seen| seen.stamp == source.stamp && Arc::ptr_eq(&seen.live, &source.live))
    }

    fn view_is_current(&self, authority: &SourceAuthority, project: &str) -> bool {
        self.view.live.load(Ordering::Acquire) && authority.views.get(project)
            .is_some_and(|view| view.key == self.view.key && Arc::ptr_eq(&view.live, &self.view.live))
    }

    fn authority_key(&self) -> String {
        format!("{}|{}|{:?}", self.view.key, self.complete, self.membership.iter()
            .map(|(document, lease)| (document, &lease.stamp, lease.lifetime)).collect::<Vec<_>>())
    }
}

#[derive(Default)]
pub(super) struct SourceAuthority {
    sources: BTreeMap<(String, String), SourceLease>,
    epochs: BTreeMap<String, u64>,
    views: BTreeMap<String, ViewLease>,
    next_lifetime: u64,
}

fn invalid(message: impl Into<String>) -> HostError {
    StateError::InvalidInput { message: message.into() }.into()
}

impl SourceAuthority {
    /// Validate the whole native description before changing any current fence.
    pub(super) fn prepare(&mut self, request: &DocumentParseRequest, input: &DeclarationInputs)
        -> Result<SourceView, HostError>
    {
        let current = input.current.clone().unwrap_or_else(|| DeclarationSource::new(
            request.context.document_id.clone(), request.context.document_id.clone(), 0, 0, request.source.original()));
        if current.document_id != request.context.document_id
            || current.digest != format!("{:x}", Sha256::digest(request.source.original().as_bytes()))
        {
            return Err(invalid("project declaration source differs from the native request"));
        }
        let proposed = if input.current.is_some() { input.sources.clone() } else { vec![current.clone()] };
        if proposed.len() > MAX_SOURCES { return Err(invalid("project declaration source-count limit")); }
        let mut seen = BTreeMap::new();
        let mut uris = BTreeSet::new();
        let mut bytes = 0usize;
        for source in proposed {
            bytes = bytes.saturating_add(source.bytes());
            if !source.valid() || bytes > MAX_SOURCE_SET_BYTES || !uris.insert(source.uri.clone())
                || seen.insert(source.document_id.clone(), source).is_some()
            {
                return Err(invalid("invalid, duplicate or oversized project declaration source set"));
            }
        }
        if seen.get(&current.document_id) != Some(&current) {
            return Err(invalid("project declaration source set does not contain the current source"));
        }
        let complete = input.current.is_some() && input.membership_complete;
        if input.batch_epoch.is_some_and(|epoch| self.epochs.get(&request.project_uri).is_some_and(|latest| epoch < *latest)) {
            return Err(invalid("stale project declaration source batch"));
        }
        // Standalone parsing has an explicit document-only domain and cannot
        // lend or restore a versioned project's source authority.
        if input.current.is_none() {
            let lease = SourceLease { stamp: current.clone(), live: Arc::new(AtomicBool::new(true)), lifetime: 0 };
            return Ok(SourceView { view: ViewLease { key: "document-only".into(), live: Arc::new(AtomicBool::new(true)) }, current: lease.clone(), membership: BTreeMap::from([(current.document_id, lease)]), complete: false });
        }
        let mut next_sources = self.sources.clone();
        let mut revoked = Vec::new();
        let mut next_lifetime = self.next_lifetime;
        if complete {
            next_sources.retain(|(project, document), lease| {
                let keep = project != &request.project_uri || seen.contains_key(document);
                if !keep { revoked.push(Arc::clone(&lease.live)); }
                keep
            });
        }
        let mut membership = BTreeMap::new();
        for (document, stamp) in seen {
            let key = (request.project_uri.clone(), document.clone());
            let existing = next_sources.get(&key).filter(|lease| lease.stamp == stamp && lease.is_live()).cloned();
            let lease = if let Some(lease) = existing { lease } else {
                next_lifetime = next_lifetime.checked_add(1).ok_or_else(|| invalid("project source lifetime exhausted"))?;
                if let Some(prior) = next_sources.get(&key) { revoked.push(Arc::clone(&prior.live)); }
                SourceLease { stamp, live: Arc::new(AtomicBool::new(true)), lifetime: next_lifetime }
            };
            next_sources.insert(key, lease.clone());
            membership.insert(document, lease);
        }
        let stored_bytes = next_sources.iter().fold(0usize, |total, ((project, _), lease)|
            total.saturating_add(project.len()).saturating_add(lease.stamp.bytes()));
        if next_sources.len() > MAX_TRACKED_SOURCES || stored_bytes > MAX_TRACKED_SOURCE_BYTES {
            return Err(invalid("stored project source authority limit"));
        }
        let view_key = format!("{:x}", Sha256::digest(format!("{:?}|{}|{:?}", input.batch_epoch, complete,
            membership.iter().map(|(document, lease)| (document, &lease.stamp)).collect::<Vec<_>>()).as_bytes()));
        let view = self.views.get(&request.project_uri)
            .filter(|view| view.key == view_key && view.live.load(Ordering::Acquire)).cloned()
            .unwrap_or_else(|| ViewLease { key: view_key, live: Arc::new(AtomicBool::new(true)) });
        if let Some(previous) = self.views.insert(request.project_uri.clone(), view.clone())
            && !Arc::ptr_eq(&previous.live, &view.live) { previous.live.store(false, Ordering::Release); }
        for lease in revoked { lease.store(false, Ordering::Release); }
        self.sources = next_sources;
        self.next_lifetime = next_lifetime;
        if let Some(epoch) = input.batch_epoch { self.epochs.insert(request.project_uri.clone(), epoch); }
        Ok(SourceView { view, current: membership[&current.document_id].clone(), membership, complete })
    }

    pub(super) fn is_current(&self, project: &str, source: &SourceLease) -> bool {
        source.is_live() && self.sources.get(&(project.into(), source.stamp.document_id.clone()))
            .is_some_and(|current| current.stamp == source.stamp && Arc::ptr_eq(&current.live, &source.live))
    }

    pub(super) fn release(&mut self, project: &str, document: &str) {
        if let Some(source) = self.sources.remove(&(project.into(), document.into())) {
            source.live.store(false, Ordering::Release);
        }
        if !self.sources.keys().any(|(known, _)| known == project) {
            self.epochs.remove(project);
            if let Some(view) = self.views.remove(project) { view.live.store(false, Ordering::Release); }
        }
    }
}
impl Drop for SourceAuthority {
    fn drop(&mut self) {
        for source in self.sources.values() { source.live.store(false, Ordering::Release); }
        for view in self.views.values() { view.live.store(false, Ordering::Release); }
    }
}

/// Reuse the host's revocable provider universe while keeping source lifetime
/// separate from the revision-specific editing/semantic-transfer document fence.
#[derive(Clone, Debug)]
pub(super) struct ProviderStamp {
    epoch: Arc<AtomicBool>,
    catalog: Option<Arc<Catalog>>,
    source: Option<(String, String)>,
    owners: Vec<(String, String, u64, bool, bool)>,
}
impl ProviderStamp {
    pub(super) fn capture(host: &ParserHost) -> Self {
        Self { epoch: host.transfer_authority.identity(), catalog: host.config.syntax_catalog.clone(),
            source: host.config.syntax_catalog.as_deref().and_then(Catalog::source)
                .map(|s| (s.snapshot_id.clone(), s.source_digest.clone())),
            owners: host.components.iter().map(|c| (c.manifest.component_id.clone(), c.component_sha256.clone(),
                c.preparation_generation, c.disabled, c.unloaded)).collect() }
    }

    pub(super) fn is_current(&self, host: &ParserHost) -> bool { self.matches(&Self::capture(host)) }

    fn matches(&self, current: &Self) -> bool {
        self.epoch.load(Ordering::Acquire) && Arc::ptr_eq(&self.epoch, &current.epoch)
            && match (&self.catalog, &current.catalog) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            } && self.source == current.source && self.owners == current.owners
    }

    /// Never include the Arc address or an attempt-local token in semantic keys.
    pub(super) fn semantic_key(&self) -> String { format!("{:?}|{:?}", self.source, self.owners) }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PROJECT: &str = "file:///project";
    fn request(document: &str, revision: u64, text: &str) -> DocumentParseRequest {
        DocumentParseRequest::new(PROJECT, document, revision, Arc::<str>::from(text))
    }
    fn stamp(document: &str, generation: u64, epoch: u64, text: &str) -> DeclarationSource {
        DeclarationSource::new(format!("file:///project/{document}.sk"), document, generation, epoch, text)
    }

    #[test]
    fn same_source_reparse_preserves_authority_but_edits_and_lifetimes_revoke() {
        let mut authority = SourceAuthority::default();
        let original = stamp("a", 4, 8, "old");
        let input = DeclarationInputs::new(original.clone(), vec![original.clone()], true);
        let first = authority.prepare(&request("a", 1, "old"), &input).unwrap();
        let repeated = authority.prepare(&request("a", 20, "old"), &input).unwrap();
        assert!(Arc::ptr_eq(&first.current.live, &repeated.current.live));
        assert!(authority.is_current(PROJECT, &first.current));
        assert!(first.complete);
        for next in [stamp("a", 5, 8, "old"), stamp("a", 5, 9, "old"), stamp("a", 5, 9, "new")] {
            let text = if next.digest == stamp("a", 0, 0, "new").digest { "new" } else { "old" };
            let previous = authority.sources[&(PROJECT.into(), "a".into())].clone();
            let result = authority.prepare(&request("a", 21, text), &DeclarationInputs::new(next.clone(), vec![next], true)).unwrap();
            assert!(!authority.is_current(PROJECT, &previous));
            assert!(authority.is_current(PROJECT, &result.current));
        }
    }

    #[test]
    fn observed_delete_and_identical_recreate_never_revive_a_source_fence() {
        let mut authority = SourceAuthority::default();
        let a = stamp("a", 0, 0, "same"); let b = stamp("b", 0, 0, "b");
        let first = authority.prepare(&request("b", 1, "b"), &DeclarationInputs::new(b.clone(), vec![a.clone(), b.clone()], true)).unwrap();
        let old = first.membership["a"].clone();
        authority.prepare(&request("b", 2, "b"), &DeclarationInputs::new(b.clone(), vec![b.clone()], true)).unwrap();
        assert!(!old.is_live());
        let recreated = authority.prepare(&request("b", 3, "b"), &DeclarationInputs::new(b.clone(), vec![a, b], true)).unwrap();
        assert!(!authority.is_current(PROJECT, &old));
        assert!(authority.is_current(PROJECT, &recreated.membership["a"]));
        assert_ne!(first.authority_key(), recreated.authority_key());
        assert!(!Arc::ptr_eq(&old.live, &recreated.membership["a"].live));
    }

    #[test]
    fn partial_membership_does_not_prove_absence_and_release_is_revocable() {
        let mut authority = SourceAuthority::default();
        let a = stamp("a", 0, 0, "a"); let b = stamp("b", 0, 0, "b");
        let first = authority.prepare(&request("b", 1, "b"), &DeclarationInputs::new(b.clone(), vec![a, b.clone()], true)).unwrap();
        let old = first.membership["a"].clone();
        let partial = authority.prepare(&request("b", 2, "b"), &DeclarationInputs::new(b.clone(), vec![b], false)).unwrap();
        assert!(!partial.complete);
        assert!(authority.is_current(PROJECT, &old));
        assert!(!first.permits(&authority, PROJECT, &old), "a new partial view revokes an older full view without declaring the source absent");
        assert!(!partial.permits(&authority, PROJECT, &old), "a retained source is not an import permission for this partial view");
        authority.release(PROJECT, "a");
        assert!(!old.is_live());
        let standalone = authority.prepare(&request("b", 3, "b"), &DeclarationInputs::default()).unwrap();
        assert!(!standalone.complete);
    }

    #[test]
    fn mismatched_duplicate_and_oversized_native_views_are_atomic_rejections() {
        let mut authority = SourceAuthority::default();
        let a = stamp("a", 1, 2, "a");
        let first = authority.prepare(&request("a", 1, "a"), &DeclarationInputs::new(a.clone(), vec![a.clone()], true)).unwrap();
        let cases = [
            DeclarationInputs::new(a.clone(), vec![a.clone(), a.clone()], true),
            DeclarationInputs::new(a.clone(), vec![], true),
            DeclarationInputs::new(stamp("a", 2, 2, "different"), vec![a.clone()], true),
            DeclarationInputs::new(a.clone(), vec![a; MAX_SOURCES + 1], true),
        ];
        for input in cases {
            assert!(authority.prepare(&request("a", 2, "a"), &input).is_err());
            assert!(authority.is_current(PROJECT, &first.current));
        }
    }

    #[test]
    fn the_same_batch_view_is_shared_across_consumer_reparses() {
        let mut authority = SourceAuthority::default();
        let a = stamp("a", 1, 2, "a"); let b = stamp("b", 3, 4, "b");
        let sources = vec![a.clone(), b.clone()];
        let first = authority.prepare(&request("a", 1, "a"), &DeclarationInputs::new(a, sources.clone(), true).with_epoch(9)).unwrap();
        let second = authority.prepare(&request("b", 10, "b"), &DeclarationInputs::new(b, sources, true).with_epoch(9)).unwrap();
        assert!(Arc::ptr_eq(&first.view.live, &second.view.live));
        assert!(first.permits(&authority, PROJECT, &second.current));
        assert!(second.permits(&authority, PROJECT, &first.current));
        assert!(first.view_is_current(&authority, PROJECT));
        assert_eq!(first.authority_key(), second.authority_key());
    }

    #[test]
    fn stale_batches_and_exhausted_lifetimes_do_not_invalidate_newer_sources() {
        let mut authority = SourceAuthority::default();
        let a = stamp("a", 1, 2, "a");
        let input = DeclarationInputs::new(a.clone(), vec![a.clone()], true).with_epoch(9);
        let first = authority.prepare(&request("a", 1, "a"), &input).unwrap();
        assert!(authority.prepare(&request("a", 2, "a"), &DeclarationInputs::new(a.clone(), vec![a.clone()], true).with_epoch(8)).is_err());
        assert!(authority.is_current(PROJECT, &first.current));
        authority.next_lifetime = u64::MAX;
        let changed = stamp("a", 2, 2, "changed");
        assert!(authority.prepare(&request("a", 3, "changed"), &DeclarationInputs::new(changed.clone(), vec![changed], true).with_epoch(10)).is_err());
        assert!(authority.is_current(PROJECT, &first.current));
        assert_eq!(authority.epochs.get(PROJECT), Some(&9));
        assert!(first.view_is_current(&authority, PROJECT));
        let standalone = authority.prepare(&request("a", 4, "a"), &DeclarationInputs::default()).unwrap();
        assert!(!authority.is_current(PROJECT, &standalone.current));
        assert!(authority.is_current(PROJECT, &first.current));
    }

    #[test]
    fn stored_source_bytes_are_bounded_before_revocation() {
        let mut authority = SourceAuthority::default();
        let a = stamp("a", 1, 1, "a");
        let first = authority.prepare(&request("a", 1, "a"), &DeclarationInputs::new(a.clone(), vec![a], true)).unwrap();
        let mut next = 0u64;
        loop {
            let document = format!("extra-{next}");
            let source = DeclarationSource::new(format!("file:///{}", "x".repeat(60_000)), document.clone(), 1, 1, "x");
            let request = request(&document, 1, "x");
            let input = DeclarationInputs::new(source.clone(), vec![source], false);
            if authority.prepare(&request, &input).is_err() { break; }
            next += 1;
            assert!(next < 200, "stored byte quota must stop partial-view accumulation");
        }
        assert!(authority.is_current(PROJECT, &first.current));
        assert!(next > 0);
    }

    #[test]
    fn provider_stamp_belongs_to_one_host_and_revocation_is_not_a_semantic_key() {
        let mut first = semantic_transfer::Authority::default();
        let second = semantic_transfer::Authority::default();
        let stamp = ProviderStamp { epoch: first.identity(), catalog: None, source: None,
            owners: vec![("provider".into(), "binary".into(), 1, false, false)] };
        assert!(stamp.matches(&stamp));
        let mut other = stamp.clone(); other.epoch = second.identity();
        assert!(!stamp.matches(&other));
        other = stamp.clone(); other.owners[0].2 += 1;
        assert!(!stamp.matches(&other), "preparation generation is checked separately from source data");
        first.revoke();
        assert!(!stamp.matches(&stamp));
        let mut fresh = stamp.clone(); fresh.epoch = first.identity();
        assert!(fresh.matches(&fresh));
        assert_eq!(stamp.semantic_key(), fresh.semantic_key());
    }
}
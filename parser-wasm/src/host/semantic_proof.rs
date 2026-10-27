//! Host-owned provenance for provider facts consumed after mutable AST phases.
//!
//! Receipt text in guest-visible metadata is only a reference. Authority is the
//! private per-parse registry plus the native parser's retained call trace.
use super::{
    AstNode, AstTree, CaptureValue, HookCall, HookMode, HookOutput, HookPayload, WitMetadataEntry,
};
#[cfg(test)]
use super::HookCallOrigin;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write};
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};

const RECEIPT: &str = "parser.semantic-writer-receipt.v1";

mod body;
#[cfg(test)]
mod local_state_tests;
pub(super) use body::BodyReceiptBundle;

/// Direct reach of accepted host effects, not a guest purity certificate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum SemanticEffects {
    #[default]
    None,
    /// Host-proven own private Invocation/Parse writes. Keep the local receipt.
    Local,
    /// Context, extra parsing, registration, or State writes not proven local.
    Unbound,
}

#[derive(Debug, Clone)]
struct Receipt {
    issuer: String,
    writers: BTreeSet<String>,
    parents: BTreeSet<u64>,
    // Host-only accepted nested calls, distinct from payload metadata ancestry.
    admitted_nested: BTreeSet<u64>,
    valid: bool,
    unbound: bool,
}

#[derive(Debug, Default)]
pub(super) struct ReceiptRegistry {
    receipts: BTreeMap<u64, Receipt>,
    sequence: Arc<AtomicU64>,
    exhausted: bool,
}

#[derive(Default)]
pub(super) struct DispatchWriters {
    writers: BTreeSet<String>,
    parents: BTreeSet<u64>,
    admitted_nested: BTreeSet<u64>,
    invalid: bool,
}

fn metadata_receipts(metadata: &[WitMetadataEntry], output: &mut BTreeSet<u64>) -> bool {
    let mut valid = true;
    for entry in metadata
        .iter()
        .filter(|entry| entry.owner_component_id.is_none() && entry.key == RECEIPT)
    {
        match entry.value.parse() {
            Ok(token) => {
                output.insert(token);
            }
            Err(_) => valid = false,
        }
    }
    valid
}

fn visit_metadata(payload: &HookPayload, mut visit: impl FnMut(&[WitMetadataEntry])) {
    if let Some(metadata) = super::payload_metadata(payload) {
        visit(metadata);
    }
    if let HookPayload::Expression(value) = payload {
        for candidate in &value.candidates {
            visit(&candidate.metadata);
        }
    }
}

fn visit_metadata_mut(
    payload: &mut HookPayload,
    mut visit: impl FnMut(&mut Vec<WitMetadataEntry>),
) {
    if let Some(metadata) = super::payload_metadata_mut(payload) {
        visit(metadata);
    }
    if let HookPayload::Expression(value) = payload {
        for candidate in &mut value.candidates {
            visit(&mut candidate.metadata);
        }
    }
}

impl ReceiptRegistry {
    pub(super) fn with_sequence(sequence: Arc<AtomicU64>) -> Self {
        Self { sequence, ..Self::default() }
    }
    pub(super) fn begin(&self, payload: &HookPayload) -> DispatchWriters {
        let mut result = DispatchWriters::default();
        visit_metadata(payload, |metadata| {
            result.invalid |= !metadata_receipts(metadata, &mut result.parents)
        });
        for token in &result.parents {
            if let Some(receipt) = self.receipts.get(token) {
                result.writers.extend(receipt.writers.iter().cloned());
                result.invalid |= !receipt.valid;
            } else {
                result.invalid = true;
            }
        }
        result
    }

    /// Admit actual calls retained by this exact successful native/Add-on parse
    /// continuation. Guest metadata and ordinary parent links never grant this.
    pub(super) fn admit_nested<'a>(
        &self,
        writers: &mut DispatchWriters,
        calls: impl IntoIterator<Item = &'a HookCall>,
    ) {
        for call in calls {
            let Some(token) = call.semantic_receipt else {
                continue;
            };
            let Some(receipt) = self
                .receipts
                .get(&token)
                .filter(|receipt| receipt.issuer == call.component_id)
            else {
                writers.invalid = true;
                continue;
            };
            writers.invalid |= !receipt.valid;
            writers.writers.extend(receipt.writers.iter().cloned());
            writers.admitted_nested.insert(token);
        }
    }

    pub(super) fn accept(
        &mut self,
        writers: &mut DispatchWriters,
        payload: &mut HookPayload,
        component: &str,
        mode: HookMode,
        semantic_effects: SemanticEffects,
    ) -> Option<u64> {
        if matches!(mode, HookMode::Observe) && semantic_effects == SemanticEffects::None {
            return None;
        }
        writers.writers.insert(component.to_owned());
        let Ok(previous) = self.sequence.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| value.checked_add(1)) else {
            self.exhausted = true;
            return None;
        };
        let token = previous + 1;
        self.receipts.insert(
            token,
            Receipt {
                issuer: component.to_owned(),
                writers: writers.writers.clone(),
                parents: writers.parents.clone(),
                admitted_nested: writers.admitted_nested.clone(),
                valid: !writers.invalid,
                unbound: semantic_effects == SemanticEffects::Unbound,
            },
        );
        writers.parents.insert(token);
        visit_metadata_mut(payload, |metadata| {
            metadata.retain(|entry| entry.owner_component_id.is_some() || entry.key != RECEIPT);
            metadata.push(WitMetadataEntry {
                owner_component_id: None,
                key: RECEIPT.to_owned(),
                value: token.to_string(),
            });
        });
        Some(token)
    }
}

/// Metadata paths reachable from cacheable transport payloads. Keep immutable
/// contexts and nested summaries in this traversal: their receipt references
/// must be rebound too, even when the guest changes only the outer candidate.
fn cache_metadata(payload: &HookPayload, visit: &mut impl FnMut(&[WitMetadataEntry])) {
    use super::HookPayload::*;
    let context = match payload {
        Expression(value) => Some(&value.context),
        RegisteredExpression(value) => Some(&value.context),
        DefaultExpression(value) => Some(&value.context),
        Effect(value) => Some(&value.context),
        Condition(value) => Some(&value.context),
        _ => None,
    };
    if let Some(context) = context {
        for frame in &context.section_stack {
            visit(&frame.metadata);
            for capture in &frame.captures {
                if let Some(summary) = &capture.summary {
                    visit(&summary.metadata);
                }
            }
        }
    }
    match payload {
        Expression(value) => {
            for candidate in &value.candidates {
                visit(&candidate.metadata);
            }
        }
        RegisteredExpression(value) => {
            visit(&value.metadata);
            for child in &value.children {
                visit(&child.metadata);
            }
            for capture in &value.parsed_captures {
                if let Some(summary) = &capture.summary {
                    visit(&summary.metadata);
                }
            }
        }
        DefaultExpression(value) => visit(&value.metadata),
        Effect(value) => {
            for candidate in value.candidate.iter().chain(value.alternatives.iter()) {
                visit(&candidate.metadata);
                for capture in &candidate.parsed_captures {
                    if let Some(summary) = &capture.summary {
                        visit(&summary.metadata);
                    }
                }
            }
        }
        Condition(value) => {
            visit(&value.candidate.metadata);
            for child in &value.candidate.children {
                visit(&child.metadata);
            }
        }
        _ => visit_metadata(payload, visit),
    }
}

fn cache_metadata_mut(
    payload: &mut HookPayload,
    visit: &mut impl FnMut(&mut Vec<WitMetadataEntry>),
) {
    use super::HookPayload::*;
    let context = match payload {
        Expression(value) => Some(&mut value.context),
        RegisteredExpression(value) => Some(&mut value.context),
        DefaultExpression(value) => Some(&mut value.context),
        Effect(value) => Some(&mut value.context),
        Condition(value) => Some(&mut value.context),
        _ => None,
    };
    if let Some(context) = context {
        for frame in &mut context.section_stack {
            visit(&mut frame.metadata);
            for capture in &mut frame.captures {
                if let Some(summary) = &mut capture.summary {
                    visit(&mut summary.metadata);
                }
            }
        }
    }
    match payload {
        Expression(value) => {
            for candidate in &mut value.candidates {
                visit(&mut candidate.metadata);
            }
        }
        RegisteredExpression(value) => {
            visit(&mut value.metadata);
            for child in &mut value.children {
                visit(&mut child.metadata);
            }
            for capture in &mut value.parsed_captures {
                if let Some(summary) = &mut capture.summary {
                    visit(&mut summary.metadata);
                }
            }
        }
        DefaultExpression(value) => visit(&mut value.metadata),
        Effect(value) => {
            for candidate in value
                .candidate
                .iter_mut()
                .chain(value.alternatives.iter_mut())
            {
                visit(&mut candidate.metadata);
                for capture in &mut candidate.parsed_captures {
                    if let Some(summary) = &mut capture.summary {
                        visit(&mut summary.metadata);
                    }
                }
            }
        }
        Condition(value) => {
            visit(&mut value.candidate.metadata);
            for child in &mut value.candidate.children {
                visit(&mut child.metadata);
            }
        }
        _ => visit_metadata_mut(payload, visit),
    }
}

fn receipt_entry(entry: &WitMetadataEntry) -> bool {
    entry.owner_component_id.is_none() && entry.key == RECEIPT
}

/// Preserve receipt alias topology while excluding per-parse numeric identities.
/// Opted-in guests treat this protected host field as opaque/nonsemantic.
fn receipt_ordinals(payload: &HookPayload) -> Option<BTreeMap<String, usize>> {
    let mut map = BTreeMap::new();
    let mut valid = true;
    cache_metadata(payload, &mut |metadata| {
        for entry in metadata.iter().filter(|entry| receipt_entry(entry)) {
            if entry
                .value
                .parse::<u64>()
                .ok()
                .is_none_or(|token| token == 0)
            {
                valid = false;
            }
            let ordinal = map.len() + 1;
            map.entry(entry.value.clone()).or_insert(ordinal);
        }
    });
    valid.then_some(map)
}

/// Render a canonical key without cloning large type-option payloads. Restore
/// every original metadata value before the actual guest sees its request.
pub(super) fn with_payload_receipts_normalized<T>(
    payload: &mut HookPayload,
    render: impl FnOnce(&HookPayload) -> T,
) -> Option<T> {
    let ordinals = receipt_ordinals(payload)?;
    let mut originals = Vec::new();
    cache_metadata_mut(payload, &mut |metadata| {
        for entry in metadata.iter_mut().filter(|entry| receipt_entry(entry)) {
            let ordinal = ordinals[&entry.value];
            originals.push(std::mem::replace(&mut entry.value, ordinal.to_string()));
        }
    });
    let result = render(payload);
    let mut originals = originals.into_iter();
    cache_metadata_mut(payload, &mut |metadata| {
        for entry in metadata.iter_mut().filter(|entry| receipt_entry(entry)) {
            entry.value = originals
                .next()
                .expect("receipt traversal is unchanged during key rendering");
        }
    });
    debug_assert!(originals.next().is_none());
    Some(result)
}

/// Stored output references are ordinal placeholders, never old registry IDs.
/// Unknown/forged references cannot become cache entries.
pub(super) fn normalize_output_receipts(original: &HookPayload, output: &mut HookOutput) -> bool {
    let Some(ordinals) = receipt_ordinals(original) else {
        return false;
    };
    let mut valid = true;
    if let Some(payload) = &mut output.replacement {
        cache_metadata_mut(payload, &mut |metadata| {
            for entry in metadata.iter_mut().filter(|entry| receipt_entry(entry)) {
                if let Some(ordinal) = ordinals.get(&entry.value) {
                    entry.value = ordinal.to_string();
                } else {
                    valid = false;
                }
            }
        });
    }
    valid
}

/// Rebind before normal output validation, which then mints current acceptance
/// receipts. No cached number is ever interpreted as a current registry token.
pub(super) fn restore_output_receipts(original: &HookPayload, output: &mut HookOutput) -> bool {
    let Some(ordinals) = receipt_ordinals(original) else {
        return false;
    };
    let current: BTreeMap<_, _> = ordinals
        .into_iter()
        .map(|(token, ordinal)| (ordinal, token))
        .collect();
    let mut valid = true;
    if let Some(payload) = &mut output.replacement {
        cache_metadata_mut(payload, &mut |metadata| {
            for entry in metadata.iter_mut().filter(|entry| receipt_entry(entry)) {
                if let Some(token) = entry
                    .value
                    .parse::<usize>()
                    .ok()
                    .and_then(|ordinal| current.get(&ordinal))
                {
                    entry.value = token.clone();
                } else {
                    valid = false;
                }
            }
        });
    }
    valid
}

#[derive(Debug, Default)]
pub(super) struct SemanticProofRegistry {
    forest: Option<[u8; 32]>,
    source: Option<[u8; 32]>,
    writers: BTreeMap<u64, BTreeSet<String>>,
    valid: bool,
}

struct Canonical {
    tree: AstTree,
    rank: BTreeMap<u64, u64>,
    postorder: Vec<u64>,
}

fn references(node: &AstNode) -> Vec<u64> {
    let mut ids = node.children.clone();
    for capture in &node.captures {
        match &capture.value {
            CaptureValue::Node(id) => ids.push(*id),
            CaptureValue::Nodes(nodes) => ids.extend(nodes),
            _ => {}
        }
    }
    ids
}

fn is_recovery(node: &AstNode) -> bool {
    node.metadata
        .iter()
        .any(|entry| entry.owner_component_id.is_none() && entry.key == "parser.editor-recovery")
}

fn canonical(tree: &AstTree, ignore_recovery: bool) -> Option<Canonical> {
    let index: BTreeMap<_, _> = tree.nodes.iter().map(|node| (node.id, node)).collect();
    if index.len() != tree.nodes.len() {
        return None;
    }
    let roots: Vec<_> = tree
        .roots
        .iter()
        .copied()
        .filter(|id| !ignore_recovery || !index.get(id).is_some_and(|node| is_recovery(node)))
        .collect();
    let mut rank = BTreeMap::new();
    let mut order = Vec::new();
    let mut postorder = Vec::new();
    let mut visiting = BTreeSet::new();
    let mut finished = BTreeSet::new();
    for root in &roots {
        let mut work = vec![(*root, false)];
        while let Some((id, exit)) = work.pop() {
            let node = *index.get(&id)?;
            if exit {
                visiting.remove(&id);
                finished.insert(id);
                postorder.push(id);
                continue;
            }
            if finished.contains(&id) {
                continue;
            }
            if !visiting.insert(id) {
                return None;
            }
            rank.insert(id, order.len() as u64);
            order.push(id);
            work.push((id, true));
            work.extend(
                references(node)
                    .into_iter()
                    .rev()
                    .map(|child| (child, false)),
            );
        }
    }
    let nodes = order
        .iter()
        .map(|id| {
            let mut node = (*index[id]).clone();
            node.id = rank[id];
            for child in &mut node.children {
                *child = rank[child];
            }
            for capture in &mut node.captures {
                match &mut capture.value {
                    CaptureValue::Node(id) => *id = rank[id],
                    CaptureValue::Nodes(ids) => {
                        for id in ids {
                            *id = rank[id];
                        }
                    }
                    _ => {}
                }
            }
            node
        })
        .collect();
    Some(Canonical {
        tree: AstTree {
            roots: roots.iter().map(|id| rank[id]).collect(),
            nodes,
        },
        rank,
        postorder,
    })
}

fn debug_digest(domain: &str, value: &impl fmt::Debug) -> [u8; 32] {
    struct Sink(Sha256);
    impl fmt::Write for Sink {
        fn write_str(&mut self, value: &str) -> fmt::Result {
            self.0.update(value.as_bytes());
            Ok(())
        }
    }
    let mut sink = Sink(Sha256::new());
    write!(&mut sink, "{domain}:{value:?}").expect("hash sink is infallible");
    sink.0.finalize().into()
}

fn digest(tree: &AstTree) -> [u8; 32] {
    debug_digest("semantic-forest-v1", tree)
}

impl SemanticProofRegistry {
    /// Whole-result gate for publishing a body artifact after mutable phases.
    pub(super) fn authenticates_result(&self, tree: &AstTree, source: &skript_parser::MappedSource) -> bool {
        self.valid && self.source == Some(debug_digest("semantic-source-v1", source))
            && canonical(tree, true).is_some_and(|value| self.forest == Some(digest(&value.tree)))
    }
    pub(super) fn capture(
        tree: &AstTree,
        receipts: Option<&ReceiptRegistry>,
        calls: &[HookCall],
    ) -> Self {
        let Some(canonical) = canonical(tree, false) else {
            return Self::default();
        };
        let mut result = Self {
            forest: Some(digest(&canonical.tree)),
            source: None,
            writers: BTreeMap::new(),
            valid: !receipts.is_some_and(|registry| registry.exhausted),
        };
        let mut retained: BTreeSet<_> = calls
            .iter()
            .filter_map(|call| call.semantic_receipt)
            .collect();
        // Only a retained accepted call can admit its exact nested parse calls.
        // Do not follow ordinary metadata parents here: an existing registry
        // token, transient attempt, or copied payload is not adoption evidence.
        let mut nested = retained.iter().copied().collect::<Vec<_>>();
        while let Some(token) = nested.pop() {
            if let Some(receipt) = receipts.and_then(|registry| registry.receipts.get(&token)) {
                for child in &receipt.admitted_nested {
                    if retained.insert(*child) {
                        nested.push(*child);
                    }
                }
            }
        }
        let mut attached = BTreeSet::new();
        let mut own = BTreeMap::new();
        for node in &tree.nodes {
            if !canonical.rank.contains_key(&node.id) {
                continue;
            }
            let mut tokens = BTreeSet::new();
            result.valid &= metadata_receipts(&node.metadata, &mut tokens);
            if let Some(summary) = &node.summary {
                result.valid &= metadata_receipts(&summary.metadata, &mut tokens);
            }
            let mut writers = BTreeSet::new();
            for token in tokens {
                attached.insert(token);
                let Some(receipt) = receipts
                    .and_then(|registry| registry.receipts.get(&token))
                    .filter(|_| retained.contains(&token))
                else {
                    result.valid = false;
                    continue;
                };
                result.valid &= receipt.valid;
                writers.extend(receipt.writers.iter().cloned());
            }
            own.insert(node.id, writers);
        }
        // Earlier committed rounds remain bound by the latest cumulative receipt.
        let mut work: Vec<_> = attached.iter().copied().collect();
        while let Some(token) = work.pop() {
            if let Some(receipt) = receipts.and_then(|registry| registry.receipts.get(&token)) {
                for parent in &receipt.parents {
                    if attached.insert(*parent) {
                        work.push(*parent);
                    }
                }
            }
        }
        let mut global = BTreeSet::new();
        for token in &retained {
            if let Some(receipt) = receipts
                .and_then(|registry| registry.receipts.get(token))
                .filter(|receipt| receipt.unbound)
            {
                global.extend(receipt.writers.iter().cloned());
            }
        }
        for token in retained.difference(&attached) {
            let Some(receipt) = receipts.and_then(|registry| registry.receipts.get(token)) else {
                result.valid = false;
                continue;
            };
            result.valid &= receipt.valid;
            // Matching/other receipts without a surviving native-node binding
            // conservatively affect the document, never disappear from proof.
            global.extend(receipt.writers.iter().cloned());
        }
        let index: BTreeMap<_, _> = tree.nodes.iter().map(|node| (node.id, node)).collect();
        let mut ancestors = own.clone();
        for writers in ancestors.values_mut() {
            writers.extend(global.iter().cloned());
        }
        let mut queue = std::collections::VecDeque::from_iter(canonical.rank.keys().copied());
        while let Some(id) = queue.pop_front() {
            let inherited = ancestors.get(&id).cloned().unwrap_or_default();
            for child in references(index[&id]) {
                let entry = ancestors.entry(child).or_default();
                let before = entry.len();
                entry.extend(inherited.iter().cloned());
                if entry.len() != before {
                    queue.push_back(child);
                }
            }
        }
        let mut subtree = own;
        for id in &canonical.postorder {
            let mut writers = subtree.get(id).cloned().unwrap_or_default();
            for child in references(index[id]) {
                writers.extend(subtree.get(&child).into_iter().flatten().cloned());
            }
            subtree.insert(*id, writers);
        }
        for (id, rank) in canonical.rank {
            let mut writers = ancestors.remove(&id).unwrap_or_default();
            writers.extend(subtree.remove(&id).unwrap_or_default());
            result.writers.insert(rank, writers);
        }
        result
    }

    /// Bind the native proof before mutable AST phases. A public source-map swap
    /// must not relocate otherwise unchanged semantic facts.
    pub(super) fn bind_source(mut self, source: &skript_parser::MappedSource) -> Self {
        self.source = Some(debug_digest("semantic-source-v1", source));
        self
    }

    pub(super) fn view_for_source<'a>(
        &self,
        tree: &'a AstTree,
        source: &skript_parser::MappedSource,
    ) -> SemanticWitnesses<'a> {
        if self.source != Some(debug_digest("semantic-source-v1", source)) {
            return SemanticWitnesses { nodes: BTreeMap::new(), writers: BTreeMap::new() };
        }
        self.view(tree)
    }

    pub(super) fn view<'a>(&self, tree: &'a AstTree) -> SemanticWitnesses<'a> {
        let Some(canonical) = canonical(tree, true) else {
            return SemanticWitnesses {
                nodes: BTreeMap::new(),
                writers: BTreeMap::new(),
            };
        };
        if !self.valid || self.forest != Some(digest(&canonical.tree)) {
            return SemanticWitnesses {
                nodes: BTreeMap::new(),
                writers: BTreeMap::new(),
            };
        }
        let writers = canonical
            .rank
            .into_iter()
            .filter_map(|(id, rank)| {
                self.writers
                    .get(&rank)
                    .cloned()
                    .map(|writers| (id, writers))
            })
            .collect();
        let nodes = tree.nodes.iter().map(|node| (node.id, node)).collect();
        SemanticWitnesses { nodes, writers }
    }
}

/// A read-only view of final nodes whose native semantics and provider history
/// survived every AST phase. Build once per result, then query many facts.
pub struct SemanticWitnesses<'a> {
    nodes: BTreeMap<u64, &'a AstNode>,
    writers: BTreeMap<u64, BTreeSet<String>>,
}

impl SemanticWitnesses<'_> {
    /// Validates native facts, including immutable capture expectations.
    pub fn native_valid(&self, node: &AstNode) -> bool {
        self.writers.contains_key(&node.id)
            && self
                .nodes
                .get(&node.id)
                .is_some_and(|current| std::ptr::eq(*current, node))
    }
    /// Provider facts require every adopted modifying contributor in their
    /// ancestor/subtree scope to be the same explicitly named issuer.
    pub fn provider_valid(&self, node: &AstNode, issuer: &str) -> bool {
        self.unique_issuer(node) == Some(issuer)
    }
    /// Authorship of an attached, host-stamped metadata entry in the current
    /// sealed forest. Unlike `provider_valid`, this permits other accepted
    /// writers in the scope. It is for data-only editing/presentation, never a
    /// certificate of a type, parse result or other semantic guarantee.
    pub fn metadata_owner<'n>(&self, node: &AstNode, entry: &'n WitMetadataEntry) -> Option<&'n str> {
        if !self.native_valid(node) || !node.metadata.iter().any(|current|std::ptr::eq(current,entry)) {
            return None;
        }
        let owner=entry.owner_component_id.as_deref()?;
        self.writers.get(&node.id)?.contains(owner).then_some(owner)
    }
    /// Public-data schemas have no owner field; the actual unique accepted
    /// provider is available only through this private native proof chain.
    pub fn unique_issuer(&self, node: &AstNode) -> Option<&str> {
        if !self.native_valid(node) {
            return None;
        }
        let writers = self.writers.get(&node.id)?;
        if writers.len() != 1 {
            return None;
        }
        writers.first().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::{
        AstContextOrigin, MatchingPayload, MatchingScope, MatchingStatus, MatchingTiming,
        SyntaxKind, WitTextRange,
    };

    pub(super) fn node(id: u64) -> AstNode {
        AstNode {
            id,
            kind: SyntaxKind::Expression,
            syntax_id: format!("fixture:{id}"),
            text: format!("node {id}"),
            span: super::super::MappedSpan {
                virtual_range: WitTextRange {
                    start: id,
                    end: id + 1,
                },
                origins: Vec::new(),
            },
            syntax_context: 0,
            context_origin: AstContextOrigin::Preserved,
            summary: None,
            captures: Vec::new(),
            children: Vec::new(),
            function: None,
            metadata: Vec::new(),
        }
    }

    pub(super) fn payload() -> HookPayload {
        HookPayload::Matching(MatchingPayload {
            input: "value".into(),
            pattern: None,
            definition_id: "definition".into(),
            registration_id: "registration".into(),
            pattern_index: None,
            element_path: Vec::new(),
            pattern_span: None,
            scope: MatchingScope::Pattern,
            timing: MatchingTiming::After,
            input_range: WitTextRange { start: 0, end: 5 },
            span: node(0).span,
            status: MatchingStatus::Matched,
            failure_reason: None,
            metadata: Vec::new(),
        })
    }

    pub(super) fn write(registry: &mut ReceiptRegistry, node: &mut AstNode, issuer: &str) -> HookCall {
        let mut payload = payload();
        if let HookPayload::Matching(value) = &mut payload {
            value.metadata = node.metadata.clone();
        }
        let mut writers = registry.begin(&payload);
        let receipt = registry.accept(
            &mut writers,
            &mut payload,
            issuer,
            HookMode::Transform,
            SemanticEffects::None,
        );
        if let HookPayload::Matching(value) = payload {
            node.metadata = value.metadata;
        }
        HookCall {
            origin: HookCallOrigin::Executed,
            component_id: issuer.into(),
            subscription_id: "test".into(),
            semantic_receipt: receipt,
        }
    }

    #[test]
    fn body_receipts_keep_closed_adoption_and_host_lifetime_identity() {
        let sequence = Arc::default();
        let mut first = ReceiptRegistry::with_sequence(Arc::clone(&sequence));
        let mut value = node(0);
        let a = write(&mut first, &mut value, "a");
        let b = write(&mut first, &mut value, "b");
        let metadata = value.metadata.iter().map(|m| (m.key.clone(),m.value.clone())).collect::<Vec<_>>();
        assert!(first.export_body(&[b.clone()], &metadata).is_none(), "no implicit import of an earlier parent");
        let bundle = first.export_body(&[a.clone(),b.clone()], &metadata).unwrap();
        let mut current = ReceiptRegistry::with_sequence(sequence);
        let fresh = write(&mut current, &mut node(1), "fresh");
        assert!(fresh.semantic_receipt > b.semantic_receipt);
        assert!(current.can_import_body(&bundle));
        assert!(current.import_body(&bundle));
        assert!(!current.can_import_body(&bundle), "cannot overwrite current authority");
        assert!(!ReceiptRegistry::default().can_import_body(&bundle), "numeric IDs are not authority in another host");
        let mut forged = metadata;
        forged.push((RECEIPT.into(), "999999".into()));
        assert!(first.export_body(&[a,b], &forged).is_none());
    }

    #[test]
    fn unbound_host_mutation_taints_other_roots_without_rewriting_their_metadata() {
        let mut registry = ReceiptRegistry::default();
        let mut payload = payload();
        let mut writers = registry.begin(&payload);
        let observer = registry.accept(
            &mut writers,
            &mut payload,
            "state-observer",
            HookMode::Observe,
            SemanticEffects::Unbound,
        );
        let mut first = node(0);
        if let HookPayload::Matching(value) = payload {
            first.metadata = value.metadata;
        }
        let mut second = node(1);
        let provider = write(&mut registry, &mut second, "provider");
        let tree = AstTree {
            roots: vec![0, 1],
            nodes: vec![first, second],
        };
        let proof = SemanticProofRegistry::capture(
            &tree,
            Some(&registry),
            &[
                HookCall {
                    origin: HookCallOrigin::Executed,
                    component_id: "state-observer".into(),
                    subscription_id: "state".into(),
                    semantic_receipt: observer,
                },
                provider,
            ],
        );
        let view = proof.view(&tree);
        assert!(view.native_valid(&tree.nodes[1]));
        assert!(!view.provider_valid(&tree.nodes[1], "provider"));
    }

    #[test]
    fn editing_metadata_authorship_is_distinct_from_single_issuer_semantic_proof() {
        let mut registry=ReceiptRegistry::default();let mut root=node(0);
        let first=write(&mut registry,&mut root,"core");
        root.metadata.push(WitMetadataEntry {key:"editor.proposals".into(),value:"names".into(),owner_component_id:Some("provider".into())});
        root.metadata.push(WitMetadataEntry {key:"editor.foreign".into(),value:"forged".into(),owner_component_id:Some("foreign".into())});
        let second=write(&mut registry,&mut root,"provider");
        let tree=AstTree {roots:vec![0],nodes:vec![root]};
        let proof=SemanticProofRegistry::capture(&tree,Some(&registry),&[first.clone(),second]);
        let view=proof.view(&tree);let root=&tree.nodes[0];
        let entry=root.metadata.iter().find(|m|m.key=="editor.proposals").unwrap();
        assert!(view.native_valid(root));assert!(!view.provider_valid(root,"provider"));
        assert_eq!(view.metadata_owner(root,entry),Some("provider"));
        assert_eq!(view.metadata_owner(root,&entry.clone()),None);
        assert_eq!(view.metadata_owner(&root.clone(),entry),None);
        let foreign=root.metadata.iter().find(|m|m.key=="editor.foreign").unwrap();
        assert_eq!(view.metadata_owner(root,foreign),None);
        for mode in 0..3 {
            let mut changed=tree.clone();let index=changed.nodes[0].metadata.iter().position(|m|m.key=="editor.proposals").unwrap();
            match mode {0=>changed.nodes[0].metadata[index].value.push_str(" changed"),1=>changed.nodes[0].metadata[index].owner_component_id=Some("foreign".into()),
                _=>changed.nodes[0].span.virtual_range.end+=1}
            assert_eq!(proof.view(&changed).metadata_owner(&changed.nodes[0],&changed.nodes[0].metadata[index]),None);
        }
        let rejected=SemanticProofRegistry::capture(&tree,Some(&registry),&[first]);
        assert_eq!(rejected.view(&tree).metadata_owner(root,entry),None,"an unadopted/rolled-back writer grants no editing authority");
    }

    #[test]
    fn observe_context_or_additional_parse_effects_are_semantic_contributors() {
        let mut registry = ReceiptRegistry::default();
        let mut payload = payload();
        let mut writers = registry.begin(&payload);
        let observer = registry.accept(
            &mut writers,
            &mut payload,
            "context-observer",
            HookMode::Observe,
            SemanticEffects::Unbound,
        );
        assert!(observer.is_some());
        let mut root = node(0);
        if let HookPayload::Matching(value) = payload {
            root.metadata = value.metadata;
        }
        let provider = write(&mut registry, &mut root, "provider");
        let tree = AstTree {
            roots: vec![0],
            nodes: vec![root],
        };
        let proof = SemanticProofRegistry::capture(
            &tree,
            Some(&registry),
            &[
                HookCall {
                    origin: HookCallOrigin::Executed,
                    component_id: "context-observer".into(),
                    subscription_id: "context".into(),
                    semantic_receipt: observer,
                },
                provider,
            ],
        );
        assert!(!proof.view(&tree).provider_valid(&tree.nodes[0], "provider"));
    }

    #[test]
    fn provider_requires_all_adopted_writers_before_and_after_its_fact() {
        for order in [["foreign", "provider"], ["provider", "foreign"]] {
            let mut registry = ReceiptRegistry::default();
            let mut root = node(0);
            let calls: Vec<_> = order
                .into_iter()
                .map(|issuer| write(&mut registry, &mut root, issuer))
                .collect();
            let tree = AstTree {
                roots: vec![0],
                nodes: vec![root],
            };
            let proof = SemanticProofRegistry::capture(&tree, Some(&registry), &calls);
            let view = proof.view(&tree);
            assert!(view.native_valid(&tree.nodes[0]));
            assert!(!view.provider_valid(&tree.nodes[0], "provider"));
            assert!(view.unique_issuer(&tree.nodes[0]).is_none());
        }
    }

    #[test]
    fn empty_root_and_written_intermediate_propagate_to_a_deeper_leaf() {
        let mut registry = ReceiptRegistry::default();
        let mut root = node(0);
        root.children = vec![1];
        let mut middle = node(1);
        middle.children = vec![2];
        let mut leaf = node(2);
        let calls = vec![
            write(&mut registry, &mut middle, "foreign"),
            write(&mut registry, &mut leaf, "provider"),
        ];
        let tree = AstTree {
            roots: vec![0],
            nodes: vec![root, middle, leaf],
        };
        let proof = SemanticProofRegistry::capture(&tree, Some(&registry), &calls);
        assert!(!proof.view(&tree).provider_valid(&tree.nodes[2], "provider"));
    }

    #[test]
    fn transient_and_observe_calls_do_not_become_adopted_writers() {
        let mut registry = ReceiptRegistry::default();
        let mut unused = node(99);
        let _discarded = write(&mut registry, &mut unused, "foreign");
        let mut root = node(0);
        let accepted = write(&mut registry, &mut root, "provider");
        let mut observed_payload = payload();
        let mut writers = registry.begin(&observed_payload);
        assert!(
            registry
                .accept(
                    &mut writers,
                    &mut observed_payload,
                    "observer",
                    HookMode::Observe,
                    SemanticEffects::None
                )
                .is_none()
        );
        let tree = AstTree {
            roots: vec![0],
            nodes: vec![root],
        };
        let proof = SemanticProofRegistry::capture(
            &tree,
            Some(&registry),
            &[
                accepted,
                HookCall {
                    origin: HookCallOrigin::Executed,
                    component_id: "observer".into(),
                    subscription_id: "observe".into(),
                    semantic_receipt: None,
                },
            ],
        );
        assert!(proof.view(&tree).provider_valid(&tree.nodes[0], "provider"));
    }

    #[test]
    fn unbound_accepted_matching_writer_is_conservative_document_dependency() {
        let mut registry = ReceiptRegistry::default();
        let mut root = node(0);
        let accepted = write(&mut registry, &mut root, "provider");
        let mut unbound = node(99);
        let matching = write(&mut registry, &mut unbound, "foreign");
        let tree = AstTree {
            roots: vec![0],
            nodes: vec![root],
        };
        let proof = SemanticProofRegistry::capture(&tree, Some(&registry), &[accepted, matching]);
        assert!(!proof.view(&tree).provider_valid(&tree.nodes[0], "provider"));
    }

    #[test]
    fn complete_native_forest_proof_covers_summaries_captures_order_and_children() {
        let mut registry = ReceiptRegistry::default();
        let mut root = node(0);
        root.children = vec![1, 2];
        root.captures.push(super::super::Capture {
            name: "slot".into(),
            value: CaptureValue::Node(1),
        });
        let accepted = write(&mut registry, &mut root, "provider");
        let tree = AstTree {
            roots: vec![0],
            nodes: vec![root, node(1), node(2)],
        };
        let proof = SemanticProofRegistry::capture(&tree, Some(&registry), &[accepted]);
        assert!(proof.view(&tree).provider_valid(&tree.nodes[1], "provider"));
        for mode in 0..7 {
            let mut changed = tree.clone();
            match mode {
                0 => changed.nodes[1].text.push_str(" changed"),
                1 => changed.nodes[0].children.reverse(),
                2 => changed.nodes[0].captures[0].value = CaptureValue::Node(2),
                3 => changed.nodes[1].syntax_context += 1,
                4 => changed.nodes[1].metadata.push(WitMetadataEntry {
                    owner_component_id: Some("provider".into()),
                    key: "retained-fact".into(),
                    value: "changed".into(),
                }),
                5 => changed.nodes[1].span.virtual_range.end += 1,
                6 => changed.nodes[1].syntax_id.push_str(" changed"),
                _ => unreachable!(),
            }
            assert!(
                !proof.view(&changed).native_valid(&changed.nodes[0]),
                "mutation {mode}"
            );
        }
    }

    #[test]
    fn arena_renumbering_preserves_proof_and_editor_recovery_never_gains_it() {
        let mut registry = ReceiptRegistry::default();
        let mut root = node(0);
        root.children = vec![1];
        let accepted = write(&mut registry, &mut root, "provider");
        let tree = AstTree {
            roots: vec![0],
            nodes: vec![root, node(1)],
        };
        let proof = SemanticProofRegistry::capture(&tree, Some(&registry), &[accepted]);
        let mut current = tree.clone();
        current.roots = vec![50];
        current.nodes[0].id = 50;
        current.nodes[0].children = vec![70];
        current.nodes[1].id = 70;
        current.nodes.reverse();
        assert!(
            proof
                .view(&current)
                .provider_valid(&current.nodes[0], "provider")
        );
        let mut recovered = node(90);
        recovered.metadata.push(WitMetadataEntry {
            owner_component_id: None,
            key: "parser.editor-recovery".into(),
            value: "invalid-structure-entries".into(),
        });
        current.roots.push(90);
        current.nodes.push(recovered);
        let view = proof.view(&current);
        assert!(view.provider_valid(&current.nodes[0], "provider"));
        assert!(!view.native_valid(current.nodes.last().unwrap()));
    }

    #[test]
    fn forged_unretained_and_mutated_receipts_fail_closed() {
        let mut registry = ReceiptRegistry::default();
        let mut root = node(0);
        let accepted = write(&mut registry, &mut root, "provider");
        let tree = AstTree {
            roots: vec![0],
            nodes: vec![root],
        };
        let proof = SemanticProofRegistry::capture(&tree, Some(&registry), &[]);
        assert!(!proof.view(&tree).native_valid(&tree.nodes[0]));
        let proof = SemanticProofRegistry::capture(&tree, Some(&registry), &[accepted]);
        let mut changed = tree.clone();
        changed.nodes[0].metadata[0].value = "999999999".into();
        assert!(!proof.view(&changed).native_valid(&changed.nodes[0]));
        assert!(
            !SemanticProofRegistry::capture(&changed, Some(&registry), &[])
                .view(&changed)
                .native_valid(&changed.nodes[0])
        );
        let mut cyclic = tree.clone();
        cyclic.nodes[0].children.push(0);
        assert!(!proof.view(&cyclic).native_valid(&cyclic.nodes[0]));
    }
    fn write_with_nested(
        registry: &mut ReceiptRegistry,
        node: &mut AstNode,
        issuer: &str,
        nested: &[HookCall],
    ) -> HookCall {
        let mut payload = payload();
        if let HookPayload::Matching(value) = &mut payload {
            value.metadata = node.metadata.clone();
        }
        let mut writers = registry.begin(&payload);
        registry.admit_nested(&mut writers, nested);
        let receipt = registry.accept(
            &mut writers,
            &mut payload,
            issuer,
            HookMode::Transform,
            SemanticEffects::Unbound,
        );
        if let HookPayload::Matching(value) = payload {
            node.metadata = value.metadata;
        }
        HookCall {
            origin: HookCallOrigin::Executed,
            component_id: issuer.into(),
            subscription_id: "test".into(),
            semantic_receipt: receipt,
        }
    }

    #[test]
    fn accepted_nested_parse_calls_retain_their_exact_recursive_private_provenance() {
        let mut registry = ReceiptRegistry::default();
        let mut leaf = node(2);
        let leaf_call = write(&mut registry, &mut leaf, "provider");
        let mut middle = node(1);
        middle.children = vec![2];
        let middle_call = write_with_nested(&mut registry, &mut middle, "provider", &[leaf_call]);
        let mut root = node(0);
        root.children = vec![1];
        let root_call = write_with_nested(&mut registry, &mut root, "provider", &[middle_call]);
        let tree = AstTree {
            roots: vec![0],
            nodes: vec![root, middle, leaf],
        };
        let proof = SemanticProofRegistry::capture(&tree, Some(&registry), &[root_call]);
        let view = proof.view(&tree);
        for node in &tree.nodes {
            assert!(view.native_valid(node));
            assert!(view.provider_valid(node, "provider"));
        }
        let copied = tree.nodes[2].clone();
        assert!(!view.native_valid(&copied));
    }

    #[test]
    fn ordinary_metadata_ancestry_or_unadopted_nested_attempt_never_grants_membership() {
        let mut registry = ReceiptRegistry::default();
        let mut leaf = node(1);
        let _attempted_call = write(&mut registry, &mut leaf, "provider");
        let mut root = node(0);
        root.children = vec![1];
        // Even authentic metadata-parent ancestry does not replace admission
        // from the actual successful native continuation's retained call list.
        root.metadata = leaf.metadata.clone();
        let root_call = write(&mut registry, &mut root, "provider");
        let tree = AstTree {
            roots: vec![0],
            nodes: vec![root, leaf],
        };
        let proof = SemanticProofRegistry::capture(&tree, Some(&registry), &[root_call]);
        assert!(!proof.view(&tree).native_valid(&tree.nodes[1]));
    }

    #[test]
    fn accepted_unreferenced_foreign_nested_effect_remains_a_semantic_contributor() {
        let mut registry = ReceiptRegistry::default();
        let mut unused = node(99);
        let foreign_call = write(&mut registry, &mut unused, "foreign");
        let mut root = node(0);
        let root_call = write_with_nested(&mut registry, &mut root, "provider", &[foreign_call]);
        let tree = AstTree {
            roots: vec![0],
            nodes: vec![root],
        };
        let proof = SemanticProofRegistry::capture(&tree, Some(&registry), &[root_call]);
        let view = proof.view(&tree);
        assert!(view.native_valid(&tree.nodes[0]));
        assert!(!view.provider_valid(&tree.nodes[0], "provider"));
        assert!(view.unique_issuer(&tree.nodes[0]).is_none());
    }

    #[test]
    fn unknown_or_misattributed_nested_call_receipts_fail_closed() {
        for token_exists in [false, true] {
            let mut registry = ReceiptRegistry::default();
            let mut unused = node(99);
            let mut forged = write(&mut registry, &mut unused, "foreign");
            if token_exists {
                forged.component_id = "provider".into();
            } else {
                forged.semantic_receipt = Some(u64::MAX);
            }
            let mut root = node(0);
            let call = write_with_nested(&mut registry, &mut root, "provider", &[forged]);
            let tree = AstTree {
                roots: vec![0],
                nodes: vec![root],
            };
            let proof = SemanticProofRegistry::capture(&tree, Some(&registry), &[call]);
            assert!(!proof.view(&tree).native_valid(&tree.nodes[0]));
        }
    }
}

//! Bounded, immutable Tree macro imports. Semantic interpretation stays in guests.
use super::*;
use crate::bindings::nlaocs::skript_parser_addon::{tree_data as wire, types::*};

fn access_error(error: tree_view::AccessError) -> TreeError {
    use tree_view::AccessError as E;
    match error {
        E::Stale => TreeError::Stale,
        E::Cancelled => TreeError::Cancelled,
        E::QueryQuota => TreeError::QueryQuota,
        E::ByteQuota => TreeError::ByteQuota,
        E::InvalidPage => TreeError::InvalidPage,
        E::TokenExhausted => TreeError::Unavailable,
    }
}

pub(super) fn reference(reference: tree_view::Reference) -> TreeReference {
    TreeReference { epoch_high: reference.epoch_high, epoch_low: reference.epoch_low,
        token: reference.token, generation: reference.generation }
}

pub(super) fn kind(kind: ParserRawNodeKind) -> WitRawNodeKind {
    match kind {
        ParserRawNodeKind::Blank => WitRawNodeKind::Blank,
        ParserRawNodeKind::Comment => WitRawNodeKind::Comment,
        ParserRawNodeKind::Simple => WitRawNodeKind::Simple,
        ParserRawNodeKind::Section => WitRawNodeKind::Section,
        ParserRawNodeKind::Invalid => WitRawNodeKind::Invalid,
    }
}

pub(super) fn header(node: &skript_parser::RawNode) -> NodeHeader {
    NodeHeader { id: node.id.get(), kind: kind(node.kind), parent: node.parent.map(|id| id.get()),
        syntax_context: u64::from(node.syntax_context.get()), child_count: node.children.len() as u64 }
}

fn summary(node: &skript_parser::RawNode) -> NodeSummary {
    NodeSummary { header: header(node), text: node.text.clone() }
}

fn node(tree: &ParserRawTree, id: u64) -> Option<&skript_parser::RawNode> {
    // RawNodeId::index expects a native index. Guard arbitrary guest u64 input,
    // including on 32-bit hosts, before invoking the arena accessor.
    (id < tree.nodes.len() as u64).then(|| tree.get(ParserRawNodeId::new(id))).flatten()
}

// Conservative upper bounds include alignment/record/list/option overhead and
// every nested dynamic field. Check before cloning or lowering an owned value.
fn span_size(span: &skript_parser::MappedSpan) -> usize {
    64usize.saturating_add(span.origins.len().saturating_mul(64))
}
fn add(size: &mut usize, more: usize) -> Result<(), TreeError> {
    *size = size.saturating_add(more);
    if *size > tree_view::MAX_REPLY_BYTES { Err(TreeError::ByteQuota) } else { Ok(()) }
}
fn detail_size(node: &skript_parser::RawNode) -> Result<usize, TreeError> {
    let mut size = 512;
    add(&mut size, node.text.len())?;
    for span in [&node.span, &node.line.span, &node.line.content_span,
        &node.line.line_ending_span, &node.line.indentation.span] { add(&mut size, span_size(span))?; }
    for span in [&node.code_span, &node.header_span, &node.body_span].into_iter().flatten() {
        add(&mut size, span_size(span))?;
    }
    add(&mut size, node.line.raw_text.len())?;
    add(&mut size, node.line.indentation.text.len())?;
    for trivia in &node.line.trailing_trivia {
        add(&mut size, 128usize.saturating_add(trivia.text.len()).saturating_add(span_size(&trivia.span)))?;
    }
    Ok(size)
}
fn diagnostic_size(value: &skript_parser::RawDiagnostic) -> Result<usize, TreeError> {
    let mut size = 256;
    add(&mut size, value.message.len())?;
    add(&mut size, span_size(&value.span))?;
    for related in &value.related {
        add(&mut size, 128usize.saturating_add(related.message.len()).saturating_add(span_size(&related.span)))?;
    }
    Ok(size)
}

impl StoreData {
    fn read_tree(&mut self, reference: TreeReference) -> Result<Arc<ParserRawTree>, TreeError> {
        self.note_initialization_host_call();
        if self.invocation.is_none() { return Err(TreeError::Unavailable); }
        self.tree_view.as_mut().ok_or(TreeError::Unavailable)?.read(tree_view::Reference {
            epoch_high: reference.epoch_high, epoch_low: reference.epoch_low,
            token: reference.token, generation: reference.generation,
        }).map_err(access_error)
    }
    fn reserve_tree_response(&mut self, bytes: usize) -> Result<(), TreeError> {
        self.tree_view.as_mut().ok_or(TreeError::Unavailable)?
            .reserve_response(bytes).map_err(access_error)
    }
    fn node_page(&mut self, tree: &ParserRawTree, ids: &[ParserRawNodeId], cursor: u64, limit: u32)
        -> Result<NodePage, TreeError> {
        let (ids, count_next) = tree_view::page(ids, cursor, limit).map_err(access_error)?;
        self.reserve_tree_response(64usize.saturating_add(ids.len().saturating_mul(128)))?;
        let mut nodes = Vec::with_capacity(ids.len());
        for id in ids {
            nodes.push(header(node(tree, id.get()).ok_or(TreeError::InvalidPage)?));
        }
        Ok(NodePage { nodes, next: count_next })
    }
}

impl wire::Host for StoreData {
    fn info(&mut self, reference: TreeReference) -> Result<TreeInfo, TreeError> {
        let tree = self.read_tree(reference)?;
        self.reserve_tree_response(256usize.saturating_add(tree.indentation.as_ref().map_or(0, |i| i.unit.len())))?;
        Ok(TreeInfo { node_count: tree.nodes.len() as u64, root_count: tree.roots.len() as u64,
            diagnostic_count: tree.diagnostics.len() as u64, indentation: indentation(&tree) })
    }
    fn node(&mut self, reference: TreeReference, id: u64) -> Result<Option<NodeSummary>, TreeError> {
        let tree = self.read_tree(reference)?;
        let Some(node) = node(&tree, id) else { self.reserve_tree_response(64)?; return Ok(None); };
        self.reserve_tree_response(512usize.saturating_add(node.text.len()))?;
        Ok(Some(summary(node)))
    }
    fn details(&mut self, reference: TreeReference, id: u64) -> Result<Option<NodeDetails>, TreeError> {
        let tree = self.read_tree(reference)?;
        let Some(node) = node(&tree, id) else { self.reserve_tree_response(64)?; return Ok(None); };
        self.reserve_tree_response(detail_size(node)?)?;
        Ok(Some(details(node)))
    }
    fn span(&mut self, reference: TreeReference, id: u64, kind: NodeSpanKind) -> Result<Option<MappedSpan>, TreeError> {
        let tree = self.read_tree(reference)?;
        let selected = node(&tree, id).and_then(|node| match kind {
            NodeSpanKind::Node => Some(&node.span), NodeSpanKind::Code => node.code_span.as_ref(),
            NodeSpanKind::Header => node.header_span.as_ref(), NodeSpanKind::Body => node.body_span.as_ref(),
        });
        self.reserve_tree_response(selected.map_or(64, span_size))?;
        Ok(selected.map(|span| mapped_span_to_wit(span.clone())))
    }
    fn roots(&mut self, reference: TreeReference, cursor: u64, limit: u32) -> Result<NodePage, TreeError> {
        let tree = self.read_tree(reference)?;
        self.node_page(&tree, &tree.roots, cursor, limit)
    }
    fn children(&mut self, reference: TreeReference, parent: u64, cursor: u64, limit: u32)
        -> Result<Option<NodePage>, TreeError> {
        let tree = self.read_tree(reference)?;
        let Some(node) = node(&tree, parent) else { self.reserve_tree_response(64)?; return Ok(None); };
        self.node_page(&tree, &node.children, cursor, limit).map(Some)
    }
    fn diagnostics(&mut self, reference: TreeReference, cursor: u64, limit: u32) -> Result<DiagnosticPage, TreeError> {
        let tree = self.read_tree(reference)?;
        let start = usize::try_from(cursor).map_err(|_| TreeError::InvalidPage)?;
        if start > tree.diagnostics.len() || limit == 0 || limit as usize > tree_view::MAX_PAGE {
            return Err(TreeError::InvalidPage);
        }
        let end = start.saturating_add(limit as usize).min(tree.diagnostics.len());
        let mut size = 64usize;
        let mut count = 0;
        for diagnostic in &tree.diagnostics[start..end] {
            let item_size = match diagnostic_size(diagnostic) {
                Ok(size) => size,
                Err(TreeError::ByteQuota) if count > 0 => break,
                Err(error) => return Err(error),
            };
            if size.saturating_add(item_size) > tree_view::MAX_REPLY_BYTES { break; }
            size += item_size; count += 1;
        }
        if count == 0 && start != end { return Err(TreeError::ByteQuota); }
        self.reserve_tree_response(size)?;
        let end = start + count;
        Ok(DiagnosticPage { diagnostics: tree.diagnostics[start..end].iter().map(raw_diagnostic).collect(),
            next: (end < tree.diagnostics.len()).then_some(end as u64) })
    }
}

pub(super) fn indentation(tree: &ParserRawTree) -> Option<Indentation> {
    tree.indentation.as_ref().map(|value| Indentation {
        kind: match value.kind { ParserIndentKind::Space => WitIndentKind::Space, ParserIndentKind::Tab => WitIndentKind::Tab },
        unit: value.unit.clone(),
    })
}

fn trivia(value: &skript_parser::RawTrivia) -> RawTrivia {
    RawTrivia { kind: match value.kind {
        ParserRawTriviaKind::Whitespace => WitRawTriviaKind::Whitespace,
        ParserRawTriviaKind::LineComment => WitRawTriviaKind::LineComment,
        ParserRawTriviaKind::BlockComment => WitRawTriviaKind::BlockComment,
        ParserRawTriviaKind::LineEnding => WitRawTriviaKind::LineEnding,
    }, text: value.text.clone(), span: mapped_span_to_wit(value.span.clone()) }
}

pub(super) fn details(node: &skript_parser::RawNode) -> NodeDetails {
    NodeDetails { summary: summary(node), span: mapped_span_to_wit(node.span.clone()),
        line: RawLine { number: node.line.number as u64, raw_text: node.line.raw_text.clone(),
            line_ending: match node.line.line_ending { ParserLineEnding::None => WitLineEnding::None,
                ParserLineEnding::Lf => WitLineEnding::Lf, ParserLineEnding::CrLf => WitLineEnding::CrLf,
                ParserLineEnding::Cr => WitLineEnding::Cr },
            span: mapped_span_to_wit(node.line.span.clone()), content_span: mapped_span_to_wit(node.line.content_span.clone()),
            line_ending_span: mapped_span_to_wit(node.line.line_ending_span.clone()),
            indentation: trivia(&node.line.indentation),
            trailing_trivia: node.line.trailing_trivia.iter().map(trivia).collect(),
        },
        code_span: node.code_span.clone().map(mapped_span_to_wit),
        header_span: node.header_span.clone().map(mapped_span_to_wit),
        body_span: node.body_span.clone().map(mapped_span_to_wit), indent_level: node.indent_level,
        invalid_reason: node.invalid_reason.as_ref().map(|reason| match reason {
            ParserRawInvalidReason::MixedIndentation => WitRawInvalidReason::MixedIndentation,
            ParserRawInvalidReason::InvalidIndentation => WitRawInvalidReason::InvalidIndentation,
            ParserRawInvalidReason::UnexpectedIndentation { expected_level, actual_level } =>
                WitRawInvalidReason::UnexpectedIndentation(UnexpectedIndentation { expected_level: *expected_level, actual_level: *actual_level }),
        }),
    }
}

pub(super) fn raw_diagnostic(value: &skript_parser::RawDiagnostic) -> RawDiagnostic {
    RawDiagnostic { code: match value.code {
        ParserRawDiagnosticCode::MixedIndentation => WitRawDiagnosticCode::MixedIndentation,
        ParserRawDiagnosticCode::InvalidIndentation => WitRawDiagnosticCode::InvalidIndentation,
        ParserRawDiagnosticCode::UnexpectedIndentation => WitRawDiagnosticCode::UnexpectedIndentation,
        ParserRawDiagnosticCode::EmptySection => WitRawDiagnosticCode::EmptySection,
        ParserRawDiagnosticCode::UnclosedBlockComment => WitRawDiagnosticCode::UnclosedBlockComment,
    }, severity: match value.severity { ParserRawDiagnosticSeverity::Error => WitRawDiagnosticSeverity::Error,
        ParserRawDiagnosticSeverity::Warning => WitRawDiagnosticSeverity::Warning },
        message: value.message.clone(), span: mapped_span_to_wit(value.span.clone()),
        related: value.related.iter().map(|related| RawRelatedSpan {
            message: related.message.clone(), span: mapped_span_to_wit(related.span.clone()),
        }).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(source: &str) -> (Store<StoreData>, ParseTransaction, TreeReference, tempfile::TempDir) {
        let directory = tempfile::tempdir().unwrap();
        let states = StateStore::new(StateStoreConfig {
            data_directory: Some(directory.path().to_owned()), ..Default::default()
        }).unwrap();
        states.register_component("fixture.tree-reader", &[]).unwrap();
        let transaction = states.begin_parse("file:///public-tree-view", "file:///public-tree-view/test.sk", 1).unwrap();
        let tree = Arc::new(skript_parser::parse_raw_tree(&MappedSource::identity(source),
            skript_parser::RawTreeOptions::for_skript_version(2, 15)));
        let mut store = create_store(&Engine::default(), &HostConfig::default(), Arc::from([]));
        store.data_mut().initializing = false;
        store.data_mut().invocation = Some(transaction.begin_invocation("fixture.tree-reader").unwrap());
        let lease = tree_view::Lease::new(tree, 0, None).unwrap();
        let reference = reference(lease.reference());
        store.data_mut().tree_view = Some(lease);
        (store, transaction, reference, directory)
    }

    #[test]
    fn imports_preserve_lossless_data_and_reject_absent_or_expired_leases() {
        let (mut store, transaction, reference, _directory) = fixture("root:\r\n    child # note\r\n");
        let info = wire::Host::info(store.data_mut(), reference.clone()).unwrap();
        assert_eq!(info.node_count, 2);
        assert_eq!(info.indentation.unwrap().unit, "    ");
        let roots = wire::Host::roots(store.data_mut(), reference.clone(), 0, 128).unwrap();
        assert_eq!(roots.nodes[0].child_count, 1);
        assert_eq!(wire::Host::node(store.data_mut(), reference.clone(), roots.nodes[0].id).unwrap().unwrap().text, "root");
        let children = wire::Host::children(store.data_mut(), reference.clone(), 0, 0, 128).unwrap().unwrap();
        assert_eq!(wire::Host::node(store.data_mut(), reference.clone(), children.nodes[0].id).unwrap().unwrap().text, "child");
        let detail = wire::Host::details(store.data_mut(), reference.clone(), 1).unwrap().unwrap();
        assert_eq!(detail.line.raw_text, "    child # note");
        assert!(matches!(detail.line.line_ending, WitLineEnding::CrLf));
        assert!(detail.line.trailing_trivia.iter().any(|item| matches!(item.kind, WitRawTriviaKind::LineComment)));
        assert!(!detail.span.origins.is_empty());
        assert!(detail.code_span.is_some());
        assert!(wire::Host::node(store.data_mut(), reference.clone(), u64::MAX).unwrap().is_none());
        let old = store.data_mut().tree_view.take().unwrap();
        assert!(matches!(wire::Host::info(store.data_mut(), reference.clone()), Err(TreeError::Unavailable)));
        let snapshot = Arc::new(skript_parser::parse_raw_tree(&MappedSource::identity("replacement\n"),
            skript_parser::RawTreeOptions::for_skript_version(2, 15)));
        store.data_mut().tree_view = Some(tree_view::Lease::new(snapshot, 1, None).unwrap());
        assert!(matches!(wire::Host::info(store.data_mut(), reference), Err(TreeError::Stale)));
        drop(old);
        store.data_mut().tree_view.take();
        store.data_mut().invocation.take().unwrap().rollback();
        transaction.cancel().unwrap();
    }

    #[test]
    fn header_pages_skip_unrequested_text_and_oversized_details_are_explicit() {
        let text = (0..10).map(|_| format!("{}\n", "x".repeat(9000))).collect::<String>();
        let (mut store, transaction, reference, _directory) = fixture(&text);
        let first = wire::Host::roots(store.data_mut(), reference.clone(), 0, 6).unwrap();
        assert!(!first.nodes.is_empty() && first.nodes.len() < 10);
        let next = first.next.unwrap();
        let second = wire::Host::roots(store.data_mut(), reference.clone(), next, 128).unwrap();
        assert_eq!(first.nodes.len() + second.nodes.len(), 10);
        assert!(second.next.is_none());
        assert!(matches!(wire::Host::roots(store.data_mut(), reference.clone(), 0, 129), Err(TreeError::InvalidPage)));
        store.data_mut().tree_view.take();
        store.data_mut().invocation.take().unwrap().rollback(); transaction.cancel().unwrap();
        let huge = format!("{}\n", "x".repeat(tree_view::MAX_REPLY_BYTES));
        let (mut store, transaction, reference, _directory) = fixture(&huge);
        assert_eq!(wire::Host::roots(store.data_mut(), reference.clone(), 0, 128).unwrap().nodes.len(), 1);
        assert!(matches!(wire::Host::node(store.data_mut(), reference.clone(), 0), Err(TreeError::ByteQuota)));
        assert!(matches!(wire::Host::details(store.data_mut(), reference, 0), Err(TreeError::ByteQuota)));
        store.data_mut().tree_view.take();
        store.data_mut().invocation.take().unwrap().rollback(); transaction.cancel().unwrap();
    }

    #[test]
    fn initialization_or_non_tree_calls_cannot_access_another_invocations_tree() {
        let mut store = create_store(&Engine::default(), &HostConfig::default(), Arc::from([]));
        let fake = TreeReference { epoch_high: 0, epoch_low: 0, token: 0, generation: 0 };
        assert!(matches!(wire::Host::info(store.data_mut(), fake.clone()), Err(TreeError::Unavailable)));
        assert!(store.data().initialization_host_call_attempted);
        store.data_mut().initializing = false;
        assert!(matches!(wire::Host::info(store.data_mut(), fake), Err(TreeError::Unavailable)));
    }
}

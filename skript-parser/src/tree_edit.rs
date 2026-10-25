//! Validated replacement operations over a lossless `RawTree`.
//!
//! Addon-provided fragments use local IDs. The host allocates stable node,
//! expansion, and syntax-context identities only after the entire edit validates.
#![allow(missing_docs)] // Type-level docs describe aggregate field contracts.

use crate::{
    ExpansionId, LineEnding, MappedSource, MappedSpan, OriginKind, RawLine, RawNode, RawNodeId,
    RawNodeKind, RawTree, RawTrivia, RawTriviaKind, SourceMapError, SourceMapSegment, SourceOrigin,
    SyntaxContextId, TextRange, TreeExpansion, TreeExpansionError,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Addon-local identity used only while validating a generated fragment.
pub struct GeneratedRawNodeId(u64);

impl GeneratedRawNodeId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for GeneratedRawNodeId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Allowed structural kind for an addon-generated raw node.
pub enum GeneratedRawNodeKind {
    Blank,
    Comment,
    Simple,
    Section,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One node in an addon-provided replacement fragment.
pub struct GeneratedRawNode {
    pub id: GeneratedRawNodeId,
    pub kind: GeneratedRawNodeKind,
    pub text: String,
    pub children: Vec<GeneratedRawNodeId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
/// Validated-input arena and roots supplied by a Tree macro.
pub struct GeneratedRawTree {
    pub roots: Vec<GeneratedRawNodeId>,
    pub nodes: Vec<GeneratedRawNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Where original Section children are attached to generated content.
pub enum RetainedChildrenPlacement {
    Prepend,
    Append,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Policy for preserving children of the replaced Section.
pub struct RetainedChildren {
    pub parent: GeneratedRawNodeId,
    pub placement: RetainedChildrenPlacement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Atomic structural operation targeting the current RawTree node.
pub enum TreeEdit {
    /// Byte edits in the current decoded code, retaining its native node,
    /// children and syntax context. Physical and decoded code must agree.
    EditCode { edits: Vec<crate::TextEdit> },
    ReplaceNode {
        replacement: GeneratedRawTree,
        retained_children: Option<RetainedChildren>,
    },
    ReplaceChildren {
        replacement: GeneratedRawTree,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Component and hook identity attached to generated tree provenance.
pub struct TreeEditMetadata {
    pub component: String,
    pub hook: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Updated tree/source provenance and generated root identities.
pub struct TreeEditApplication {
    pub source: MappedSource,
    pub tree: RawTree,
    pub expansion: Option<ExpansionId>,
    pub syntax_context: SyntaxContextId,
    pub replacement_roots: usize,
}

/// Validates and atomically applies one generated Tree macro operation.
///
/// Code edits retain the existing node context and exact untouched slices.
/// Empty/no-op code edit batches produce no expansion. Generated node IDs are
/// local to a replacement fragment. After complete
/// validation, the host assigns stable arena IDs, attaches a new expansion and
/// syntax context, and returns a new tree without mutating the input tree.
///
/// # Examples
///
/// ~~~
/// use skript_parser::{
///     apply_tree_edit, parse_raw_tree, GeneratedRawNode, GeneratedRawNodeId,
///     GeneratedRawNodeKind, GeneratedRawTree, MappedSource, RawTreeOptions,
///     TreeEdit, TreeEditMetadata,
/// };
///
/// let source = MappedSource::identity("replace me\n");
/// let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 15));
/// let target = tree.roots[0];
/// let generated_id = GeneratedRawNodeId::new(0);
///
/// let applied = apply_tree_edit(
///     &source,
///     &tree,
///     target,
///     TreeEdit::ReplaceNode {
///         replacement: GeneratedRawTree {
///             roots: vec![generated_id],
///             nodes: vec![GeneratedRawNode {
///                 id: generated_id,
///                 kind: GeneratedRawNodeKind::Simple,
///                 text: "broadcast \"generated\"".to_owned(),
///                 children: Vec::new(),
///             }],
///         },
///         retained_children: None,
///     },
///     TreeEditMetadata {
///         component: "example.addon".to_owned(),
///         hook: "expand-statement".to_owned(),
///     },
/// )?;
///
/// let replacement = applied.tree.get(applied.tree.roots[0]).unwrap();
/// assert_eq!(replacement.text, "broadcast \"generated\"");
/// assert!(replacement.span.is_generated());
/// assert_eq!(
///     applied.source.virtual_source(),
///     "replace me\nbroadcast \"generated\"\n",
/// );
/// assert_eq!(applied.replacement_roots, 1);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ~~~
///
/// # Errors
///
/// Returns [TreeEditError] if the target is missing, a fragment ID is
/// duplicated or unresolved, generated structure is invalid, retained children
/// target a non-section, or expansion provenance cannot be registered. The
/// original source and tree remain unchanged.
pub fn apply_tree_edit(
    source: &MappedSource,
    tree: &RawTree,
    target: RawNodeId,
    edit: TreeEdit,
    metadata: TreeEditMetadata,
) -> Result<TreeEditApplication, TreeEditError> {
    if let TreeEdit::EditCode { edits } = edit {
        return apply_code_edit(source, tree, target, edits, metadata);
    }
    let fragment = match &edit {
        TreeEdit::EditCode { .. } => unreachable!(),
        TreeEdit::ReplaceNode { replacement, .. } | TreeEdit::ReplaceChildren { replacement } => {
            replacement
        }
    };
    let fragment_index = validate_fragment(fragment)?;
    let mut forest = inflate_tree(tree)?;
    let target_node = find_subtree(&forest, target)
        .ok_or(TreeEditError::UnknownTarget { target })?
        .node
        .clone();

    if matches!(edit, TreeEdit::ReplaceChildren { .. }) && target_node.kind != RawNodeKind::Section
    {
        return Err(TreeEditError::ChildrenOfNonSection { target });
    }

    let expansion = source.register_tree_expansion(
        &target_node.span,
        TreeExpansion::new(metadata.component, metadata.hook),
    )?;
    let span = generated_span(&target_node.span, expansion.expansion);
    let point = generated_point(&target_node.span, expansion.expansion);
    let generated = build_generated_forest(
        fragment,
        &fragment_index,
        &span,
        &point,
        target_node.line.number,
        expansion.syntax_context,
    );
    let replacement_roots = generated.len();

    match edit {
        TreeEdit::ReplaceNode {
            retained_children, ..
        } => {
            if !replace_node(&mut forest, target, generated, retained_children)? {
                return Err(TreeEditError::UnknownTarget { target });
            }
        }
        TreeEdit::ReplaceChildren { .. } => {
            if !replace_children(&mut forest, target, generated) {
                return Err(TreeEditError::UnknownTarget { target });
            }
        }
        TreeEdit::EditCode { .. } => unreachable!(),
    }

    let projected_source = project_forest(
        &expansion.source,
        &mut forest,
        &target_node.span,
        expansion.expansion,
    )?;

    Ok(TreeEditApplication {
        source: projected_source,
        tree: flatten_tree(forest, tree),
        expansion: Some(expansion.expansion),
        syntax_context: expansion.syntax_context,
        replacement_roots,
    })
}

fn apply_code_edit(
    source: &MappedSource, tree: &RawTree, target: RawNodeId,
    mut edits: Vec<crate::TextEdit>, metadata: TreeEditMetadata,
) -> Result<TreeEditApplication, TreeEditError> {
    let invalid = |reason: &str| TreeEditError::InvalidCodeEdit(reason.to_owned());
    let node = tree.get(target).ok_or(TreeEditError::UnknownTarget { target })?;
    if !matches!(node.kind, RawNodeKind::Simple | RawNodeKind::Section) {
        return Err(invalid("target is not code"));
    }
    let code = node.code_span.as_ref().ok_or_else(|| invalid("missing code span"))?;
    if code.virtual_range.start < node.line.content_span.virtual_range.start
        || code.virtual_range.end > node.line.content_span.virtual_range.end
        || node.line.span.virtual_range.start > node.line.content_span.virtual_range.start
        || node.line.content_span.virtual_range.end > node.line.span.virtual_range.end
    {
        return Err(invalid("code is not contained in its native line"));
    }
    let physical = code.virtual_range.slice(source.virtual_source())
        .ok_or_else(|| invalid("code span is outside the active source"))?;
    let physical = if node.kind == RawNodeKind::Section {
        physical.strip_suffix(':').ok_or_else(|| invalid("section colon is absent"))?
    } else { physical };
    if physical != node.text || node.line.content_span.virtual_range.slice(source.virtual_source()) != Some(node.line.raw_text.as_str()) {
        return Err(invalid("decoded and physical code are not byte-identical"));
    }
    edits.sort_by_key(|edit| (edit.range.start, edit.range.end));
    let mut end = 0;
    for edit in &edits {
        if edit.anchor.is_some() || edit.range.is_empty() || !edit.range.is_valid_for(&node.text)
            || edit.range.start < end || edit.replacement.contains(['\r', '\n'])
        {
            return Err(invalid("edits must be disjoint nonempty UTF-8 code ranges without anchors or line breaks"));
        }
        end = edit.range.end;
    }
    edits.retain(|edit| edit.range.slice(&node.text) != Some(edit.replacement.as_str()));
    if edits.is_empty() {
        return Ok(TreeEditApplication {
            source: source.clone(), tree: tree.clone(), expansion: None,
            syntax_context: node.syntax_context, replacement_roots: 1,
        });
    }
    let mut text = String::new();
    let mut cursor = 0;
    for edit in &edits {
        text.push_str(&node.text[cursor..edit.range.start]);
        text.push_str(&edit.replacement);
        cursor = edit.range.end;
    }
    text.push_str(&node.text[cursor..]);
    if text.trim().is_empty() || node.kind == RawNodeKind::Section && text.trim_end().ends_with(':') {
        return Err(invalid("edited code does not preserve the node kind"));
    }
    let mut forest = inflate_tree(tree)?;
    let selected = find_subtree_mut(&mut forest, target).ok_or(TreeEditError::UnknownTarget { target })?;
    selected.code_edits = Some(edits);
    let expansion = source.register_tree_expansion(
        &node.span, TreeExpansion::new(metadata.component, metadata.hook),
    )?;
    let projected = project_forest(&expansion.source, &mut forest, &node.span, expansion.expansion)?;
    Ok(TreeEditApplication {
        source: projected, tree: flatten_tree(forest, tree), expansion: Some(expansion.expansion),
        syntax_context: node.syntax_context, replacement_roots: 1,
    })
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
/// Rejected generated tree, target, retained-child policy, or provenance operation.
pub enum TreeEditError {
    #[error("invalid node code edit: {0}")]
    InvalidCodeEdit(String),
    #[error("RawTree references unknown node {id}")]
    UnknownInputNode { id: RawNodeId },
    #[error("RawTree contains a parent/child cycle at node {id}")]
    InputCycle { id: RawNodeId },
    #[error("RawTree node {id} is reachable more than once")]
    ReusedInputNode { id: RawNodeId },
    #[error("RawTree node {id} is not reachable from a root")]
    UnreachableInputNode { id: RawNodeId },
    #[error("tree edit targets unknown node {target}")]
    UnknownTarget { target: RawNodeId },
    #[error("tree edit cannot replace children of non-Section node {target}")]
    ChildrenOfNonSection { target: RawNodeId },
    #[error("generated node ID {id} is declared more than once")]
    DuplicateGeneratedId { id: GeneratedRawNodeId },
    #[error("generated tree references unknown node {id}")]
    UnknownGeneratedNode { id: GeneratedRawNodeId },
    #[error("generated node {id} has more than one parent")]
    ReusedGeneratedNode { id: GeneratedRawNodeId },
    #[error("generated tree contains a cycle at node {id}")]
    GeneratedCycle { id: GeneratedRawNodeId },
    #[error("generated node {id} is not reachable from a root")]
    UnreachableGeneratedNode { id: GeneratedRawNodeId },
    #[error("generated {kind:?} node {id} must not have children")]
    ChildrenOfGeneratedLeaf {
        id: GeneratedRawNodeId,
        kind: GeneratedRawNodeKind,
    },
    #[error("generated {kind:?} node {id} has invalid text")]
    InvalidGeneratedText {
        id: GeneratedRawNodeId,
        kind: GeneratedRawNodeKind,
    },
    #[error("retained children target unknown generated node {id}")]
    UnknownRetainedChildrenParent { id: GeneratedRawNodeId },
    #[error("retained children target {id} is not a generated Section")]
    RetainedChildrenParentNotSection { id: GeneratedRawNodeId },
    #[error(transparent)]
    Expansion(#[from] TreeExpansionError),
    #[error(transparent)]
    SourceMap(#[from] SourceMapError),
}

#[derive(Debug, Clone)]
struct OwnedSubtree {
    local_id: Option<GeneratedRawNodeId>,
    node: RawNode,
    children: Vec<OwnedSubtree>,
    code_edits: Option<Vec<crate::TextEdit>>,
}

fn inflate_tree(tree: &RawTree) -> Result<Vec<OwnedSubtree>, TreeEditError> {
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut forest = Vec::with_capacity(tree.roots.len());
    for root in &tree.roots {
        forest.push(inflate_node(tree, *root, &mut visiting, &mut visited)?);
    }
    for node in &tree.nodes {
        if !visited.contains(&node.id) {
            return Err(TreeEditError::UnreachableInputNode { id: node.id });
        }
    }
    Ok(forest)
}

fn inflate_node(
    tree: &RawTree,
    id: RawNodeId,
    visiting: &mut BTreeSet<RawNodeId>,
    visited: &mut BTreeSet<RawNodeId>,
) -> Result<OwnedSubtree, TreeEditError> {
    if visited.contains(&id) {
        return Err(TreeEditError::ReusedInputNode { id });
    }
    if !visiting.insert(id) {
        return Err(TreeEditError::InputCycle { id });
    }
    let node = tree
        .get(id)
        .ok_or(TreeEditError::UnknownInputNode { id })?
        .clone();
    let mut children = Vec::with_capacity(node.children.len());
    for child in &node.children {
        children.push(inflate_node(tree, *child, visiting, visited)?);
    }
    visiting.remove(&id);
    visited.insert(id);
    Ok(OwnedSubtree {
        local_id: None,
        node,
        children,
        code_edits: None,
    })
}

fn validate_fragment(
    fragment: &GeneratedRawTree,
) -> Result<BTreeMap<GeneratedRawNodeId, usize>, TreeEditError> {
    let mut by_id = BTreeMap::new();
    for (index, node) in fragment.nodes.iter().enumerate() {
        if by_id.insert(node.id, index).is_some() {
            return Err(TreeEditError::DuplicateGeneratedId { id: node.id });
        }
        validate_generated_node(node)?;
    }

    let mut parent_count = BTreeMap::<GeneratedRawNodeId, usize>::new();
    for root in &fragment.roots {
        if !by_id.contains_key(root) {
            return Err(TreeEditError::UnknownGeneratedNode { id: *root });
        }
        *parent_count.entry(*root).or_default() += 1;
    }
    for node in &fragment.nodes {
        for child in &node.children {
            if !by_id.contains_key(child) {
                return Err(TreeEditError::UnknownGeneratedNode { id: *child });
            }
            *parent_count.entry(*child).or_default() += 1;
        }
    }
    if let Some((&id, _)) = parent_count.iter().find(|(_, count)| **count > 1) {
        return Err(TreeEditError::ReusedGeneratedNode { id });
    }

    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for root in &fragment.roots {
        validate_generated_acyclic(fragment, &by_id, *root, &mut visiting, &mut visited)?;
    }
    for node in &fragment.nodes {
        if !visited.contains(&node.id) {
            return Err(TreeEditError::UnreachableGeneratedNode { id: node.id });
        }
    }
    Ok(by_id)
}

fn validate_generated_node(node: &GeneratedRawNode) -> Result<(), TreeEditError> {
    if node.kind != GeneratedRawNodeKind::Section && !node.children.is_empty() {
        return Err(TreeEditError::ChildrenOfGeneratedLeaf {
            id: node.id,
            kind: node.kind,
        });
    }
    let invalid_text = node.text.contains(['\r', '\n'])
        || match node.kind {
            GeneratedRawNodeKind::Blank => !node.text.is_empty(),
            GeneratedRawNodeKind::Comment => false,
            GeneratedRawNodeKind::Simple => node.text.trim().is_empty(),
            GeneratedRawNodeKind::Section => {
                node.text.trim().is_empty() || node.text.trim_end().ends_with(':')
            }
        };
    if invalid_text {
        return Err(TreeEditError::InvalidGeneratedText {
            id: node.id,
            kind: node.kind,
        });
    }
    Ok(())
}

fn validate_generated_acyclic(
    fragment: &GeneratedRawTree,
    by_id: &BTreeMap<GeneratedRawNodeId, usize>,
    id: GeneratedRawNodeId,
    visiting: &mut BTreeSet<GeneratedRawNodeId>,
    visited: &mut BTreeSet<GeneratedRawNodeId>,
) -> Result<(), TreeEditError> {
    if visited.contains(&id) {
        return Ok(());
    }
    if !visiting.insert(id) {
        return Err(TreeEditError::GeneratedCycle { id });
    }
    let node = &fragment.nodes[by_id[&id]];
    for child in &node.children {
        validate_generated_acyclic(fragment, by_id, *child, visiting, visited)?;
    }
    visiting.remove(&id);
    visited.insert(id);
    Ok(())
}

fn build_generated_forest(
    fragment: &GeneratedRawTree,
    by_id: &BTreeMap<GeneratedRawNodeId, usize>,
    span: &MappedSpan,
    point: &MappedSpan,
    line_number: usize,
    syntax_context: SyntaxContextId,
) -> Vec<OwnedSubtree> {
    fragment
        .roots
        .iter()
        .map(|root| {
            build_generated_node(
                fragment,
                by_id,
                *root,
                span,
                point,
                line_number,
                syntax_context,
            )
        })
        .collect()
}

fn build_generated_node(
    fragment: &GeneratedRawTree,
    by_id: &BTreeMap<GeneratedRawNodeId, usize>,
    id: GeneratedRawNodeId,
    span: &MappedSpan,
    point: &MappedSpan,
    line_number: usize,
    syntax_context: SyntaxContextId,
) -> OwnedSubtree {
    let generated = &fragment.nodes[by_id[&id]];
    let children = generated
        .children
        .iter()
        .map(|child| {
            build_generated_node(
                fragment,
                by_id,
                *child,
                span,
                point,
                line_number,
                syntax_context,
            )
        })
        .collect();
    let kind = match generated.kind {
        GeneratedRawNodeKind::Blank => RawNodeKind::Blank,
        GeneratedRawNodeKind::Comment => RawNodeKind::Comment,
        GeneratedRawNodeKind::Simple => RawNodeKind::Simple,
        GeneratedRawNodeKind::Section => RawNodeKind::Section,
    };
    let raw_text = match generated.kind {
        GeneratedRawNodeKind::Blank => String::new(),
        GeneratedRawNodeKind::Section => format!("{}:", generated.text),
        GeneratedRawNodeKind::Comment | GeneratedRawNodeKind::Simple => generated.text.clone(),
    };
    let code_span = matches!(
        generated.kind,
        GeneratedRawNodeKind::Simple | GeneratedRawNodeKind::Section
    )
    .then(|| span.clone());
    let node = RawNode {
        id: RawNodeId::new(0),
        kind,
        text: generated.text.clone(),
        span: span.clone(),
        line: RawLine {
            number: line_number,
            raw_text,
            line_ending: LineEnding::Lf,
            span: span.clone(),
            content_span: span.clone(),
            line_ending_span: point.clone(),
            indentation: RawTrivia {
                kind: RawTriviaKind::Whitespace,
                text: String::new(),
                span: point.clone(),
            },
            trailing_trivia: Vec::new(),
        },
        code_span: code_span.clone(),
        header_span: (kind == RawNodeKind::Section).then_some(span.clone()),
        body_span: (kind == RawNodeKind::Section).then_some(point.clone()),
        indent_level: None,
        invalid_reason: None,
        syntax_context,
        parent: None,
        children: Vec::new(),
        original_extent: None,
    };
    OwnedSubtree {
        local_id: Some(id),
        node,
        children,
        code_edits: None,
    }
}

/// Appends one self-consistent snapshot of the active tree to the virtual arena.
/// Earlier bytes stay addressable so existing expansion backtraces remain valid.
fn project_forest(
    source: &MappedSource,
    forest: &mut [OwnedSubtree],
    call_site: &MappedSpan,
    expansion: ExpansionId,
) -> Result<MappedSource, SourceMapError> {
    if forest.is_empty() {
        return Ok(source.clone());
    }

    let generated_origins = generated_span(call_site, expansion).origins;
    let mut virtual_source = source.virtual_source().to_owned();
    let mut segments = source.source_map().segments().to_vec();
    if !virtual_source.is_empty()
        && !virtual_source.ends_with('\n')
        && !virtual_source.ends_with('\r')
    {
        let start = virtual_source.len();
        virtual_source.push('\n');
        segments.push(SourceMapSegment {
            virtual_range: TextRange::new(start, start + 1),
            origins: generated_origins.clone(),
        });
    }

    for subtree in forest.iter_mut() {
        append_projected_subtree(
            source,
            subtree,
            &generated_origins,
            &mut virtual_source,
            &mut segments,
        )?;
    }

    let projected = MappedSource::new(
        source.original().to_owned(),
        virtual_source,
        source.expansions().clone(),
        segments,
    )?;
    for subtree in forest {
        finalize_projected_subtree(&projected, subtree)?;
    }
    Ok(projected)
}

fn append_projected_subtree(
    source: &MappedSource,
    subtree: &mut OwnedSubtree,
    generated_origins: &[SourceOrigin],
    virtual_source: &mut String,
    segments: &mut Vec<SourceMapSegment>,
) -> Result<(), SourceMapError> {
    let new_start = virtual_source.len();
    if let Some(edits) = &subtree.code_edits {
        append_edited_line(source, &mut subtree.node, edits, generated_origins, virtual_source, segments)?;
    } else if subtree.local_id.is_some() {
        virtual_source.push_str(&subtree.node.line.raw_text);
        virtual_source.push('\n');
        let new_end = virtual_source.len();
        segments.push(SourceMapSegment {
            virtual_range: TextRange::new(new_start, new_end),
            origins: generated_origins.to_vec(),
        });
        initialize_generated_line(&mut subtree.node, new_start, new_end, generated_origins);
    } else {
        let old_start = subtree.node.line.span.virtual_range.start;
        source.append_preserved_range(
            subtree.node.line.span.virtual_range,
            virtual_source,
            segments,
        )?;
        rebase_existing_line(&mut subtree.node, old_start, new_start);
    }

    for child in &mut subtree.children {
        append_projected_subtree(source, child, generated_origins, virtual_source, segments)?;
    }
    Ok(())
}

fn append_edited_line(
    source: &MappedSource, node: &mut RawNode, edits: &[crate::TextEdit],
    generated_origins: &[SourceOrigin], output: &mut String, segments: &mut Vec<SourceMapSegment>,
) -> Result<(), SourceMapError> {
    let old_line = node.line.span.virtual_range;
    let old_code = node.code_span.as_ref().expect("validated code").virtual_range;
    let new_start = output.len();
    let mut cursor = old_line.start;
    let mut new_text = String::new();
    let mut text_cursor = 0;
    for edit in edits {
        let range = TextRange::new(old_code.start + edit.range.start, old_code.start + edit.range.end);
        source.append_preserved_range(TextRange::new(cursor, range.start), output, segments)?;
        let start = output.len();
        output.push_str(&edit.replacement);
        if start != output.len() {
            let expansion = generated_origins.first().and_then(|origin| origin.expansion);
            let origins = source.map_range(range)?.origins.into_iter().map(|origin| SourceOrigin {
                original_range: origin.original_range,
                kind: if origin.original_range.is_empty() { OriginKind::Anchored } else { OriginKind::Replaced },
                expansion,
            });
            segments.push(SourceMapSegment::with_origins(TextRange::new(start, output.len()), origins));
        }
        new_text.push_str(&node.text[text_cursor..edit.range.start]);
        new_text.push_str(&edit.replacement);
        text_cursor = edit.range.end;
        cursor = range.end;
    }
    new_text.push_str(&node.text[text_cursor..]);
    source.append_preserved_range(TextRange::new(cursor, old_line.end), output, segments)?;
    let colon = usize::from(node.kind == RawNodeKind::Section);
    let new_code_end = new_start + old_code.start - old_line.start + new_text.len() + colon;
    let at = |old: usize| if old >= old_code.end {
        new_code_end + old - old_code.end
    } else { new_start + old - old_line.start };
    let shift = |span: &mut MappedSpan| {
        span.virtual_range = TextRange::new(at(span.virtual_range.start), at(span.virtual_range.end));
    };
    shift(&mut node.line.span);
    shift(&mut node.line.content_span);
    shift(&mut node.line.line_ending_span);
    shift(&mut node.line.indentation.span);
    for trivia in &mut node.line.trailing_trivia { shift(&mut trivia.span); }
    if let Some(span) = &mut node.code_span { shift(span); }
    if let Some(span) = &mut node.header_span { shift(span); }
    node.line.raw_text = output[node.line.content_span.virtual_range.start..node.line.content_span.virtual_range.end].to_owned();
    node.text = new_text;
    Ok(())
}

fn initialize_generated_line(
    node: &mut RawNode,
    line_start: usize,
    line_end: usize,
    origins: &[SourceOrigin],
) {
    let content_end = line_end - 1;
    let mapped = |range| MappedSpan {
        virtual_range: range,
        origins: origins.to_vec(),
    };
    let point = mapped(TextRange::empty(line_start));
    node.line.span = mapped(TextRange::new(line_start, line_end));
    node.line.content_span = mapped(TextRange::new(line_start, content_end));
    node.line.line_ending_span = mapped(TextRange::new(content_end, line_end));
    node.line.indentation.span = point;
    node.line.trailing_trivia.clear();
    node.code_span = matches!(node.kind, RawNodeKind::Simple | RawNodeKind::Section)
        .then(|| mapped(TextRange::new(line_start, content_end)));
    node.header_span = (node.kind == RawNodeKind::Section)
        .then(|| mapped(TextRange::new(line_start, content_end)));
}

fn rebase_existing_line(node: &mut RawNode, old_start: usize, new_start: usize) {
    rebase_span(&mut node.line.span, old_start, new_start);
    rebase_span(&mut node.line.content_span, old_start, new_start);
    rebase_span(&mut node.line.line_ending_span, old_start, new_start);
    rebase_span(&mut node.line.indentation.span, old_start, new_start);
    for trivia in &mut node.line.trailing_trivia {
        rebase_span(&mut trivia.span, old_start, new_start);
    }
    if let Some(span) = &mut node.code_span {
        rebase_span(span, old_start, new_start);
    }
    if let Some(span) = &mut node.header_span {
        rebase_span(span, old_start, new_start);
    }
}

fn rebase_span(span: &mut MappedSpan, old_start: usize, new_start: usize) {
    let range = span.virtual_range;
    span.virtual_range = TextRange::new(
        new_start + range.start - old_start,
        new_start + range.end - old_start,
    );
}

fn finalize_projected_subtree(
    source: &MappedSource,
    subtree: &mut OwnedSubtree,
) -> Result<(), SourceMapError> {
    remap_line_spans(source, &mut subtree.node)?;
    for child in &mut subtree.children {
        finalize_projected_subtree(source, child)?;
    }

    if subtree.node.kind == RawNodeKind::Section {
        let line_range = subtree.node.line.span.virtual_range;
        let body_range =
            subtree
                .children
                .first()
                .map_or(TextRange::empty(line_range.end), |first| {
                    TextRange::new(
                        first.node.line.span.virtual_range.start,
                        subtree
                            .children
                            .last()
                            .expect("a non-empty child list has a last item")
                            .node
                            .span
                            .virtual_range
                            .end,
                    )
                });
        subtree.node.body_span = Some(source.map_range(body_range)?);
        subtree.node.span = source.map_range(TextRange::new(line_range.start, body_range.end))?;
    } else {
        subtree.node.body_span = None;
        subtree.node.span = subtree.node.line.span.clone();
    }
    Ok(())
}

fn remap_line_spans(source: &MappedSource, node: &mut RawNode) -> Result<(), SourceMapError> {
    remap_span(source, &mut node.line.span)?;
    remap_span(source, &mut node.line.content_span)?;
    remap_span(source, &mut node.line.line_ending_span)?;
    remap_span(source, &mut node.line.indentation.span)?;
    for trivia in &mut node.line.trailing_trivia {
        remap_span(source, &mut trivia.span)?;
    }
    if let Some(span) = &mut node.code_span {
        remap_span(source, span)?;
    }
    if let Some(span) = &mut node.header_span {
        remap_span(source, span)?;
    }
    Ok(())
}

fn remap_span(source: &MappedSource, span: &mut MappedSpan) -> Result<(), SourceMapError> {
    *span = source.map_range(span.virtual_range)?;
    Ok(())
}

fn generated_span(call_site: &MappedSpan, expansion: ExpansionId) -> MappedSpan {
    MappedSpan {
        virtual_range: call_site.virtual_range,
        origins: call_site
            .origins
            .iter()
            .map(|origin| SourceOrigin {
                original_range: origin.original_range,
                kind: if origin.original_range.is_empty() {
                    OriginKind::Anchored
                } else {
                    OriginKind::Replaced
                },
                expansion: Some(expansion),
            })
            .collect(),
    }
}

fn generated_point(call_site: &MappedSpan, expansion: ExpansionId) -> MappedSpan {
    let mut span = generated_span(call_site, expansion);
    span.virtual_range = TextRange::empty(call_site.virtual_range.end);
    span
}

fn find_subtree(forest: &[OwnedSubtree], target: RawNodeId) -> Option<&OwnedSubtree> {
    for subtree in forest {
        if subtree.node.id == target {
            return Some(subtree);
        }
        if let Some(found) = find_subtree(&subtree.children, target) {
            return Some(found);
        }
    }
    None
}

fn find_subtree_mut(forest: &mut [OwnedSubtree], target: RawNodeId) -> Option<&mut OwnedSubtree> {
    for subtree in forest {
        if subtree.node.id == target { return Some(subtree); }
        if let Some(found) = find_subtree_mut(&mut subtree.children, target) { return Some(found); }
    }
    None
}

fn replace_node(
    forest: &mut Vec<OwnedSubtree>,
    target: RawNodeId,
    mut replacement: Vec<OwnedSubtree>,
    retained_children: Option<RetainedChildren>,
) -> Result<bool, TreeEditError> {
    for index in 0..forest.len() {
        if forest[index].node.id == target {
            let original = forest.remove(index);
            if let Some(retained) = retained_children {
                let parent = find_generated_mut(&mut replacement, retained.parent).ok_or(
                    TreeEditError::UnknownRetainedChildrenParent {
                        id: retained.parent,
                    },
                )?;
                if parent.node.kind != RawNodeKind::Section {
                    return Err(TreeEditError::RetainedChildrenParentNotSection {
                        id: retained.parent,
                    });
                }
                match retained.placement {
                    RetainedChildrenPlacement::Prepend => {
                        let mut children = original.children;
                        children.append(&mut parent.children);
                        parent.children = children;
                    }
                    RetainedChildrenPlacement::Append => {
                        parent.children.extend(original.children);
                    }
                }
            }
            forest.splice(index..index, replacement);
            return Ok(true);
        }
        if replace_node(
            &mut forest[index].children,
            target,
            replacement.clone(),
            retained_children,
        )? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn replace_children(
    forest: &mut [OwnedSubtree],
    target: RawNodeId,
    replacement: Vec<OwnedSubtree>,
) -> bool {
    for subtree in forest {
        if subtree.node.id == target {
            subtree.children = replacement;
            return true;
        }
        if replace_children(&mut subtree.children, target, replacement.clone()) {
            return true;
        }
    }
    false
}

fn find_generated_mut(
    forest: &mut [OwnedSubtree],
    id: GeneratedRawNodeId,
) -> Option<&mut OwnedSubtree> {
    for subtree in forest {
        if subtree.local_id == Some(id) {
            return Some(subtree);
        }
        if let Some(found) = find_generated_mut(&mut subtree.children, id) {
            return Some(found);
        }
    }
    None
}

fn flatten_tree(forest: Vec<OwnedSubtree>, original: &RawTree) -> RawTree {
    let mut tree = original.empty_with_metadata();
    for subtree in forest {
        let id = flatten_node(subtree, None, &mut tree);
        tree.roots.push(id);
    }
    tree
}

fn flatten_node(
    mut subtree: OwnedSubtree,
    parent: Option<RawNodeId>,
    tree: &mut RawTree,
) -> RawNodeId {
    let id = RawNodeId::new(
        u64::try_from(tree.nodes.len()).expect("RawTree node count cannot exceed u64"),
    );
    subtree.node.id = id;
    subtree.node.parent = parent;
    subtree.node.children.clear();
    tree.nodes.push(subtree.node);
    let child_ids = subtree
        .children
        .into_iter()
        .map(|child| flatten_node(child, Some(id), tree))
        .collect::<Vec<_>>();
    tree.nodes[usize::try_from(id.get()).expect("allocated ID fits usize")].children = child_ids;
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RawTreeOptions, parse_raw_tree};

    fn source_tree(source: &str) -> (MappedSource, RawTree) {
        let source = MappedSource::identity(source);
        let tree = parse_raw_tree(&source, RawTreeOptions::for_skript_version(2, 9));
        (source, tree)
    }

    fn code_edit(start: usize, end: usize, replacement: &str) -> crate::TextEdit {
        crate::TextEdit { range: TextRange::new(start, end), replacement: replacement.into(), anchor: None }
    }

    fn edit_metadata() -> TreeEditMetadata {
        TreeEditMetadata { component: "test".into(), hook: "edit-code".into() }
    }

    #[test]
    fn edit_code_preserves_exact_fragments_context_trivia_and_native_extent() {
        let (source, tree) = source_tree("# \u{65e5}\u{1f98a}\r\non join:\r\n    set {_x} to {@value} # tail\r\n    send {_x}\r\n");
        let owner = tree.roots[1];
        let target = tree.get(owner).unwrap().children[0];
        let before = tree.get(target).unwrap();
        let at = before.text.find("{@value}").unwrap();
        let result = apply_tree_edit(&source, &tree, target, TreeEdit::EditCode {
            edits: vec![code_edit(at, at + "{@value}".len(), "1 day")],
        }, edit_metadata()).unwrap();
        let node = result.tree.get(target).unwrap();
        assert_eq!(node.text, "set {_x} to 1 day");
        assert_eq!(node.line.raw_text, "    set {_x} to 1 day # tail");
        assert_eq!(node.line.line_ending, LineEnding::CrLf);
        assert_eq!(node.syntax_context, before.syntax_context);
        assert_eq!(result.tree.get(owner).unwrap().children, tree.get(owner).unwrap().children);
        assert_eq!(result.tree.original_source(), Some(source.original()));
        assert_eq!(result.tree.original_extent(node), tree.original_extent(before));
        let code = node.code_span.as_ref().unwrap().virtual_range.start;
        let x = node.text.find("{_x}").unwrap();
        let origin = result.source.map_range(TextRange::new(code+x, code+x+4)).unwrap();
        assert!(matches!(origin.origins.as_slice(), [origin] if origin.kind==OriginKind::Exact
            && origin.original_range.slice(source.original())==Some("{_x}")));
        let value = node.text.find("1 day").unwrap();
        let origin = result.source.map_range(TextRange::new(code+value, code+value+5)).unwrap();
        assert!(matches!(origin.origins.as_slice(), [origin] if origin.kind==OriginKind::Replaced
            && origin.original_range.slice(source.original())==Some("{@value}")
            && origin.expansion==result.expansion));
        assert_eq!(source.virtual_source(), source.original(), "input source is immutable");
        assert_eq!(tree.get(target).unwrap().text, "set {_x} to {@value}");
    }

    #[test]
    fn edit_code_rejects_invalid_ranges_and_keeps_noops_inert() {
        let (source, tree) = source_tree("send \u{65e5}\u{672c}\n");
        let target = tree.roots[0];
        for edits in [Vec::new(), vec![code_edit(0,4,"send")]] {
            let result = apply_tree_edit(&source, &tree, target, TreeEdit::EditCode { edits }, edit_metadata()).unwrap();
            assert_eq!(result.expansion, None);
            assert_eq!(result.source, source);
            assert_eq!(result.tree, tree);
        }
        let mut anchored = code_edit(0,4,"show");anchored.anchor=Some(0);
        for edits in [
            vec![code_edit(0,0,"a")], vec![code_edit(6,7,"a")],
            vec![code_edit(0,999,"a")], vec![code_edit(0,4,"a\nb")],
            vec![code_edit(0,3,"a"),code_edit(2,4,"b")], vec![anchored],
        ] {
            assert!(matches!(apply_tree_edit(&source, &tree, target, TreeEdit::EditCode { edits }, edit_metadata()),
                Err(TreeEditError::InvalidCodeEdit(_))));
        }
    }

    #[test]
    fn edit_code_preserves_section_colon_and_rejects_decoded_offset_aliases() {
        let (source, tree) = source_tree("command /old:\r\n    send 1\r\n");
        let target = tree.roots[0];
        let result = apply_tree_edit(&source, &tree, target, TreeEdit::EditCode {
            edits: vec![code_edit(9,12,"new-name")],
        }, edit_metadata()).unwrap();
        let node = result.tree.get(target).unwrap();
        assert_eq!(node.text, "command /new-name");
        assert_eq!(node.line.raw_text, "command /new-name:");
        assert_eq!(node.kind, RawNodeKind::Section);
        assert_eq!(node.line.line_ending, LineEnding::CrLf);
        assert_eq!(node.children, tree.get(target).unwrap().children);
        let (source, tree) = source_tree("send ## {@value}\n");
        let target = tree.roots[0];
        let node = tree.get(target).unwrap();
        assert_ne!(node.code_span.as_ref().unwrap().virtual_range.slice(source.virtual_source()), Some(node.text.as_str()));
        let at=node.text.find("{@value}").unwrap();
        assert!(matches!(apply_tree_edit(&source, &tree, target, TreeEdit::EditCode {
            edits: vec![code_edit(at,at+8,"1")],
        }, edit_metadata()), Err(TreeEditError::InvalidCodeEdit(_))));
    }

    #[test]
    fn edit_code_never_launders_a_generated_binding_or_context() {
        let (source, tree)=source_tree("original\n");
        let generated=apply_tree_edit(&source,&tree,tree.roots[0],TreeEdit::ReplaceNode {
            replacement:GeneratedRawTree { roots:vec![GeneratedRawNodeId::new(0)],nodes:vec![
                generated_node(0,GeneratedRawNodeKind::Simple,"set {_x} to marker",&[])
            ]},retained_children:None,
        },edit_metadata()).unwrap();
        let target=generated.tree.roots[0];let before=generated.tree.get(target).unwrap();
        let at=before.text.find("marker").unwrap();
        let result=apply_tree_edit(&generated.source,&generated.tree,target,TreeEdit::EditCode {
            edits:vec![code_edit(at,at+6,"1")],
        },edit_metadata()).unwrap();
        let node=result.tree.get(target).unwrap();
        assert_eq!(node.syntax_context,before.syntax_context);
        assert_ne!(node.syntax_context,SyntaxContextId::ROOT);
        assert_eq!(result.tree.original_extent(node),None);
        assert!(node.code_span.as_ref().unwrap().origins.iter().all(|o|o.kind!=OriginKind::Exact));
    }

    fn generated_node(
        id: u64,
        kind: GeneratedRawNodeKind,
        text: &str,
        children: &[u64],
    ) -> GeneratedRawNode {
        GeneratedRawNode {
            id: GeneratedRawNodeId::new(id),
            kind,
            text: text.to_owned(),
            children: children
                .iter()
                .copied()
                .map(GeneratedRawNodeId::new)
                .collect(),
        }
    }

    fn metadata(hook: &str) -> TreeEditMetadata {
        TreeEditMetadata {
            component: "test.component".to_owned(),
            hook: hook.to_owned(),
        }
    }

    #[test]
    fn replaces_one_node_with_multiple_generated_nodes() {
        let (source, tree) = source_tree("first\nreplace\nlast\n");
        let application = apply_tree_edit(
            &source,
            &tree,
            RawNodeId::new(1),
            TreeEdit::ReplaceNode {
                replacement: GeneratedRawTree {
                    roots: vec![GeneratedRawNodeId::new(10), GeneratedRawNodeId::new(11)],
                    nodes: vec![
                        generated_node(10, GeneratedRawNodeKind::Simple, "alpha", &[]),
                        generated_node(11, GeneratedRawNodeKind::Simple, "beta", &[]),
                    ],
                },
                retained_children: None,
            },
            metadata("replace"),
        )
        .unwrap();

        assert_eq!(
            application
                .tree
                .roots
                .iter()
                .map(|id| application.tree.get(*id).unwrap().text.as_str())
                .collect::<Vec<_>>(),
            ["first", "alpha", "beta", "last"]
        );
        assert_eq!(application.replacement_roots, 2);
        let generated = application.tree.get(application.tree.roots[1]).unwrap();
        assert_eq!(generated.syntax_context, application.syntax_context);
        assert_eq!(
            generated
                .code_span
                .as_ref()
                .unwrap()
                .virtual_range
                .slice(application.source.virtual_source()),
            Some("alpha"),
        );
        let second = application.tree.get(application.tree.roots[2]).unwrap();
        assert_eq!(
            second
                .code_span
                .as_ref()
                .unwrap()
                .virtual_range
                .slice(application.source.virtual_source()),
            Some("beta"),
        );
        assert!(
            generated
                .span
                .origins
                .iter()
                .all(|origin| origin.expansion == application.expansion)
        );
        assert_eq!(
            application
                .source
                .expansion_backtrace(application.expansion.unwrap())
                .unwrap()[0]
                .hook
                .as_str(),
            "replace"
        );
    }

    #[test]
    fn preserves_original_section_children_at_a_generated_section() {
        let (source, tree) = source_tree("old:\n    child\n");
        let application = apply_tree_edit(
            &source,
            &tree,
            RawNodeId::new(0),
            TreeEdit::ReplaceNode {
                replacement: GeneratedRawTree {
                    roots: vec![GeneratedRawNodeId::new(1)],
                    nodes: vec![
                        generated_node(1, GeneratedRawNodeKind::Section, "new", &[2]),
                        generated_node(2, GeneratedRawNodeKind::Simple, "generated", &[]),
                    ],
                },
                retained_children: Some(RetainedChildren {
                    parent: GeneratedRawNodeId::new(1),
                    placement: RetainedChildrenPlacement::Append,
                }),
            },
            metadata("preserve"),
        )
        .unwrap();

        let section = application.tree.get(application.tree.roots[0]).unwrap();
        assert_eq!(section.text, "new");
        assert_eq!(
            section.header_span.as_ref().and_then(|span| span
                .virtual_range
                .slice(application.source.virtual_source())),
            Some("new:")
        );
        assert_eq!(
            section.body_span.as_ref().and_then(|span| span
                .virtual_range
                .slice(application.source.virtual_source())),
            Some("generated\n    child\n")
        );
        assert_eq!(
            section
                .span
                .virtual_range
                .slice(application.source.virtual_source()),
            Some("new:\ngenerated\n    child\n")
        );
        assert_eq!(
            section
                .children
                .iter()
                .map(|id| application.tree.get(*id).unwrap().text.as_str())
                .collect::<Vec<_>>(),
            ["generated", "child"]
        );
        assert_eq!(
            application
                .tree
                .get(section.children[1])
                .unwrap()
                .syntax_context,
            SyntaxContextId::ROOT
        );
    }

    #[test]
    fn replaces_a_section_body_without_replacing_its_header() {
        let (source, tree) = source_tree("section:\n    old\n");
        let application = apply_tree_edit(
            &source,
            &tree,
            RawNodeId::new(0),
            TreeEdit::ReplaceChildren {
                replacement: GeneratedRawTree {
                    roots: vec![GeneratedRawNodeId::new(1)],
                    nodes: vec![generated_node(1, GeneratedRawNodeKind::Simple, "new", &[])],
                },
            },
            metadata("body"),
        )
        .unwrap();

        let section = application.tree.get(application.tree.roots[0]).unwrap();
        assert_eq!(section.text, "section");
        assert_eq!(section.syntax_context, SyntaxContextId::ROOT);
        assert_eq!(
            section.header_span.as_ref().and_then(|span| span
                .virtual_range
                .slice(application.source.virtual_source())),
            Some("section:")
        );
        assert_eq!(
            section.body_span.as_ref().and_then(|span| span
                .virtual_range
                .slice(application.source.virtual_source())),
            Some("new\n")
        );
        assert_eq!(
            application.tree.get(section.children[0]).unwrap().text,
            "new"
        );
    }

    #[test]
    fn rejects_cycles_and_reused_generated_nodes() {
        let cycle = GeneratedRawTree {
            roots: vec![GeneratedRawNodeId::new(1)],
            nodes: vec![
                generated_node(1, GeneratedRawNodeKind::Section, "one", &[2]),
                generated_node(2, GeneratedRawNodeKind::Section, "two", &[1]),
            ],
        };
        assert!(matches!(
            validate_fragment(&cycle),
            Err(TreeEditError::ReusedGeneratedNode { .. })
                | Err(TreeEditError::GeneratedCycle { .. })
        ));

        let reused = GeneratedRawTree {
            roots: vec![GeneratedRawNodeId::new(1), GeneratedRawNodeId::new(1)],
            nodes: vec![generated_node(1, GeneratedRawNodeKind::Simple, "once", &[])],
        };
        assert!(matches!(
            validate_fragment(&reused),
            Err(TreeEditError::ReusedGeneratedNode { .. })
        ));
    }

    #[test]
    fn rejects_generated_text_that_contains_physical_line_breaks() {
        let multiline = GeneratedRawTree {
            roots: vec![GeneratedRawNodeId::new(1)],
            nodes: vec![generated_node(
                1,
                GeneratedRawNodeKind::Simple,
                "first\nsecond",
                &[],
            )],
        };

        assert!(matches!(
            validate_fragment(&multiline),
            Err(TreeEditError::InvalidGeneratedText { .. })
        ));
    }

    #[test]
    fn projected_tree_spans_stay_on_utf8_boundaries() {
        let (source, tree) = source_tree("前\n置換\n後\n");
        let application = apply_tree_edit(
            &source,
            &tree,
            RawNodeId::new(1),
            TreeEdit::ReplaceNode {
                replacement: GeneratedRawTree {
                    roots: vec![GeneratedRawNodeId::new(1)],
                    nodes: vec![generated_node(1, GeneratedRawNodeKind::Simple, "生成", &[])],
                },
                retained_children: None,
            },
            metadata("utf8"),
        )
        .unwrap();
        let virtual_source = application.source.virtual_source();

        for node in &application.tree.nodes {
            for range in [
                node.span.virtual_range,
                node.line.span.virtual_range,
                node.line.content_span.virtual_range,
                node.line.line_ending_span.virtual_range,
            ] {
                assert!(virtual_source.is_char_boundary(range.start));
                assert!(virtual_source.is_char_boundary(range.end));
            }
            if let Some(span) = &node.code_span {
                assert!(virtual_source.is_char_boundary(span.virtual_range.start));
                assert!(virtual_source.is_char_boundary(span.virtual_range.end));
            }
        }

        let generated = application.tree.get(application.tree.roots[1]).unwrap();
        assert_eq!(
            generated
                .code_span
                .as_ref()
                .and_then(|span| span.virtual_range.slice(virtual_source)),
            Some("生成")
        );
        assert_eq!(
            generated
                .code_span
                .as_ref()
                .and_then(|span| application.source.map_range(span.virtual_range).ok())
                .and_then(|span| span.primary_origin())
                .and_then(|origin| origin.original_range.slice(source.original())),
            Some("置換\n")
        );
    }
}

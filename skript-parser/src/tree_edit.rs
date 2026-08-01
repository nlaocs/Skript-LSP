//! Validated replacement operations over a lossless `RawTree`.
//!
//! Addon-provided fragments use local IDs. The host allocates stable node,
//! expansion, and syntax-context identities only after the entire edit validates.
#![allow(missing_docs)] // Type-level docs describe aggregate field contracts.

use crate::{
    ExpansionId, MappedSource, MappedSpan, OriginKind, RawLine, RawNode, RawNodeId, RawNodeKind,
    RawTree, RawTrivia, RawTriviaKind, SourceOrigin, SyntaxContextId, TextRange, TreeExpansion,
    TreeExpansionError,
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
    pub expansion: ExpansionId,
    pub syntax_context: SyntaxContextId,
    pub replacement_roots: usize,
}

/// Validates and atomically applies one generated Tree macro operation.
///
/// Generated node IDs are local to the supplied fragment. After complete
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
/// assert_eq!(applied.source.virtual_source(), "replace me\n");
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
    let fragment = match &edit {
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
    }

    Ok(TreeEditApplication {
        source: expansion.source,
        tree: flatten_tree(forest, tree),
        expansion: expansion.expansion,
        syntax_context: expansion.syntax_context,
        replacement_roots,
    })
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
/// Rejected generated tree, target, retained-child policy, or provenance operation.
pub enum TreeEditError {
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
}

#[derive(Debug, Clone)]
struct OwnedSubtree {
    local_id: Option<GeneratedRawNodeId>,
    node: RawNode,
    children: Vec<OwnedSubtree>,
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
    let invalid_text = match node.kind {
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
            line_ending: crate::LineEnding::None,
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
    };
    OwnedSubtree {
        local_id: Some(id),
        node,
        children,
    }
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
    let mut tree = RawTree {
        roots: Vec::new(),
        nodes: Vec::new(),
        diagnostics: original.diagnostics.clone(),
        indentation: original.indentation.clone(),
    };
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
        assert!(
            generated
                .span
                .origins
                .iter()
                .all(|origin| origin.expansion == Some(application.expansion))
        );
        assert_eq!(
            application
                .source
                .expansion_backtrace(application.expansion)
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
}

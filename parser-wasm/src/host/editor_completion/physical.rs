//! Current native physical geometry for editing-only observations. Preparation
//! attempts retain their separate authority, lifetime and invalidation graph.
use std::collections::{BTreeMap, BTreeSet};
use skript_parser::{MappedSource, RawNodeKind, RawTree};

const MAX_INDEX_NODES: usize = 65_536;

pub(super) struct PhysicalOwners {
    // Ephemeral addresses of immutably borrowed native parse inputs. They are
    // compared only during this live editor Session, never dereferenced,
    // serialized, cached across revisions, or treated as semantic evidence.
    source: usize,
    tree: usize,
    parents: BTreeMap<u64, Option<u64>>,
    lines: BTreeMap<(u64, u64), Option<u64>>,
}

impl PhysicalOwners {
    pub(super) fn new(source: &MappedSource, tree: &RawTree) -> Option<Self> {
        if tree.nodes.len() > MAX_INDEX_NODES
            || tree.original_source().is_some_and(|original| original != source.original()) { return None; }
        let mut parents = BTreeMap::new();
        let mut lines = BTreeMap::new();
        for node in &tree.nodes {
            if parents.insert(node.id.get(), node.parent.map(|id| id.get())).is_some() {
                return None;
            }
            if node.kind != RawNodeKind::Simple { continue; }
            let Some(span) = &node.code_span else { continue; };
            let range = span.virtual_range;
            if range.start >= range.end || source.virtual_source().get(range.start..range.end).is_none() {
                return None;
            }
            // Duplicate geometry cannot identify one physical statement.
            lines.entry((range.start as u64, range.end as u64))
                .and_modify(|owner| *owner = None)
                .or_insert(node.parent.map(|id| id.get()));
        }
        Some(Self {source:std::ptr::from_ref(source) as usize,
            tree:std::ptr::from_ref(tree) as usize,parents,lines})
    }

    pub(super) fn matches(&self, source: &MappedSource, tree: &RawTree) -> bool {
        self.source == std::ptr::from_ref(source) as usize
            && self.tree == std::ptr::from_ref(tree) as usize
    }

    pub(super) fn parent_for(&self, logical_owner: u64, start: u64, end: u64) -> Option<u64> {
        let parent = self.lines.get(&(start, end)).copied().flatten()?;
        let mut cursor = Some(parent);
        let mut seen = BTreeSet::new();
        let mut owns = false;
        // Validate the complete chain, even after reaching the logical owner:
        // a cycle or missing ancestor never legitimizes an editing input.
        for _ in 0..=self.parents.len() {
            let Some(id) = cursor else { return owns.then_some(parent); };
            if !seen.insert(id) { return None; }
            owns |= id == logical_owner;
            cursor = *self.parents.get(&id)?;
        }
        None
    }
}

#[derive(Default)]
pub(super) struct Geometry {
    identity: Option<(usize, usize)>,
    owners: Option<PhysicalOwners>,
    invalid: bool,
}

impl Geometry {
    /// Returns true once when a different native parse invalidates old batches.
    pub(super) fn observe(&mut self, headers: bool, source: &MappedSource, tree: &RawTree) -> bool {
        if self.invalid { return false; }
        let identity=(std::ptr::from_ref(source) as usize,std::ptr::from_ref(tree) as usize);
        if self.identity.is_none() && headers {
            self.identity=Some(identity);self.owners=PhysicalOwners::new(source,tree);return false;
        }
        let same=self.owners.as_ref().map_or(self.identity==Some(identity),|owners|owners.matches(source,tree));
        if !same { self.invalid=true;self.owners=None;return true; }
        false
    }
    pub(super) fn parent_for(&self, logical: u64, start: u64, end: u64) -> Option<u64> {
        if self.invalid { return None; }
        self.owners.as_ref()?.parent_for(logical,start,end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn code_id(tree:&RawTree,source:&MappedSource,text:&str)->u64 {
        tree.nodes.iter().find(|node|node.code_span.as_ref().is_some_and(|span|
            source.virtual_source().get(span.virtual_range.start..span.virtual_range.end).is_some_and(|code|code.trim_end_matches(':')==text)))
            .unwrap().id.get()
    }
    fn span(tree:&RawTree,id:u64)->(u64,u64) {
        let range=tree.nodes.iter().find(|node|node.id.get()==id).unwrap().code_span.as_ref().unwrap().virtual_range;
        (range.start as u64,range.end as u64)
    }
    #[test]
    fn physical_entry_parent_is_distinct_from_the_current_preparation_owner() {
        let source=MappedSource::identity("command /one:\n    trigger:\n        {_x}.len\n    other:\n        {_y}.len\ncommand /two:\n    trigger:\n        {_z}.len\n");
        let tree=skript_parser::parse_raw_tree(&source,skript_parser::RawTreeOptions::for_skript_version(2,15));
        let index=PhysicalOwners::new(&source,&tree).unwrap();
        let root=code_id(&tree,&source,"command /one");
        let body=code_id(&tree,&source,"trigger");
        let line=code_id(&tree,&source,"{_x}.len");let (start,end)=span(&tree,line);
        assert_eq!(index.parent_for(root,start,end),Some(body));
        assert_eq!(index.parent_for(body,start,end),Some(body));
        assert_eq!(index.parent_for(code_id(&tree,&source,"other"),start,end),None);
        assert_eq!(index.parent_for(code_id(&tree,&source,"command /two"),start,end),None);
        assert_eq!(index.parent_for(root,start+1,end),None);
        assert!(index.matches(&source,&tree));
        assert!(!index.matches(&source.clone(),&tree));
        assert!(!index.matches(&source,&tree.clone()));
    }
    #[test]
    fn ambiguous_geometry_missing_ancestors_and_cycles_fail_closed() {
        let source=MappedSource::identity("owner:\n    child:\n        {_x}.len\n        {_y}.len\n");
        let tree=skript_parser::parse_raw_tree(&source,skript_parser::RawTreeOptions::for_skript_version(2,15));
        let root=code_id(&tree,&source,"owner");let body=code_id(&tree,&source,"child");
        let line=code_id(&tree,&source,"{_x}.len");let (start,end)=span(&tree,line);
        let mut index=PhysicalOwners::new(&source,&tree).unwrap();
        index.parents.insert(root,Some(body));
        assert_eq!(index.parent_for(root,start,end),None,"cycle through the logical owner");
        let mut index=PhysicalOwners::new(&source,&tree).unwrap();index.parents.remove(&root);
        assert_eq!(index.parent_for(body,start,end),None,"missing ancestor beyond the logical owner");
        let mut duplicate=tree.clone();
        let original=duplicate.nodes.iter().find(|node|node.id.get()==line).unwrap().code_span.clone();
        let other=code_id(&tree,&source,"{_y}.len");
        duplicate.nodes.iter_mut().find(|node|node.id.get()==other).unwrap().code_span=original;
        assert_eq!(PhysicalOwners::new(&source,&duplicate).unwrap().parent_for(root,start,end),None);
        let mut duplicate=tree.clone();duplicate.nodes.push(duplicate.nodes[0].clone());
        assert!(PhysicalOwners::new(&source,&duplicate).is_none());
        let mut oversized=tree.clone();oversized.nodes.resize(MAX_INDEX_NODES+1,tree.nodes[0].clone());
        assert!(PhysicalOwners::new(&source,&oversized).is_none());
        assert!(PhysicalOwners::new(&MappedSource::identity("unrelated source"),&tree).is_none());
    }
    #[test]
    fn source_tree_guard_is_sticky_and_scoped_to_one_session() {
        let source=MappedSource::identity("owner:\n    {_x}.len\n");
        let tree=skript_parser::parse_raw_tree(&source,skript_parser::RawTreeOptions::for_skript_version(2,15));
        let root=code_id(&tree,&source,"owner");let line=code_id(&tree,&source,"{_x}.len");let (start,end)=span(&tree,line);
        let mut geometry=Geometry::default();assert!(!geometry.observe(true,&source,&tree));
        assert_eq!(geometry.parent_for(root,start,end),Some(root));
        assert!(!geometry.observe(false,&source,&tree));
        assert!(geometry.observe(false,&source,&tree.clone()));
        assert_eq!(geometry.parent_for(root,start,end),None);
        assert!(!geometry.observe(true,&source,&tree),"a new Headers event cannot revive the old session");
        assert_eq!(geometry.parent_for(root,start,end),None);
        let mut fresh=Geometry::default();assert!(!fresh.observe(true,&source,&tree));
        assert_eq!(fresh.parent_for(root,start,end),Some(root));
        assert!(fresh.observe(false,&source.clone(),&tree));
    }
    #[test]
    fn unicode_ranges_must_stay_on_current_character_boundaries() {
        let source=MappedSource::identity("owner:\n    {_\u{96ea}\u{1f600}}.len\n");
        let mut tree=skript_parser::parse_raw_tree(&source,skript_parser::RawTreeOptions::for_skript_version(2,15));
        let root=code_id(&tree,&source,"owner");let line=code_id(&tree,&source,"{_\u{96ea}\u{1f600}}.len");let (start,end)=span(&tree,line);
        assert_eq!(PhysicalOwners::new(&source,&tree).unwrap().parent_for(root,start,end),Some(root));
        let inside=source.virtual_source().find('\u{1f600}').unwrap()+1;
        tree.nodes.iter_mut().find(|node|node.id.get()==line).unwrap().code_span.as_mut().unwrap().virtual_range.start=inside;
        assert!(PhysicalOwners::new(&source,&tree).is_none());
    }
}

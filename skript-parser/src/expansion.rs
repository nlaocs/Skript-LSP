//! Macro expansion identities, syntax contexts, sites, and provenance graph.
//!
//! The graph is a validated DAG. It supports both a primary backtrace and every
//! distinct parent path for text assembled from multiple origins.
#![allow(missing_docs)] // Type-level docs describe aggregate field contracts.

use crate::TextRange;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable numeric identity of one graph expansion.
pub struct ExpansionId(u32);

impl ExpansionId {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for ExpansionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Hygiene context assigned to original or generated syntax.
pub struct SyntaxContextId(u32);

impl SyntaxContextId {
    pub const ROOT: Self = Self(0);

    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Validated owner identity copied from a WASM component manifest.
pub struct ComponentId(String);

impl ComponentId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ComponentId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Stable subscription or macro hook identity.
pub struct HookId(String);

impl HookId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for HookId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Parser representation level transformed by a macro.
pub enum ExpansionKind {
    Text,
    Tree,
    Ast,
}

/// A location in the original document, optionally produced by another expansion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExpansionSite {
    pub original_range: TextRange,
    pub expansion: Option<ExpansionId>,
}

impl ExpansionSite {
    pub const fn original(original_range: TextRange) -> Self {
        Self {
            original_range,
            expansion: None,
        }
    }

    pub const fn expanded(original_range: TextRange, expansion: ExpansionId) -> Self {
        Self {
            original_range,
            expansion: Some(expansion),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// One accepted macro transformation and all provenance edges it introduced.
pub struct Expansion {
    pub id: ExpansionId,
    pub kind: ExpansionKind,
    pub component: ComponentId,
    pub hook: HookId,
    pub call_sites: Vec<ExpansionSite>,
    pub definition_site: Option<ExpansionSite>,
    pub syntax_context: SyntaxContextId,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Validated acyclic collection of macro expansions.
pub struct ExpansionGraph {
    expansions: BTreeMap<ExpansionId, Expansion>,
}

impl ExpansionGraph {
    pub fn new(
        expansions: impl IntoIterator<Item = Expansion>,
    ) -> Result<Self, ExpansionGraphError> {
        let mut by_id = BTreeMap::new();
        for expansion in expansions {
            if expansion.component.as_str().trim().is_empty() {
                return Err(ExpansionGraphError::BlankComponent { id: expansion.id });
            }
            if expansion.hook.as_str().trim().is_empty() {
                return Err(ExpansionGraphError::BlankHook { id: expansion.id });
            }
            let id = expansion.id;
            if by_id.insert(id, expansion).is_some() {
                return Err(ExpansionGraphError::DuplicateId { id });
            }
        }

        for expansion in by_id.values() {
            if expansion.call_sites.is_empty() {
                return Err(ExpansionGraphError::MissingCallSite { id: expansion.id });
            }
            for call_site in &expansion.call_sites {
                Self::validate_reference(&by_id, expansion.id, "call site", *call_site)?;
            }
            if let Some(definition_site) = expansion.definition_site {
                Self::validate_reference(&by_id, expansion.id, "definition site", definition_site)?;
            }
        }

        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        for start in by_id.keys().copied() {
            Self::validate_acyclic(&by_id, start, &mut visiting, &mut visited)?;
        }

        Ok(Self { expansions: by_id })
    }

    fn validate_reference(
        expansions: &BTreeMap<ExpansionId, Expansion>,
        id: ExpansionId,
        site: &'static str,
        reference: ExpansionSite,
    ) -> Result<(), ExpansionGraphError> {
        if let Some(referenced) = reference.expansion
            && !expansions.contains_key(&referenced)
        {
            return Err(ExpansionGraphError::UnknownReference {
                id,
                site,
                referenced,
            });
        }
        Ok(())
    }

    fn validate_acyclic(
        expansions: &BTreeMap<ExpansionId, Expansion>,
        id: ExpansionId,
        visiting: &mut BTreeSet<ExpansionId>,
        visited: &mut BTreeSet<ExpansionId>,
    ) -> Result<(), ExpansionGraphError> {
        if visited.contains(&id) {
            return Ok(());
        }
        if !visiting.insert(id) {
            return Err(ExpansionGraphError::Cycle { id });
        }
        let expansion = expansions
            .get(&id)
            .expect("all call-site references were validated");
        for parent in expansion
            .call_sites
            .iter()
            .filter_map(|site| site.expansion)
        {
            Self::validate_acyclic(expansions, parent, visiting, visited)?;
        }
        visiting.remove(&id);
        visited.insert(id);
        Ok(())
    }

    /// Looks up one expansion by stable ID.
    pub fn get(&self, id: ExpansionId) -> Option<&Expansion> {
        self.expansions.get(&id)
    }

    /// Returns whether an expansion ID exists.
    pub fn contains(&self, id: ExpansionId) -> bool {
        self.expansions.contains_key(&id)
    }

    /// Iterates expansions in stable ID order.
    pub fn iter(&self) -> impl Iterator<Item = &Expansion> {
        self.expansions.values()
    }

    /// Returns the number of accepted expansions.
    pub fn len(&self) -> usize {
        self.expansions.len()
    }

    /// Returns whether the graph contains no macro expansions.
    pub fn is_empty(&self) -> bool {
        self.expansions.is_empty()
    }

    /// Allocates the next representable expansion identity.
    pub fn next_id(&self) -> Result<ExpansionId, ExpansionGraphError> {
        let next = self
            .expansions
            .last_key_value()
            .map_or(1, |(id, _)| id.get().checked_add(1).unwrap_or(0));
        if next == 0 {
            Err(ExpansionGraphError::IdExhausted)
        } else {
            Ok(ExpansionId::new(next))
        }
    }

    /// Clones the graph with one validated expansion appended.
    pub fn with_expansion(&self, expansion: Expansion) -> Result<Self, ExpansionGraphError> {
        let mut expansions = self.expansions.values().cloned().collect::<Vec<_>>();
        expansions.push(expansion);
        Self::new(expansions)
    }

    /// Returns the primary call-site path, from the innermost expansion to the root.
    ///
    /// Follows the first call site at each expansion. Use [`Self::backtraces`]
    /// when combined origins must retain every parent path. Unknown IDs return `None`.
    pub fn backtrace(&self, id: ExpansionId) -> Option<Vec<&Expansion>> {
        let mut result = Vec::new();
        let mut current = Some(id);
        while let Some(current_id) = current {
            let expansion = self.expansions.get(&current_id)?;
            result.push(expansion);
            current = expansion.call_sites.first().and_then(|site| site.expansion);
        }
        Some(result)
    }

    /// Returns every distinct call-site path, each ordered innermost to root.
    ///
    /// Unlike [`Self::backtrace`], this visits every distinct parent expansion,
    /// including branches anchored directly to original text. Unknown IDs return `None`.
    pub fn backtraces(&self, id: ExpansionId) -> Option<Vec<Vec<&Expansion>>> {
        self.collect_backtraces(id)
    }

    fn collect_backtraces(&self, id: ExpansionId) -> Option<Vec<Vec<&Expansion>>> {
        let expansion = self.expansions.get(&id)?;
        let mut parents = Vec::new();
        for call_site in &expansion.call_sites {
            if !parents.contains(&call_site.expansion) {
                parents.push(call_site.expansion);
            }
        }

        let mut paths = Vec::new();
        for parent in parents {
            match parent {
                Some(parent) => {
                    for mut tail in self.collect_backtraces(parent)? {
                        let mut path = Vec::with_capacity(tail.len() + 1);
                        path.push(expansion);
                        path.append(&mut tail);
                        paths.push(path);
                    }
                }
                None => paths.push(vec![expansion]),
            }
        }
        Some(paths)
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
/// Invalid identity, owner, reference, or cycle in an expansion graph.
pub enum ExpansionGraphError {
    #[error("duplicate expansion ID {id}")]
    DuplicateId { id: ExpansionId },
    #[error("expansion {id} has no call site")]
    MissingCallSite { id: ExpansionId },
    #[error("expansion {id} has a blank component ID")]
    BlankComponent { id: ExpansionId },
    #[error("expansion {id} has a blank hook ID")]
    BlankHook { id: ExpansionId },
    #[error("expansion {id} has unknown {site} expansion {referenced}")]
    UnknownReference {
        id: ExpansionId,
        site: &'static str,
        referenced: ExpansionId,
    },
    #[error("expansion graph contains a cycle at expansion {id}")]
    Cycle { id: ExpansionId },
    #[error("no more expansion IDs are available")]
    IdExhausted,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expansion(id: u32, parent: Option<u32>) -> Expansion {
        Expansion {
            id: ExpansionId::new(id),
            kind: ExpansionKind::Text,
            component: ComponentId::from("test"),
            hook: HookId::from("expand"),
            call_sites: vec![ExpansionSite {
                original_range: TextRange::new(0, 3),
                expansion: parent.map(ExpansionId::new),
            }],
            definition_site: None,
            syntax_context: SyntaxContextId::new(id),
        }
    }

    #[test]
    fn returns_expansion_backtrace_from_inner_to_outer() {
        let graph = ExpansionGraph::new([
            expansion(1, None),
            expansion(2, Some(1)),
            expansion(3, Some(2)),
        ])
        .unwrap();

        let ids = graph
            .backtrace(ExpansionId::new(3))
            .unwrap()
            .into_iter()
            .map(|item| item.id.get())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![3, 2, 1]);
    }

    #[test]
    fn rejects_unknown_references_and_cycles() {
        assert_eq!(
            ExpansionGraph::new([expansion(1, Some(99))]).unwrap_err(),
            ExpansionGraphError::UnknownReference {
                id: ExpansionId::new(1),
                site: "call site",
                referenced: ExpansionId::new(99),
            }
        );

        assert!(matches!(
            ExpansionGraph::new([expansion(1, Some(2)), expansion(2, Some(1))]),
            Err(ExpansionGraphError::Cycle { .. })
        ));
    }

    #[test]
    fn rejects_duplicate_ids_and_blank_owners() {
        assert!(matches!(
            ExpansionGraph::new([expansion(1, None), expansion(1, None)]),
            Err(ExpansionGraphError::DuplicateId { .. })
        ));

        let mut blank = expansion(1, None);
        blank.component = ComponentId::from("  ");
        assert!(matches!(
            ExpansionGraph::new([blank]),
            Err(ExpansionGraphError::BlankComponent { .. })
        ));
    }

    #[test]
    fn returns_every_path_for_multi_origin_expansions() {
        let mut merged = expansion(3, None);
        merged.call_sites = vec![
            ExpansionSite::expanded(TextRange::new(0, 1), ExpansionId::new(1)),
            ExpansionSite::original(TextRange::new(1, 2)),
            ExpansionSite::expanded(TextRange::new(2, 3), ExpansionId::new(2)),
        ];
        let graph = ExpansionGraph::new([expansion(1, None), expansion(2, None), merged]).unwrap();

        let paths = graph
            .backtraces(ExpansionId::new(3))
            .unwrap()
            .into_iter()
            .map(|path| {
                path.into_iter()
                    .map(|item| item.id.get())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        assert_eq!(paths, [vec![3, 1], vec![3], vec![3, 2]]);
        assert_eq!(
            graph
                .backtrace(ExpansionId::new(3))
                .unwrap()
                .into_iter()
                .map(|item| item.id.get())
                .collect::<Vec<_>>(),
            [3, 1]
        );
    }

    #[test]
    fn rejects_expansions_without_call_sites() {
        let mut missing = expansion(1, None);
        missing.call_sites.clear();
        assert_eq!(
            ExpansionGraph::new([missing]).unwrap_err(),
            ExpansionGraphError::MissingCallSite {
                id: ExpansionId::new(1)
            }
        );
    }

    #[test]
    fn allocates_the_next_stable_id_and_extends_immutably() {
        let graph = ExpansionGraph::new([expansion(2, None)]).unwrap();
        assert_eq!(graph.next_id().unwrap(), ExpansionId::new(3));

        let extended = graph.with_expansion(expansion(3, Some(2))).unwrap();
        assert_eq!(graph.len(), 1);
        assert_eq!(extended.len(), 2);
        assert_eq!(
            extended
                .backtrace(ExpansionId::new(3))
                .unwrap()
                .into_iter()
                .map(|item| item.id.get())
                .collect::<Vec<_>>(),
            [3, 2]
        );
    }
}

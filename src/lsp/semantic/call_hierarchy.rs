//! Function edges use accepted call identities and native body membership.
//! Header/default expressions are not assigned to a runtime function body.
use super::*;
use lsp_types::{
    CallHierarchyIncomingCall, CallHierarchyItem, CallHierarchyOutgoingCall, SymbolKind,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug)]
pub(super) struct FunctionBody {
    node: u64,
    range: Range,
}
#[derive(Clone, Debug)]
pub(super) struct CallSite {
    caller: usize,
    occurrence: usize,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ItemIdentity {
    version: u8,
    stamp: String,
    source: String,
    definition: String,
    registration: String,
    signature: String,
}

pub(super) fn body(
    owner: &AstNode,
    tree: &skript_parser::RawTree,
    lines: &LineIndex,
    source: &str,
) -> Option<FunctionBody> {
    let start = usize::try_from(owner.span.virtual_range.start).ok()?;
    let mut matches = tree.nodes.iter().filter(|raw| {
        matches!(raw.kind, RawNodeKind::Section)
            && raw
                .code_span
                .as_ref()
                .is_some_and(|span| span.virtual_range.start == start)
    });
    let raw = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    let [origin] = raw.span.origins.as_slice() else {
        return None;
    };
    if origin.kind != skript_parser::OriginKind::Exact {
        return None;
    }
    let original = origin.original_range;
    let text = source.get(original.start..original.end)?;
    let start = original.start + text.len() - text.trim_start().len();
    let end = start + text.trim().len();
    if start >= end {
        return None;
    }
    Some(FunctionBody {
        node: owner.id,
        range: lines.range(source, TextRange::new(start, end)).ok()?,
    })
}

impl Document {
    pub(super) fn index_function_calls(&mut self, ast: &AstTree) {
        let nodes = ast
            .nodes
            .iter()
            .map(|n| (n.id, n))
            .collect::<HashMap<_, _>>();
        let mut owners = HashMap::<u64, Option<usize>>::new();
        for (index, symbol) in self.functions.iter().enumerate() {
            let Some(body) = &symbol.body else {
                continue;
            };
            let Some(root) = nodes.get(&body.node) else {
                continue;
            };
            let Ok(mut pending) = body_nodes(root) else {
                continue;
            };
            let mut seen = HashSet::new();
            while let Some(id) = pending.pop() {
                if !seen.insert(id) {
                    continue;
                }
                let Some(node) = nodes.get(&id) else {
                    continue;
                };
                // A nested declaration has its own body ownership, never its parent's.
                if node.kind == SyntaxKind::Structure {
                    continue;
                }
                owners
                    .entry(id)
                    .and_modify(|owner| *owner = None)
                    .or_insert(Some(index));
                pending.extend(node.children.iter().copied());
            }
        }
        self.calls = self
            .occurrences
            .iter()
            .enumerate()
            .filter_map(|(index, occurrence)| {
                if !matches!(occurrence.reference, Reference::Function { .. }) {
                    return None;
                }
                let caller = owners.get(&occurrence.node).copied().flatten()?;
                let body = self.functions[caller].body.as_ref()?;
                let range = self.lines.range(&self.source, occurrence.range).ok()?;
                (body.range.start <= range.start && range.end <= body.range.end).then_some(
                    CallSite {
                        caller,
                        occurrence: index,
                    },
                )
            })
            .collect();
    }
    fn hierarchy_item(&self, symbol: &FunctionSymbol, stamp: &str) -> Option<CallHierarchyItem> {
        let body = symbol.body.as_ref()?;
        let selection = symbol.name_range?;
        if body.range.start > selection.start || selection.end > body.range.end {
            return None;
        }
        let definition = &symbol.definition;
        Some(CallHierarchyItem {
            name: definition.name.clone(),
            kind: SymbolKind::FUNCTION,
            tags: None,
            detail: Some(definition.signature_key()),
            uri: self.uri.clone(),
            range: body.range,
            selection_range: selection,
            data: Some(
                serde_json::to_value(ItemIdentity {
                    version: 1,
                    stamp: stamp.into(),
                    source: self.source_witness.clone(),
                    definition: definition.definition_id.clone(),
                    registration: definition.registration_id.clone(),
                    signature: definition.signature_key(),
                })
                .ok()?,
            ),
        })
    }
    pub(in crate::lsp) fn prepare_hierarchy(
        &self,
        position: Position,
        project: &[&Document],
        stamp: &str,
    ) -> Option<Vec<CallHierarchyItem>> {
        let (owner, symbol) = self.function_at(position, project)?;
        Some(vec![owner.hierarchy_item(symbol, stamp)?])
    }
    fn hierarchy_target(
        &self,
        item: &CallHierarchyItem,
        stamp: &str,
    ) -> Result<&FunctionSymbol, String> {
        let identity: ItemIdentity =
            serde_json::from_value(item.data.clone().ok_or("Missing call hierarchy identity")?)
                .map_err(|_| "Invalid call hierarchy identity")?;
        if identity.version != 1
            || identity.stamp != stamp
            || identity.source != self.source_witness
            || item.uri != self.uri
        {
            return Err(
                "Call hierarchy item is from an obsolete document or project generation".into(),
            );
        }
        let mut matches = self.functions.iter().filter(|s| {
            s.definition.definition_id == identity.definition
                && s.definition.registration_id == identity.registration
                && s.definition.signature_key() == identity.signature
        });
        let symbol = matches
            .next()
            .ok_or("Call hierarchy declaration is no longer available")?;
        if matches.next().is_some() || self.hierarchy_item(symbol, stamp).as_ref() != Some(item) {
            return Err("Call hierarchy item no longer has a unique current binding".into());
        }
        Ok(symbol)
    }
    pub(in crate::lsp) fn hierarchy_incoming(
        &self,
        item: &CallHierarchyItem,
        project: &[&Document],
        stamp: &str,
    ) -> Result<Vec<CallHierarchyIncomingCall>, String> {
        let target = self.hierarchy_target(item, stamp)?;
        let mut output = Vec::new();
        for caller in
            std::iter::once(self).chain(project.iter().copied().filter(|d| d.uri != self.uri))
        {
            let mut groups = BTreeMap::<usize, Vec<Range>>::new();
            for call in &caller.calls {
                let occurrence = &caller.occurrences[call.occurrence];
                if caller.reference_binds(occurrence, self, target) {
                    groups
                        .entry(call.caller)
                        .or_default()
                        .push(caller.lines.range(&caller.source, occurrence.range)?);
                }
            }
            for (index, mut from_ranges) in groups {
                let Some(from) = caller.hierarchy_item(&caller.functions[index], stamp) else {
                    continue;
                };
                from_ranges.sort_by_key(|r| (r.start, r.end));
                from_ranges.dedup();
                output.push(CallHierarchyIncomingCall { from, from_ranges });
            }
        }
        output.sort_by(|a, b| {
            (a.from.uri.as_str(), a.from.selection_range.start)
                .cmp(&(b.from.uri.as_str(), b.from.selection_range.start))
        });
        Ok(output)
    }
    pub(in crate::lsp) fn hierarchy_outgoing(
        &self,
        item: &CallHierarchyItem,
        project: &[&Document],
        stamp: &str,
    ) -> Result<Vec<CallHierarchyOutgoingCall>, String> {
        let target = self.hierarchy_target(item, stamp)?;
        let resolver = super::navigation::FunctionResolver::new(self, project);
        let mut groups = BTreeMap::<
            (String, Position, String, String, String),
            (CallHierarchyItem, Vec<Range>),
        >::new();
        for call in &self.calls {
            if !std::ptr::eq(&self.functions[call.caller], target) {
                continue;
            }
            let occurrence = &self.occurrences[call.occurrence];
            let Some((owner, symbol)) = resolver.resolve(self, occurrence) else {
                continue;
            };
            let Some(to) = owner.hierarchy_item(symbol, stamp) else {
                continue;
            };
            let key = (
                to.uri.as_str().to_owned(),
                to.selection_range.start,
                symbol.definition.definition_id.clone(),
                symbol.definition.registration_id.clone(),
                symbol.definition.signature_key(),
            );
            groups
                .entry(key)
                .or_insert_with(|| (to, Vec::new()))
                .1
                .push(self.lines.range(&self.source, occurrence.range)?);
        }
        Ok(groups
            .into_values()
            .map(|(to, mut from_ranges)| {
                from_ranges.sort_by_key(|r| (r.start, r.end));
                from_ranges.dedup();
                CallHierarchyOutgoingCall { to, from_ranges }
            })
            .collect())
    }
}

//! Outline meaning comes from an authenticated provider record. Native AST
//! parentage and exact source maps supply geometry; source text is never parsed.
use super::*;
use lsp_types::{DocumentSymbol, SymbolKind};
use skript_parser::{MappedSource, RawTree};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Description {
    version: u32,
    name: String,
    kind: SymbolKind,
    detail: Option<String>,
    selection: Selection,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    start: usize,
    end: usize,
}

pub(super) fn collect(
    result: &DocumentParseResult,
    lines: &LineIndex,
    valid: impl Fn(&AstNode, &str) -> bool,
) -> Vec<DocumentSymbol> {
    let blocked = result
        .unresolved_spans()
        .map(|span| (span.virtual_range.start, span.virtual_range.end))
        .collect::<Vec<_>>();
    from_ast(
        &result.ast,
        &result.raw_tree,
        &result.source,
        lines,
        &blocked,
        valid,
    )
}

pub(super) fn from_ast(
    ast: &AstTree,
    raw: &RawTree,
    mapped: &MappedSource,
    lines: &LineIndex,
    blocked: &[(u64, u64)],
    valid: impl Fn(&AstNode, &str) -> bool,
) -> Vec<DocumentSymbol> {
    let nodes = ast
        .nodes
        .iter()
        .map(|n| (n.id, n))
        .collect::<HashMap<_, _>>();
    if nodes.len() != ast.nodes.len() {
        return Vec::new();
    }
    let mut geometry = HashMap::<u64, Vec<&skript_parser::RawNode>>::new();
    for node in &raw.nodes {
        if matches!(node.kind, RawNodeKind::Section | RawNodeKind::Simple)
            && let Some(code) = &node.code_span
        {
            geometry
                .entry(code.virtual_range.start as u64)
                .or_default()
                .push(node);
        }
    }
    let raw_parents = raw
        .nodes
        .iter()
        .map(|node| (node.id, node.parent))
        .collect::<HashMap<_, _>>();
    let mut raw_symbols = Vec::new();
    let mut used_geometry = HashSet::new();
    let mut stack: Vec<(u64, Option<usize>)> = ast
        .roots
        .iter()
        .rev()
        .map(|id| (*id, None))
        .collect::<Vec<_>>();
    let mut visited = HashSet::new();
    let mut symbols: Vec<(DocumentSymbol, Option<usize>)> = Vec::new();
    while let Some((id, parent)) = stack.pop() {
        if !visited.insert(id) {
            return Vec::new();
        }
        let Some(node) = nodes.get(&id) else {
            return Vec::new();
        };
        let symbol = (|| {
            let mut entries = node
                .metadata
                .iter()
                .filter(|e| e.key == parser_wasm::DOCUMENT_SYMBOL_METADATA);
            let entry = entries.next()?;
            if entries.next().is_some() || !valid(node, entry.owner_component_id.as_deref()?) {
                return None;
            }
            if !matches!(node.kind, SyntaxKind::Structure | SyntaxKind::Section) {
                return None;
            }
            if blocked.iter().any(|(start, end)| {
                node.span.virtual_range.start < *end && *start < node.span.virtual_range.end
            }) {
                return None;
            }
            let description: Description = serde_json::from_str(&entry.value).ok()?;
            // SymbolKind is forward-compatible in lsp-types; this schema is not.
            let kind = serde_json::to_value(description.kind).ok()?.as_u64()?;
            if description.version != 1
                || !(1..=26).contains(&kind)
                || description.name.trim().is_empty()
            {
                return None;
            }
            let start = usize::try_from(node.span.virtual_range.start).ok()?;
            let end = usize::try_from(node.span.virtual_range.end).ok()?;
            let header = mapped.virtual_source().get(start..end)?;
            if header != node.text || description.selection.start >= description.selection.end {
                return None;
            }
            header.get(description.selection.start..description.selection.end)?;
            // Match a single current physical node, including one-line structures.
            let [raw] = geometry.get(&node.span.virtual_range.start)?.as_slice() else {
                return None;
            };
            let code = raw.code_span.as_ref()?;
            if code.virtual_range.end < end {
                return None;
            }
            let full = raw.span.virtual_range;
            if full.start > start || full.end < end {
                return None;
            }
            let selection = TextRange::new(
                start.checked_add(description.selection.start)?,
                start.checked_add(description.selection.end)?,
            );
            let selected = exact(mapped, selection)?;
            let full = exact(mapped, full)?;
            let text = mapped.original().get(full.start..full.end)?;
            let full_start = full.start + text.len() - text.trim_start().len();
            let full = TextRange::new(full_start, full_start + text.trim().len());
            if full.start >= full.end || selected.start < full.start || selected.end > full.end {
                return None;
            }
            let range = lines.range(mapped.original(), full).ok()?;
            if let Some(parent) = parent {
                let mut ancestor = raw.parent;
                let mut seen = HashSet::new();
                while ancestor.is_some_and(|id| id != raw_symbols[parent]) {
                    let id = ancestor?;
                    if !seen.insert(id) {
                        return None;
                    }
                    ancestor = *raw_parents.get(&id)?;
                }
                if ancestor.is_none() {
                    return None;
                }
                let enclosing = &symbols[parent].0.range;
                if enclosing.start > range.start || enclosing.end < range.end {
                    return None;
                }
            }
            #[allow(deprecated)]
            Some((
                DocumentSymbol {
                    name: description.name,
                    detail: description.detail.filter(|d| d != &node.text),
                    kind: description.kind,
                    tags: None,
                    deprecated: None,
                    range,
                    selection_range: lines.range(mapped.original(), selected).ok()?,
                    children: None,
                },
                raw.id,
            ))
        })();
        let parent = if let Some((symbol, raw)) = symbol {
            if !used_geometry.insert(raw) {
                return Vec::new();
            }
            raw_symbols.push(raw);
            let index = symbols.len();
            symbols.push((symbol, parent));
            Some(index)
        } else {
            parent
        };
        stack.extend(node.children.iter().rev().map(|id| (*id, parent)));
    }
    // Parent precedes child in traversal; build iteratively to avoid stack growth.
    let mut built = symbols
        .into_iter()
        .map(|(s, p)| (Some(s), p))
        .collect::<Vec<_>>();
    let mut roots = Vec::new();
    for index in (0..built.len()).rev() {
        let mut symbol = built[index].0.take().expect("each symbol is moved once");
        if let Some(children) = &mut symbol.children {
            children.sort_by_key(|s| (s.range.start, s.range.end));
        }
        if let Some(parent) = built[index].1 {
            built[parent]
                .0
                .as_mut()
                .unwrap()
                .children
                .get_or_insert_with(Vec::new)
                .push(symbol);
        } else {
            roots.push(symbol);
        }
    }
    roots.sort_by_key(|s| (s.range.start, s.range.end));
    roots
}

fn exact(mapped: &MappedSource, range: TextRange) -> Option<TextRange> {
    let span = mapped.map_range(range).ok()?;
    let [origin] = span.origins.as_slice() else {
        return None;
    };
    if origin.kind != skript_parser::OriginKind::Exact
        || origin.original_range.is_empty()
        || mapped
            .original()
            .get(origin.original_range.start..origin.original_range.end)?
            != mapped.virtual_source().get(range.start..range.end)?
    {
        return None;
    }
    Some(origin.original_range)
}

impl Document {
    pub(in crate::lsp) fn document_symbols(&self, hierarchical: bool) -> serde_json::Value {
        if hierarchical {
            return serde_json::to_value(&self.symbols).expect("document symbols serialize");
        }
        let mut flat = Vec::new();
        let mut stack = self
            .symbols
            .iter()
            .rev()
            .map(|s| (s, None::<&str>))
            .collect::<Vec<_>>();
        while let Some((symbol, parent)) = stack.pop() {
            let mut item = serde_json::json!({"name":symbol.name,"kind":symbol.kind,
                "location":{"uri":self.uri,"range":symbol.selection_range}});
            if let Some(parent) = parent {
                item["containerName"] = parent.into();
            }
            flat.push(item);
            stack.extend(
                symbol
                    .children
                    .iter()
                    .flatten()
                    .rev()
                    .map(|s| (s, Some(symbol.name.as_str()))),
            );
        }
        flat.into()
    }
}

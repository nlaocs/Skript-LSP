//! Optional provider roles for exact, adopted source headers.
//!
//! The provider owns meaning, while the final AST owns all source geometry.
//! A record cannot color unrelated text, executable children, or a stale revision.
use super::*;
use serde::Deserialize;

const KEY: &str = "parser.source-roles.v1";
const MAX_BYTES: usize = 64 * 1024;
const MAX_SPANS: usize = 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    targets: Vec<Target>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    text: String,
    start: u64,
    end: u64,
    spans: Vec<Span>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Span {
    start: usize,
    end: usize,
    role: String,
}

pub(super) fn collect(
    ast: &AstTree,
    source: &str,
    dynamic: Option<&DynamicSyntaxSnapshot>,
) -> HashMap<u64, Vec<(TextRange, Role)>> {
    let mut output = HashMap::new();
    let Some(dynamic) = dynamic else {
        return output;
    };
    let nodes: HashMap<_, _> = ast.nodes.iter().map(|node| (node.id, node)).collect();
    for node in &ast.nodes {
        if !matches!(
            node.kind,
            SyntaxKind::Structure | SyntaxKind::Expression | SyntaxKind::Type
        ) || !complete_capture_geometry(node)
        {
            continue;
        }
        let Some((registration, definition, _)) = registered_pattern_identity(node) else {
            continue;
        };
        let mut owners = dynamic.definitions.iter().filter(|(id, value)| {
            id.qualified() == registration
                && definition == registration
                && value.id == **id
                && super::super::hover::same_kind(value.kind, node.kind)
        });
        let Some((owner, _)) = owners.next() else {
            continue;
        };
        if owners.next().is_some() {
            continue;
        }
        let mut candidates = dynamic.candidates.iter().filter(|candidate| {
            candidate.source == SyntaxCandidateSource::Dynamic(owner.clone())
                && super::super::hover::same_kind(candidate.kind, node.kind)
        });
        if candidates.next().is_none() || candidates.next().is_some() {
            continue;
        }
        let mut records = node.metadata.iter().filter(|entry| entry.key == KEY);
        let Some(record) = records.next() else {
            continue;
        };
        if records.next().is_some()
            || record.owner_component_id.as_deref() != Some(owner.component_id.as_str())
            || record.value.len() > MAX_BYTES
        {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Record>(&record.value) else {
            continue;
        };
        if let Some(pieces) = validated(node, &nodes, source, record) {
            for (id, spans) in pieces {
                output.entry(id).or_insert_with(Vec::new).extend(spans);
            }
        }
    }
    output
}

fn exact_range(node: &AstNode, source: &str) -> Option<TextRange> {
    if node.span.origins.len() != 1 || node.span.origins[0].kind != OriginKind::Exact {
        return None;
    }
    let range = origins(&node.span).ok()?.into_iter().next()?;
    (source.get(range.start..range.end)? == node.text
        && node
            .span
            .virtual_range
            .end
            .checked_sub(node.span.virtual_range.start)?
            == node.text.len() as u64)
        .then_some(range)
}

fn validated(
    root: &AstNode,
    nodes: &HashMap<u64, &AstNode>,
    source: &str,
    record: Record,
) -> Option<HashMap<u64, Vec<(TextRange, Role)>>> {
    if record.version != 1
        || record.targets.is_empty()
        || record.targets.len() > MAX_SPANS
        || record
            .targets
            .iter()
            .map(|target| target.spans.len())
            .sum::<usize>()
            > MAX_SPANS
    {
        return None;
    }
    exact_range(root, source)?;
    let mut allowed = vec![root];
    if root.kind == SyntaxKind::Structure {
        allowed.extend(
            root.children
                .iter()
                .filter_map(|id| nodes.get(id).copied())
                .filter(|node| {
                    node.kind == SyntaxKind::Structure
                        && node.summary.is_none()
                        && node.syntax_id.starts_with("parser:structure-entry.")
                        && node.metadata.iter().any(|entry| {
                            entry.key == "parser.entry-kind" && entry.owner_component_id.is_none()
                        })
                }),
        );
    }
    let mut seen = HashSet::new();
    let mut output = HashMap::new();
    for target in record.targets {
        let mut matches = allowed.iter().copied().filter(|node| {
            node.span.virtual_range.start == target.start
                && node.span.virtual_range.end == target.end
                && node.text == target.text
        });
        let node = matches.next()?;
        if matches.next().is_some() || !seen.insert(node.id) {
            return None;
        }
        let range = exact_range(node, source)?;
        let mut spans = Vec::new();
        for span in target.spans {
            if span.start >= span.end || target.text.get(span.start..span.end).is_none() {
                return None;
            }
            let role = match span.role.as_str() {
                "class" => Role::Class,
                "type" => Role::Type,
                "method" => Role::Method,
                "property" => Role::Property,
                "parameter" => Role::Parameter,
                "keyword" => Role::Keyword,
                _ => return None,
            };
            spans.push((
                TextRange::new(range.start + span.start, range.start + span.end),
                role,
            ));
        }
        spans.sort_by_key(|(range, _)| (range.start, range.end));
        if spans.windows(2).any(|pair| pair[0].0.end > pair[1].0.start) {
            return None;
        }
        // Executable/capture children retain their own independent semantics.
        if node
            .children
            .iter()
            .filter_map(|id| nodes.get(id))
            .any(|child| {
                origins(&child.span).is_ok_and(|ranges| {
                    ranges.iter().any(|child| {
                        spans
                            .iter()
                            .any(|(span, _)| span.start < child.end && child.start < span.end)
                    })
                })
            })
        {
            return None;
        }
        output.insert(node.id, spans);
    }
    // The exact adopted root header is the fence for every child declaration.
    seen.contains(&root.id).then_some(output)
}

#[cfg(test)]
mod tests {
    use super::super::tests::node;
    use super::*;

    fn fixture() -> (String, Vec<AstNode>, serde_json::Value) {
        let source = "class Widget\n    String greet(String name)\n        return name".to_owned();
        let mut root = node(1, SyntaxKind::Structure, "structure", &source, 0, 12);
        root.children = vec![2];
        let mut entry = node(2, SyntaxKind::Structure, "", &source, 17, 42);
        entry.summary = None;
        entry.syntax_id = "parser:structure-entry.section".into();
        entry.metadata.push(
            parser_wasm::bindings::nlaocs::skript_parser_addon::types::MetadataEntry {
                key: "parser.entry-kind".into(),
                value: "section".into(),
                owner_component_id: None,
            },
        );
        let record = serde_json::json!({"version":1,"targets":[
            {"text":root.text,"start":0,"end":12,"spans":[{"start":6,"end":12,"role":"class"}]},
            {"text":entry.text,"start":17,"end":42,"spans":[
                {"start":0,"end":6,"role":"type"},{"start":7,"end":12,"role":"method"},
                {"start":20,"end":24,"role":"parameter"}]}]});
        (source, vec![root, entry], record)
    }

    #[test]
    fn exact_direct_headers_supply_generic_roles() {
        let (source, nodes, record) = fixture();
        let map = nodes.iter().map(|node| (node.id, node)).collect();
        let roles = validated(
            &nodes[0],
            &map,
            &source,
            serde_json::from_value(record).unwrap(),
        )
        .unwrap();
        assert_eq!(roles[&1], vec![(TextRange::new(6, 12), Role::Class)]);
        assert_eq!(roles[&2].len(), 3);
    }

    #[test]
    fn stale_unrelated_unfenced_or_overlapping_roles_are_ignored() {
        let (source, nodes, record) = fixture();
        let map = nodes.iter().map(|node| (node.id, node)).collect();
        for mutation in 0..7 {
            let mut record = record.clone();
            match mutation {
                0 => record["targets"][1]["text"] = "stale".into(),
                1 => record["targets"][1]["start"] = 18.into(),
                2 => {
                    record["targets"].as_array_mut().unwrap().remove(0);
                }
                3 => record["targets"][1]["spans"][0]["role"] = "unknown".into(),
                4 => record["targets"][1]["spans"][0]["end"] = 50.into(),
                5 => record["targets"][1]["spans"][1]["start"] = 4.into(),
                _ => record["version"] = 2.into(),
            }
            assert!(
                validated(
                    &nodes[0],
                    &map,
                    &source,
                    serde_json::from_value(record).unwrap()
                )
                .is_none(),
                "mutation {mutation}"
            );
        }
    }

    #[test]
    fn current_dynamic_provider_must_own_the_record() {
        let (source, mut nodes, record) = fixture();
        let id = syntaxes::DynamicSyntaxId::new("example.provider", "class");
        let snapshot = super::super::tests::dynamic_structure_snapshot(
            &id,
            "document",
            7,
            syntaxes::SyntaxKind::Structure,
            "class Widget",
        );
        let root = &mut nodes[0];
        root.syntax_id = id.qualified();
        let summary = root.summary.as_mut().unwrap();
        summary.definition_id = Some(id.qualified());
        summary.registration_id = Some(id.qualified());
        summary.pattern_index = Some(0);
        source_geometry::retain_pattern_captures(root, &[]);
        root.metadata.push(
            parser_wasm::bindings::nlaocs::skript_parser_addon::types::MetadataEntry {
                key: KEY.into(),
                value: record.to_string(),
                owner_component_id: Some(id.component_id.clone()),
            },
        );
        let ast = AstTree {
            roots: vec![1],
            nodes,
        };
        assert_eq!(collect(&ast, &source, Some(&snapshot)).len(), 2);
        assert!(
            collect(
                &ast,
                &source,
                current_dynamic_snapshot(&snapshot, "document", 8)
            )
            .is_empty()
        );
        for owner in [None, Some("another.provider".to_owned())] {
            let mut bad = ast.clone();
            bad.nodes[0]
                .metadata
                .iter_mut()
                .find(|entry| entry.key == KEY)
                .unwrap()
                .owner_component_id = owner;
            assert!(collect(&bad, &source, Some(&snapshot)).is_empty());
        }
        let mut duplicate = snapshot.clone();
        duplicate.candidates.push(duplicate.candidates[0].clone());
        assert!(collect(&ast, &source, Some(&duplicate)).is_empty());
    }

    #[test]
    fn foreign_children_and_nonexact_source_do_not_supply_roles() {
        let (source, nodes, record) = fixture();
        for mutation in 0..4 {
            let mut nodes = nodes.clone();
            match mutation {
                0 => nodes[0].children.clear(),
                1 => nodes[1].syntax_id = "foreign.node".into(),
                2 => nodes[1].span.origins[0].kind = OriginKind::Anchored,
                _ => nodes[1].text = "stale".into(),
            }
            let map = nodes.iter().map(|node| (node.id, node)).collect();
            assert!(
                validated(
                    &nodes[0],
                    &map,
                    &source,
                    serde_json::from_value(record.clone()).unwrap()
                )
                .is_none()
            );
        }
    }

    #[test]
    fn roles_replace_parent_pieces_and_preserve_utf16_and_fault_exclusion() {
        let source = "class 型😀";
        let root = node(
            1,
            SyntaxKind::Structure,
            "structure",
            source,
            0,
            source.len(),
        );
        let roles = HashMap::from([(1, vec![(TextRange::new(6, source.len()), Role::Class)])]);
        let ast = AstTree {
            roots: vec![1],
            nodes: vec![root],
        };
        let document =
            Document::from_ast_with_roles(&ast, source, vec![], HashSet::from([1]), roles.clone())
                .unwrap();
        assert_eq!(document.tokens.len(), 2);
        assert_eq!(
            document.tokens[1],
            (
                lsp_types::Range::new(
                    lsp_types::Position::new(0, 6),
                    lsp_types::Position::new(0, 9)
                ),
                Role::Class
            )
        );
        let blocked = Document::from_ast_with_roles(
            &ast,
            source,
            vec![TextRange::new(6, source.len())],
            HashSet::from([1]),
            roles,
        )
        .unwrap();
        assert_eq!(blocked.tokens.len(), 1);
        assert_eq!(blocked.tokens[0].1, Role::Structure);
    }

    #[test]
    fn invalid_utf8_boundaries_do_not_supply_roles() {
        let source = "class 型";
        let root = node(
            1,
            SyntaxKind::Structure,
            "structure",
            source,
            0,
            source.len(),
        );
        let nodes = HashMap::from([(1, &root)]);
        let record = serde_json::json!({"version":1,"targets":[{
            "text":source,"start":0,"end":source.len(),"spans":[{"start":6,"end":8,"role":"class"}]}]});
        assert!(
            validated(
                &root,
                &nodes,
                source,
                serde_json::from_value(record).unwrap()
            )
            .is_none()
        );
    }
}

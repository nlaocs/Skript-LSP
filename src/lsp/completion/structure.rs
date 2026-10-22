//! Editing scopes from the current native Structure result and its frozen declarations.
use super::*;
use skript_parser::{StructureBody, StructureDocumentNode, StructureEntryValue};
use syntaxes::{EntryData, EntryKind, EntryValidator};

#[derive(Clone, Copy)]
pub(super) enum Scope {
    Pending,
    Statements,
    Conditions,
    Entries(usize),
    Expressions,
    Raw,
}
pub(super) struct EntryScope {
    pub(super) declarations: Vec<EntryData>,
    pub(super) present: Vec<(usize, RawNodeId)>,
    pub(super) registration: String,
}

pub(super) fn apply(
    result: &DocumentParseResult,
    context: &mut BTreeMap<u32, Line>,
) -> Vec<EntryScope> {
    let mut scopes = result
        .raw_tree
        .iter()
        .filter(|n| n.kind == RawNodeKind::Section)
        .map(|n| (n.id, Scope::Raw))
        .collect::<HashMap<_, _>>();
    let mut entries = Vec::new();
    let mut condition_entries = HashSet::new();
    for root in &result.syntax.roots {
        let StructureDocumentNode::Structure(matches) = root else {
            continue;
        };
        let candidate = matches
            .selected
            .as_ref()
            .or_else(|| matches.unknown.as_ref()?.partial.as_deref());
        let Some(candidate) = candidate else { continue };
        let Some(raw) = result.raw_tree.get(candidate.raw_node_id) else {
            continue;
        };
        if exact_native(&raw.line.span).is_none() {
            continue;
        }
        if let StructureBody::Entries(values) = &candidate.body {
            condition_entry_ids(
                values,
                &candidate.entry_body_modes,
                &[],
                &mut condition_entries,
            );
        }
        match &candidate.body {
            StructureBody::Entries(values) => {
                if let Some(validator) = &candidate.entry_validator {
                    validator_scope(
                        &result.raw_tree,
                        raw.id,
                        validator,
                        values,
                        &candidate.matched.registration_id,
                        &mut scopes,
                        &mut entries,
                        context,
                    );
                }
            }
            StructureBody::Trigger(body) => {
                scopes.insert(raw.id, Scope::Statements);
                executable_scopes(body, &mut scopes);
            }
            StructureBody::None | StructureBody::Raw(_) => {
                scopes.insert(raw.id, Scope::Raw);
            }
        }
    }
    for id in condition_entries {
        scopes.insert(id, Scope::Conditions);
    }
    for line in context.values_mut() {
        if line.root {
            continue;
        }
        line.scope = line
            .ancestors
            .iter()
            .find_map(|id| scopes.get(id).copied())
            .unwrap_or(Scope::Raw);
    }
    entries
}

fn validator_scope(
    tree: &skript_parser::RawTree,
    parent: RawNodeId,
    validator: &EntryValidator,
    values: &[skript_parser::StructureEntry],
    registration: &str,
    scopes: &mut HashMap<RawNodeId, Scope>,
    entries: &mut Vec<EntryScope>,
    context: &mut BTreeMap<u32, Line>,
) {
    let Some(_) = tree.get(parent) else {
        return;
    };
    let present = values
        .iter()
        .filter_map(|entry| Some((entry.declaration_index, entry.raw_node_id?)))
        .collect();
    scopes.insert(parent, Scope::Entries(entries.len()));
    entries.push(EntryScope {
        declarations: validator.entry_data.clone(),
        present,
        registration: registration.to_owned(),
    });
    for value in values {
        let Some(id) = value.raw_node_id else {
            continue;
        };
        let Some(data) = validator.entry_data.get(value.declaration_index) else {
            continue;
        };
        match &value.value {
            StructureEntryValue::Container(children) => {
                if let Some(nested) = &data.nested_validator {
                    validator_scope(
                        tree,
                        id,
                        nested,
                        children,
                        registration,
                        scopes,
                        entries,
                        context,
                    );
                }
            }
            StructureEntryValue::Trigger(body) => {
                scopes.insert(id, Scope::Statements);
                executable_scopes(body, scopes);
            }
            StructureEntryValue::Section(_) | StructureEntryValue::Unknown(_) => {
                scopes.insert(id, Scope::Raw);
            }
            _ => {}
        }
    }
}

fn condition_entry_ids(
    values: &[skript_parser::StructureEntry],
    modes: &BTreeMap<Vec<String>, skript_parser::SectionBodyMode>,
    path: &[String],
    output: &mut HashSet<RawNodeId>,
) {
    for entry in values {
        let mut path = path.to_vec();
        path.push(entry.key.clone());
        if modes.get(&path) == Some(&skript_parser::SectionBodyMode::Conditions) {
            if let Some(id) = entry.raw_node_id {
                output.insert(id);
            }
        }
        if let StructureEntryValue::Container(children) = &entry.value {
            condition_entry_ids(children, modes, &path, output);
        }
    }
}

fn executable_scopes(
    body: &[skript_parser::SectionBodyNode],
    scopes: &mut HashMap<RawNodeId, Scope>,
) {
    for node in body {
        if let skript_parser::SectionBodyNode::Section(matches) = node {
            if let Some(section) = &matches.selected {
                scopes.insert(
                    section.raw_node_id,
                    if section.body_mode == skript_parser::SectionBodyMode::Conditions {
                        Scope::Conditions
                    } else {
                        Scope::Statements
                    },
                );
                executable_scopes(&section.body, scopes);
            }
        }
    }
}

impl Document {
    pub(super) fn structure_query_line(&self, line: &Line, offset: usize) -> Line {
        let mut query = line.clone();
        if let Scope::Entries(index) = line.scope {
            let text = &self.source[line.code_start..line.range.end];
            let scope = &self.entry_scopes[index];
            let selected = line.node.and_then(|id| {
                scope
                    .present
                    .iter()
                    .find(|(_, node)| *node == id)
                    .map(|(index, _)| *index)
            });
            let declarations = scope
                .declarations
                .iter()
                .enumerate()
                .filter(|(index, _)| selected.is_none_or(|selected| selected == *index))
                .collect::<Vec<_>>();
            // A partial edit without a parsed owner must not pick between duplicate keys.
            for (_, data) in declarations.iter().copied().filter(|(_, data)| {
                selected.is_some()
                    || declarations
                        .iter()
                        .filter(|(_, other)| other.key.eq_ignore_ascii_case(&data.key))
                        .count()
                        == 1
            }) {
                if matches!(
                    data.kind,
                    EntryKind::Trigger | EntryKind::Container | EntryKind::Section
                ) {
                    continue;
                }
                let separator = data.separator.as_deref().unwrap_or(": ");
                let Some(key) = text.get(..data.key.len()) else {
                    continue;
                };
                if !key.eq_ignore_ascii_case(&data.key)
                    || !text[data.key.len()..].starts_with(separator)
                {
                    continue;
                }
                let start = line.code_start + data.key.len() + separator.len();
                if offset < start {
                    continue;
                }
                query.code_start = start;
                query.scope =
                    if data.kind == EntryKind::Expression && data.flags.unwrap_or(3) & 1 != 0 {
                        Scope::Expressions
                    } else {
                        Scope::Raw
                    };
                break;
            }
        }
        query
    }

    pub(super) fn entry_completion(
        &self,
        line: &Line,
        offset: usize,
        index: usize,
        snippets: bool,
    ) -> Result<CompletionList, String> {
        let scope = &self.entry_scopes[index];
        let prefix = &self.source[line.code_start..offset];
        let mut items = Vec::new();
        let end = line
            .comments
            .iter()
            .map(|r| r.start)
            .filter(|start| *start >= offset)
            .min()
            .unwrap_or(line.range.end);
        let suffix = &self.source[offset..end];
        // Preserve an existing separator, value and comment when editing only its key.
        let separators = scope
            .declarations
            .iter()
            .map(|d| d.separator.as_deref().unwrap_or(": ").trim_end())
            .filter(|s| !s.is_empty());
        let separator_at = separators.filter_map(|s| suffix.find(s)).min();
        for (index, data) in scope.declarations.iter().enumerate() {
            if !starts(&data.key, prefix)
                || !data.multiple
                    && scope
                        .present
                        .iter()
                        .any(|(seen, node)| *seen == index && Some(*node) != line.node)
            {
                continue;
            }
            let section = matches!(
                data.kind,
                EntryKind::Trigger | EntryKind::Container | EntryKind::Section
            );
            let (insert, end, snippet) = if let Some(separator) = separator_at {
                (data.key.clone(), offset + separator, false)
            } else {
                let separator = if section {
                    ":"
                } else {
                    data.separator.as_deref().unwrap_or(": ")
                };
                let mut insert = if !section && snippets {
                    escape(&data.key)
                } else {
                    data.key.clone()
                };
                if !section && snippets {
                    insert.push_str(&escape(separator));
                } else {
                    insert.push_str(separator);
                }
                if !section && snippets {
                    insert.push_str("${1:value}");
                }
                let replace_end = self.source[offset..end].trim_end().len() + offset;
                (insert, replace_end, !section && snippets)
            };
            let detail = format!(
                "Structure entry / {} / {:?}",
                if data.optional {
                    "optional"
                } else {
                    "required"
                },
                data.kind
            );
            let mut item = self.item(
                &data.key,
                &insert,
                &detail,
                &scope.registration,
                CompletionItemKind::PROPERTY,
                TextRange::new(line.code_start, end),
                snippet,
            )?;
            item.filter_text = Some(data.key.clone());
            item.sort_text = Some(format!("{}:{index:04}", if data.optional { 1 } else { 0 }));
            item.data = Some(
                serde_json::json!({"structureEntry":true,"registrationId":scope.registration,"entryKey":data.key}),
            );
            items.push(item);
        }
        let limited = items.len() > MAX_ITEMS;
        items.truncate(MAX_ITEMS);
        Ok(CompletionList {
            is_incomplete: limited,
            items,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_key_declarations_track_the_selected_index_not_every_matching_node() {
        let source: Arc<str> = Arc::from("root:\n    name: first\n    ");
        let mapped = skript_parser::MappedSource::identity(Arc::clone(&source));
        let tree = skript_parser::parse_raw_tree(
            &mapped,
            skript_parser::RawTreeOptions::for_skript_version(2, 15),
        );
        let raw = tree
            .iter()
            .find(|node| node.kind == RawNodeKind::Simple)
            .unwrap();
        let data = EntryData {
            key: "name".into(),
            default_value: None,
            optional: false,
            multiple: false,
            entry_data_class: syntaxes::ClassName("test.Entry".into()),
            kind: EntryKind::KeyValue,
            separator: Some(": ".into()),
            value_type: None,
            string_mode: None,
            return_types: Vec::new(),
            flags: None,
            nested_validator: None,
        };
        let validator = EntryValidator {
            entry_data: vec![data.clone(), data.clone()],
        };
        let span = raw.code_span.clone().unwrap();
        let values = vec![skript_parser::StructureEntry {
            raw_node_id: Some(raw.id),
            declaration_index: 0,
            key_span: None,
            key: data.key.clone(),
            entry_data_class: data.entry_data_class.clone(),
            kind: data.kind,
            source: raw.text.clone(),
            span: skript_parser::MatchSpan {
                local_range: span.virtual_range,
                mapped: span,
            },
            defaulted: false,
            value: StructureEntryValue::Raw("first".into()),
        }];
        let mut document = Document::test_fixture(source);
        let mut scopes = HashMap::new();
        validator_scope(
            &tree,
            tree.roots[0],
            &validator,
            &values,
            "owner",
            &mut scopes,
            &mut document.entry_scopes,
            &mut document.context,
        );
        assert_eq!(document.entry_scopes[0].present, [(0, raw.id)]);
        document.context.get_mut(&2).unwrap().scope = Scope::Entries(0);
        let items = document.at(Position::new(2, 4), true).unwrap().items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "name");
    }

    #[test]
    fn entry_value_scope_uses_selected_declaration_and_rejects_ambiguous_recovery() {
        let source: Arc<str> = Arc::from("root:\n    name: first\n    name: sin(");
        let mapped = skript_parser::MappedSource::identity(Arc::clone(&source));
        let tree = skript_parser::parse_raw_tree(
            &mapped,
            skript_parser::RawTreeOptions::for_skript_version(2, 15),
        );
        let node = tree
            .iter()
            .filter(|node| node.kind == RawNodeKind::Simple)
            .last()
            .unwrap()
            .id;
        let raw = EntryData {
            key: "name".into(),
            default_value: None,
            optional: true,
            multiple: false,
            entry_data_class: syntaxes::ClassName("test.Entry".into()),
            kind: EntryKind::KeyValue,
            separator: Some(": ".into()),
            value_type: None,
            string_mode: None,
            return_types: Vec::new(),
            flags: None,
            nested_validator: None,
        };
        let mut expression = raw.clone();
        expression.kind = EntryKind::Expression;
        expression.flags = Some(1);
        let mut document = Document::test_fixture(source);
        document.entry_scopes.push(EntryScope {
            declarations: vec![raw, expression],
            present: vec![(1, node)],
            registration: "owner".into(),
        });
        let line = document.context.get_mut(&2).unwrap();
        line.scope = Scope::Entries(0);
        line.node = Some(node);
        let line = line.clone();
        let offset = line.range.end;
        assert!(matches!(
            document.structure_query_line(&line, offset).scope,
            Scope::Expressions
        ));
        document.entry_scopes[0].present[0].0 = 0;
        assert!(matches!(
            document.structure_query_line(&line, offset).scope,
            Scope::Raw
        ));
        document.entry_scopes[0].present.clear();
        assert!(matches!(
            document.structure_query_line(&line, offset).scope,
            Scope::Entries(0)
        ));
    }

    #[test]
    fn conditions_scope_excludes_effect_and_section_templates() {
        let mut document = Document::test_fixture(Arc::from("root:\n    i"));
        document.context.get_mut(&1).unwrap().scope = Scope::Conditions;
        let index = Arc::get_mut(&mut document.index).unwrap();
        index.templates.clear();
        for (kind, text) in [
            (SyntaxKind::Effect, "inject"),
            (SyntaxKind::Section, "if"),
            (SyntaxKind::Condition, "is set"),
        ] {
            index.templates.push(Template {
                finite_capture: false,
                documentation: None,
                kind,
                label: text.into(),
                detail: "fixture".into(),
                identity: text.into(),
                leading: text.into(),
                folded: text.into(),
                parts: vec![Part::Text(text.into())],
            });
        }
        let result = document.at(Position::new(1, 5), false).unwrap();
        assert!(!result.items.is_empty());
        assert!(
            result
                .items
                .iter()
                .all(|item| item.data.as_ref().unwrap()["syntaxKind"] == "Condition")
        );
    }
}

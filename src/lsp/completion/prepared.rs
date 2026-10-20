//! Current-header syntax proposals, isolated from final references and signatures.
use super::*;
use skript_parser::{StructureBodyMode, StructureHeaderPreparation};
use syntaxes::{EntryKind, EntryValidator};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::lsp) struct OwnerAttempt {
    pub root: RawNodeId,
    pub registration: String,
    pub pattern: usize,
}

/// This view has no semantic reference history and cannot answer signature queries.
pub(in crate::lsp) struct PreparedDocument {
    document: Document,
    owners: HashMap<RawNodeId, OwnerAttempt>,
    revoked: HashSet<RawNodeId>,
}

pub(in crate::lsp) enum PreparedQuery {
    Ready(CompletionList),
    NeedsAnalysis,
}

impl PreparedDocument {
    pub fn new(
        source: Arc<str>,
        index: Arc<CatalogIndex>,
        mapped: &skript_parser::MappedSource,
        tree: &skript_parser::RawTree,
        headers: &[StructureHeaderPreparation<'_>],
    ) -> Result<Self, String> {
        if source.as_ref() != mapped.original() || source.len() > MAX_SYNTAX_SOURCE_BYTES {
            return Err(
                "prepared completion source identity or native context budget differs".into(),
            );
        }
        let lines = LineIndex::new(&source);
        let mut context = raw_context(tree, &source, &lines, true)?;
        let mut scopes = tree
            .iter()
            .filter(|n| n.kind == RawNodeKind::Section)
            .map(|node| (node.id, Scope::Pending))
            .collect::<HashMap<_, _>>();
        let mut entry_scopes = Vec::new();
        let mut owners = HashMap::new();
        for header in headers {
            let candidate = header.candidate;
            let Some(raw) = tree.get(candidate.raw_node_id) else {
                continue;
            };
            if header.ambiguous
                || exact_native(&raw.line.span).is_none()
                || skript_parser::validate_structure_body_layout(
                    header.body_mode,
                    candidate.entry_validator.as_ref(),
                    &candidate.entry_body_modes,
                )
                .is_err()
            {
                continue;
            }
            owners.insert(
                raw.id,
                OwnerAttempt {
                    root: raw.id,
                    registration: candidate.matched.registration_id.clone(),
                    pattern: candidate.matched.pattern_index,
                },
            );
            match header.body_mode {
                StructureBodyMode::Trigger => {
                    scopes.insert(raw.id, Scope::Statements);
                }
                StructureBodyMode::Entries => {
                    if let Some(validator) = &candidate.entry_validator {
                        prepare_entries(
                            tree,
                            raw.id,
                            validator,
                            &candidate.entry_body_modes,
                            &candidate.matched.registration_id,
                            &mut scopes,
                            &mut entry_scopes,
                        );
                    } else {
                        scopes.insert(raw.id, Scope::Raw);
                    }
                }
                StructureBodyMode::Raw | StructureBodyMode::None => {
                    scopes.insert(raw.id, Scope::Raw);
                }
            }
        }
        for line in context.values_mut() {
            if !line.root {
                line.scope = line
                    .ancestors
                    .iter()
                    .find_map(|id| scopes.get(id).copied())
                    .unwrap_or(Scope::Pending);
            }
        }
        Ok(Self {
            document: Document {
                source,
                lines,
                context,
                entry_scopes,
                variables: Vec::new(),
                functions: Vec::new(),
                index,
                blocked: Vec::new(),
                function_namespace_complete: false,
            },
            owners,
            revoked: HashSet::new(),
        })
    }

    pub fn owner_at(&self, position: Position) -> Option<&OwnerAttempt> {
        let line = self.document.context.get(&position.line)?;
        line.node
            .iter()
            .chain(&line.ancestors)
            .find_map(|id| self.owners.get(id))
    }

    pub fn revoke(&mut self, attempt: &OwnerAttempt) -> bool {
        if self.owners.get(&attempt.root) != Some(attempt) || !self.revoked.insert(attempt.root) {
            return false;
        }
        for line in self.document.context.values_mut() {
            if line.node == Some(attempt.root) || line.ancestors.contains(&attempt.root) {
                line.scope = Scope::Pending;
            }
        }
        true
    }

    pub fn at(&self, position: Position, snippets: bool) -> Result<PreparedQuery, String> {
        let owner = self.owner_at(position);
        let revoked = owner.is_some_and(|owner| self.revoked.contains(&owner.root));
        if revoked {
            return Ok(PreparedQuery::NeedsAnalysis);
        }
        let mut list = self.document.syntax_at(position, snippets)?;
        let suppressed = self.document.suppresses_completion(position);
        let offset = self
            .document
            .lines
            .offset(&self.document.source, position)
            .ok_or("invalid completion UTF-16 position")?;
        let closed_scope = self
            .document
            .context
            .get(&position.line)
            .is_some_and(|line| {
                matches!(
                    self.document.structure_query_line(line, offset).scope,
                    Scope::Raw | Scope::Entries(_)
                )
            });
        if list.items.is_empty() && !suppressed && !closed_scope {
            return Ok(PreparedQuery::NeedsAnalysis);
        }
        list.is_incomplete = true;
        for item in &mut list.items {
            let data = item.data.get_or_insert_with(|| serde_json::json!({}));
            if let Some(data) = data.as_object_mut() {
                data.insert("completionPhase".into(), serde_json::json!("prepared"));
                if let Some(owner) = owner {
                    data.insert("ownerAttempt".into(), serde_json::json!({"rawNodeId":owner.root.get(),"registrationId":owner.registration,"patternIndex":owner.pattern}));
                }
            }
            item.detail = Some(format!(
                "provisional / {}",
                item.detail.as_deref().unwrap_or("Syntax")
            ));
            let note = "Provisional syntax proposal from the current header preparation. Body validation and document references are still pending.\n\n";
            match &mut item.documentation {
                Some(Documentation::String(text)) => text.insert_str(0, note),
                Some(Documentation::MarkupContent(content)) => content.value.insert_str(0, note),
                None => item.documentation = Some(Documentation::String(note.trim_end().into())),
            }
        }
        Ok(PreparedQuery::Ready(list))
    }
}

#[cfg(test)]
impl PreparedDocument {
    pub fn test_fixture(source: Arc<str>) -> Self {
        let mut document = Document::test_fixture(Arc::clone(&source));
        let index = Arc::get_mut(&mut document.index).unwrap();
        index.templates[0] = Template {
            documentation: None,
            kind: SyntaxKind::Effect,
            label: "hold pattern".into(),
            detail: "fixture".into(),
            identity: "fixture:0".into(),
            leading: "hold pattern".into(),
            folded: "hold pattern".into(),
            parts: vec![Part::Text("hold pattern".into())],
        };
        let mapped = skript_parser::MappedSource::identity(source);
        let tree = skript_parser::parse_raw_tree(&mapped, index.raw_options);
        let root = tree.roots[0];
        let attempt = OwnerAttempt {
            root,
            registration: "fixture".into(),
            pattern: 0,
        };
        let mut result = Self {
            document,
            owners: HashMap::from([(root, attempt)]),
            revoked: HashSet::new(),
        };
        if result.document.source.starts_with("hold-raw") {
            for line in result
                .document
                .context
                .values_mut()
                .filter(|line| !line.root)
            {
                line.scope = Scope::Raw;
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skript_parser::{
        CandidateMatch, MatchSpan, MatchSyntaxKind, PatternMatch, StructureBody, StructureCandidate,
    };
    use syntaxes::{ClassName, EntryData, NodeType};

    fn candidate(
        tree: &skript_parser::RawTree,
        root: usize,
        validator: Option<EntryValidator>,
    ) -> StructureCandidate {
        let node = tree.get(tree.roots[root]).unwrap();
        let span = node.code_span.clone().unwrap();
        StructureCandidate {
            raw_node_id: node.id,
            matched: CandidateMatch {
                extension_state: None,
                kind: MatchSyntaxKind::Structure,
                definition_id: "fixture".into(),
                registration_id: format!("owner{root}"),
                priority: 0,
                registration_order: 0,
                literal_anchor: true,
                pattern_index: 0,
                pattern: "root".into(),
                matched: PatternMatch {
                    span: MatchSpan {
                        local_range: span.virtual_range,
                        mapped: span,
                    },
                    captures: Vec::new(),
                    tags: Vec::new(),
                    mark: 0,
                    marks: Vec::new(),
                    recovered_failures: Vec::new(),
                },
            },
            element_class: None,
            declared_node_type: NodeType::Section,
            actual_node_type: NodeType::Section,
            parsed_captures: Vec::new(),
            body: StructureBody::None,
            entry_validation_failed: false,
            entry_validator: validator,
            entry_body_modes: BTreeMap::new(),
            handler: None,
            metadata: BTreeMap::new(),
        }
    }
    fn entry(key: &str, kind: EntryKind) -> EntryData {
        EntryData {
            key: key.into(),
            default_value: None,
            optional: true,
            multiple: false,
            entry_data_class: ClassName("fixture.Entry".into()),
            kind,
            separator: Some(": ".into()),
            value_type: None,
            string_mode: None,
            return_types: Vec::new(),
            flags: Some(1),
            nested_validator: None,
        }
    }
    fn prepared_view(
        source: &str,
        mode: StructureBodyMode,
        validator: Option<EntryValidator>,
        ambiguous: bool,
    ) -> PreparedDocument {
        let source: Arc<str> = Arc::from(source);
        let mapped = skript_parser::MappedSource::identity(Arc::clone(&source));
        let index = PreparedDocument::test_fixture(Arc::clone(&source))
            .document
            .index;
        let tree = skript_parser::parse_raw_tree(&mapped, index.raw_options);
        let owner = candidate(&tree, 0, validator);
        PreparedDocument::new(
            source,
            index,
            &mapped,
            &tree,
            &[StructureHeaderPreparation {
                candidate: &owner,
                body_mode: mode,
                ambiguous,
            }],
        )
        .unwrap()
    }
    fn ready(view: &PreparedDocument, row: u32, col: u32) -> CompletionList {
        let PreparedQuery::Ready(list) = view.at(Position::new(row, col), false).unwrap() else {
            panic!("expected prepared syntax")
        };
        list
    }
    #[test]
    fn current_header_proposals_are_explicit_and_references_stay_pending() {
        let view = prepared_view(
            "root:\n    hold\n    {_va",
            StructureBodyMode::Trigger,
            None,
            false,
        );
        let list = ready(&view, 1, 8);
        assert_eq!(list.items.len(), 1);
        assert!(list.is_incomplete);
        assert!(
            list.items[0]
                .detail
                .as_ref()
                .unwrap()
                .starts_with("provisional / ")
        );
        assert_eq!(
            list.items[0].data.as_ref().unwrap()["completionPhase"],
            "prepared"
        );
        assert!(matches!(
            view.at(Position::new(2, 8), false).unwrap(),
            PreparedQuery::NeedsAnalysis
        ));
        assert!(view.document.functions.is_empty() && view.document.variables.is_empty());
    }
    #[test]
    fn unknown_nested_sections_and_ambiguous_headers_wait_for_analysis() {
        let source = "root:\n    mystery:\n        hold";
        let view = prepared_view(source, StructureBodyMode::Trigger, None, false);
        assert!(matches!(
            view.at(Position::new(2, 12), false).unwrap(),
            PreparedQuery::NeedsAnalysis
        ));
        let view = prepared_view("root:\n    hold", StructureBodyMode::Trigger, None, true);
        assert!(matches!(
            view.at(Position::new(1, 8), false).unwrap(),
            PreparedQuery::NeedsAnalysis
        ));
    }
    #[test]
    fn revocation_is_exact_and_does_not_consume_reference_requests() {
        let mut view = prepared_view("root:\n    hold", StructureBodyMode::Trigger, None, false);
        let owner = view.owner_at(Position::new(1, 8)).unwrap().clone();
        let mut other = owner.clone();
        other.pattern += 1;
        assert!(!view.revoke(&other));
        assert_eq!(ready(&view, 1, 8).items.len(), 1);
        assert!(view.revoke(&owner));
        assert!(!view.revoke(&owner));
        assert!(matches!(
            view.at(Position::new(1, 8), false).unwrap(),
            PreparedQuery::NeedsAnalysis
        ));
    }
    #[test]
    fn native_entry_selection_retains_duplicate_indices_and_value_boundaries() {
        let mut expression = entry("\"q", EntryKind::Expression);
        expression.multiple = true;
        let validator = EntryValidator {
            entry_data: vec![
                entry("name", EntryKind::KeyValue),
                entry("name", EntryKind::KeyValue),
                expression,
            ],
        };
        let view = prepared_view(
            "root:\n    name: first\n    name: second\n    \"q: hold\n    ",
            StructureBodyMode::Entries,
            Some(validator),
            false,
        );
        assert_eq!(
            view.document.entry_scopes[0]
                .present
                .iter()
                .map(|p| p.0)
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert!(!view.document.suppresses_completion(Position::new(3, 12)));
        let list = ready(&view, 4, 4);
        assert!(!list.items.iter().any(|item| item.label == "name"));
    }
    #[test]
    fn executable_and_raw_entry_sections_use_owner_modes() {
        let source: Arc<str> =
            Arc::from("root:\n    execute:\n        hold\n    value:\n        hold");
        let mapped = skript_parser::MappedSource::identity(Arc::clone(&source));
        let index = PreparedDocument::test_fixture(Arc::clone(&source))
            .document
            .index;
        let tree = skript_parser::parse_raw_tree(&mapped, index.raw_options);
        let mut owner = candidate(
            &tree,
            0,
            Some(EntryValidator {
                entry_data: vec![
                    entry("execute", EntryKind::Section),
                    entry("value", EntryKind::Section),
                ],
            }),
        );
        owner.entry_body_modes.insert(
            vec!["execute".into()],
            skript_parser::SectionBodyMode::Trigger,
        );
        let view = PreparedDocument::new(
            source,
            index,
            &mapped,
            &tree,
            &[StructureHeaderPreparation {
                candidate: &owner,
                body_mode: StructureBodyMode::Entries,
                ambiguous: false,
            }],
        )
        .unwrap();
        assert_eq!(ready(&view, 2, 12).items.len(), 1);
        assert!(ready(&view, 4, 12).items.is_empty());
    }
}

fn prepare_entries(
    tree: &skript_parser::RawTree,
    root: RawNodeId,
    validator: &EntryValidator,
    modes: &BTreeMap<Vec<String>, skript_parser::SectionBodyMode>,
    registration: &str,
    scopes: &mut HashMap<RawNodeId, Scope>,
    output: &mut Vec<EntryScope>,
) {
    let mut pending = vec![(root, validator, Vec::<String>::new())];
    while let Some((parent, validator, path)) = pending.pop() {
        let Some(raw) = tree.get(parent) else {
            continue;
        };
        let mut seen = std::collections::BTreeSet::new();
        let mut present = Vec::new();
        for id in &raw.children {
            let Some(child) = tree.get(*id) else { continue };
            if matches!(child.kind, RawNodeKind::Blank | RawNodeKind::Comment) {
                continue;
            }
            let Some(index) = skript_parser::structure_entry_index(validator, child, &seen) else {
                continue;
            };
            seen.insert(index);
            present.push((index, *id));
            let data = &validator.entry_data[index];
            let mut entry_path = path.clone();
            entry_path.push(data.key.clone());
            match data.kind {
                EntryKind::Container => {
                    if let Some(nested) = &data.nested_validator {
                        pending.push((*id, nested, entry_path));
                    } else {
                        scopes.insert(*id, Scope::Raw);
                    }
                }
                EntryKind::Trigger => {
                    scopes.insert(*id, Scope::Statements);
                }
                EntryKind::Section => {
                    scopes.insert(
                        *id,
                        match modes.get(&entry_path) {
                            Some(skript_parser::SectionBodyMode::Trigger) => Scope::Statements,
                            Some(skript_parser::SectionBodyMode::Conditions) => Scope::Conditions,
                            _ => Scope::Raw,
                        },
                    );
                }
                EntryKind::Unknown if child.kind == RawNodeKind::Section => {
                    scopes.insert(*id, Scope::Raw);
                }
                _ => {}
            }
        }
        scopes.insert(parent, Scope::Entries(output.len()));
        output.push(EntryScope {
            declarations: validator.entry_data.clone(),
            present,
            registration: registration.into(),
        });
    }
}

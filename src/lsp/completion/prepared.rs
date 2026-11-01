//! Current-header syntax proposals, isolated from final references and signatures.
use super::*;
use skript_parser::{StructureBodyMode, StructureHeaderPreparation};
use syntaxes::{EntryKind, EntryValidator};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::lsp) struct OwnerAttempt {
    pub root: RawNodeId,
    pub registration: String,
    pub pattern: usize,
    pub attempt_id: u64,
}

/// This view has no semantic reference history and cannot answer signature queries.
pub(in crate::lsp) struct PreparedDocument {
    pub(super) document: Document,
    owners: HashMap<RawNodeId, OwnerAttempt>,
    revoked: HashSet<RawNodeId>,
    body_scopes: HashMap<RawNodeId, Scope>,
    raw_children: HashMap<RawNodeId, Vec<RawNodeId>>,
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
        let mut parameters = HashMap::new();
        for header in headers {
            let candidate = header.candidate;
            let Some(raw) = tree.get(candidate.raw_node_id) else {
                continue;
            };
            let Ok(entry_plans)=skript_parser::validated_structure_entry_plans(candidate,tree,header.body_mode) else {continue;};
            if header.ambiguous
                || !context.values().any(|line| line.node == Some(raw.id))
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
                    attempt_id: header.attempt_id,
                },
            );
            if let Some(bindings) = super::super::public_data::parameter_bindings(
                candidate.metadata.iter().filter_map(|(key, value)| {
                    let (owner, key) = key.split_once('/')?;
                    (!owner.is_empty()).then_some((key, value.as_str()))
                }),
            ) {
                if let Some(declarations) = parameters::project(
                    bindings,
                    &source,
                    exact_native(&raw.line.span).unwrap(),
                    raw.line
                        .span
                        .virtual_range
                        .start
                        .try_into()
                        .map_err(|_| "parameter header offset overflow")?,
                ) {
                    parameters.insert(raw.id, declarations);
                }
            }
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
                            &entry_plans,
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
                provider_completions: super::providers::Snapshot::default(),
                dynamic_patterns: super::dynamic_patterns::View::default(),
                editor_queries: Vec::new(),
                source,
                lines,
                context,
                entry_scopes,
                variables: Vec::new(),
                parameters,
                functions: Vec::new(),
                index,
                blocked: Vec::new(),
                function_namespace_complete: false,
                type_context_exact: mapped.original() == mapped.virtual_source(),
                accepted_slots: HashMap::new(),
                semantic_types: None,
            },
            owners,
            revoked: HashSet::new(),
            body_scopes: scopes,
            raw_children: tree
                .iter()
                .map(|node| (node.id, node.children.clone()))
                .collect(),
        })
    }

    fn refresh_scopes(&mut self) {
        for line in self.document.context.values_mut() {
            if !line.root {
                line.scope = line
                    .ancestors
                    .iter()
                    .find_map(|id| self.body_scopes.get(id).copied())
                    .unwrap_or(Scope::Pending);
            }
        }
    }

    fn region(&self, root: RawNodeId) -> HashSet<RawNodeId> {
        // Projection can omit ambiguous header rows; ownership still covers every raw descendant.
        let mut region = HashSet::new();
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            if let Some(children) = self.raw_children.get(&id) {
                if region.insert(id) {
                    pending.extend(children.iter().copied());
                }
            }
        }
        region
    }

    pub fn affects(&self, position: Position, owner: &OwnerAttempt) -> bool {
        self.document
            .context
            .get(&position.line)
            .is_some_and(|line| {
                line.node == Some(owner.root) || line.ancestors.contains(&owner.root)
            })
    }

    /// Replace only the owner which was actually retried; other root proposals survive.
    pub fn prepare_structure(&mut self, root: RawNodeId, mut prepared: Self) -> bool {
        if self.document.source != prepared.document.source {
            return false;
        }
        let Some(owner) = prepared.owners.remove(&root) else {
            return false;
        };
        if self.owners.get(&root) == Some(&owner) && !self.revoked.contains(&root) {
            return false;
        }
        let region = self.region(root);
        if !region.contains(&root) {
            return false;
        }
        self.owners.retain(|id, _| !region.contains(id));
        self.revoked.retain(|id| !region.contains(id));
        let offset = self.document.entry_scopes.len();
        self.document
            .entry_scopes
            .append(&mut prepared.document.entry_scopes);
        for id in &region {
            if let Some(scope) = prepared.body_scopes.remove(id) {
                let scope = match scope {
                    Scope::Entries(index) => Scope::Entries(index + offset),
                    scope => scope,
                };
                self.body_scopes.insert(*id, scope);
            }
        }
        self.document
            .parameters
            .retain(|id, _| !region.contains(id));
        self.document.parameters.extend(
            prepared
                .document
                .parameters
                .into_iter()
                .filter(|(id, _)| region.contains(id)),
        );
        self.owners.insert(root, owner);
        // Drop scopes belonging only to superseded attempts, preserving declaration order.
        let used = self
            .body_scopes
            .values()
            .filter_map(|scope| {
                if let Scope::Entries(index) = scope {
                    Some(*index)
                } else {
                    None
                }
            })
            .collect::<HashSet<_>>();
        let mut remap = HashMap::new();
        self.document.entry_scopes = std::mem::take(&mut self.document.entry_scopes)
            .into_iter()
            .enumerate()
            .filter_map(|(old, entry)| {
                if !used.contains(&old) {
                    return None;
                }
                remap.insert(old, remap.len());
                Some(entry)
            })
            .collect();
        for scope in self.body_scopes.values_mut() {
            if let Scope::Entries(index) = scope {
                *index = remap[&*index];
            }
        }
        self.refresh_scopes();
        true
    }

    /// Native enter hooks have certified this body's kind; an unknown parent cannot lend it scope.
    pub fn prepare_section(&mut self, section: PreparedSection) -> bool {
        let Some(parent) = self
            .owners
            .values()
            .find(|owner| owner.attempt_id == section.parent_attempt_id)
            .cloned()
        else {
            return false;
        };
        if self.revoked.contains(&parent.root) {
            return false;
        }
        let Some(line) = self
            .document
            .context
            .values()
            .find(|line| line.node == Some(section.owner.root))
        else {
            return false;
        };
        if line.range != section.header_range
            || !line.ancestors.contains(&parent.root)
            || line.ancestors.iter().any(|id| self.revoked.contains(id))
        {
            return false;
        }
        if self.owners.get(&section.owner.root) == Some(&section.owner) {
            return false;
        }
        let region = self.region(section.owner.root);
        self.owners.retain(|id, _| !region.contains(id));
        self.revoked.retain(|id| !region.contains(id));
        for id in region {
            if self.body_scopes.contains_key(&id) {
                self.body_scopes.insert(id, Scope::Pending);
            }
        }
        self.body_scopes.insert(section.owner.root, section.scope);
        self.owners.insert(section.owner.root, section.owner);
        self.refresh_scopes();
        true
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
        let region = self.region(attempt.root);
        self.document
            .parameters
            .retain(|id, _| !region.contains(id));
        for id in region {
            if self.body_scopes.contains_key(&id) {
                self.body_scopes.insert(id, Scope::Pending);
            }
        }
        self.refresh_scopes();
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
                    data.insert("ownerAttempt".into(), serde_json::json!({"rawNodeId":owner.root.get(),"registrationId":owner.registration,"patternIndex":owner.pattern,"attemptId":owner.attempt_id}));
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
            finite_capture: false,
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
            attempt_id: 1,
        };
        let mut result = Self {
            document,
            owners: HashMap::from([(root, attempt)]),
            revoked: HashSet::new(),
            raw_children: tree
                .iter()
                .map(|node| (node.id, node.children.clone()))
                .collect(),
            body_scopes: tree
                .iter()
                .filter(|node| node.kind == RawNodeKind::Section)
                .map(|node| {
                    (
                        node.id,
                        if node.id == root {
                            Scope::Statements
                        } else {
                            Scope::Pending
                        },
                    )
                })
                .collect(),
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

    pub fn test_nested_fixture(source: Arc<str>) -> Self {
        let mut view = Self::test_fixture(Arc::clone(&source));
        let mapped = skript_parser::MappedSource::identity(source);
        let tree = skript_parser::parse_raw_tree(&mapped, view.document.index.raw_options);
        view.document.context =
            raw_context(&tree, mapped.original(), &view.document.lines, true).unwrap();
        let root = tree.roots[0];
        let nested = tree
            .get(root)
            .unwrap()
            .children
            .iter()
            .copied()
            .find(|id| tree.get(*id).unwrap().kind == RawNodeKind::Section)
            .unwrap();
        view.owners.insert(
            nested,
            OwnerAttempt {
                root: nested,
                registration: "fixture-nested".into(),
                pattern: 0,
                attempt_id: 2,
            },
        );
        view.body_scopes.insert(nested, Scope::Statements);
        view.refresh_scopes();
        view
    }
}

/// A small owned delta; no borrowed native node or previous-revision range escapes the observer.
pub(in crate::lsp) struct PreparedSection {
    pub owner: OwnerAttempt,
    parent_attempt_id: u64,
    header_range: TextRange,
    scope: Scope,
}
impl PreparedSection {
    pub fn new(
        source: &str,
        mapped: &skript_parser::MappedSource,
        tree: &skript_parser::RawTree,
        candidate: &skript_parser::SectionCandidate,
        attempt_id: u64,
        parent_attempt_id: Option<u64>,
    ) -> Result<Option<Self>, String> {
        if source != mapped.original() {
            return Err("prepared section source identity differs".into());
        }
        let Some(parent_attempt_id) = parent_attempt_id else {
            return Ok(None);
        };
        let Some(raw) = tree.get(candidate.raw_node_id) else {
            return Ok(None);
        };
        let Some(range) = exact_native(&raw.line.span) else {
            return Ok(None);
        };
        let Some(text) = source.get(range.start..range.end) else {
            return Ok(None);
        };
        let text = text.trim_end_matches('\n').trim_end_matches('\r');
        if raw.kind != RawNodeKind::Section || text != raw.line.raw_text {
            return Ok(None);
        }
        let scope = match candidate.body_mode {
            skript_parser::SectionBodyMode::Trigger => Scope::Statements,
            skript_parser::SectionBodyMode::Conditions => Scope::Conditions,
        };
        Ok(Some(Self {
            owner: OwnerAttempt {
                root: raw.id,
                registration: candidate.matched.registration_id.clone(),
                pattern: candidate.matched.pattern_index,
                attempt_id,
            },
            parent_attempt_id,
            header_range: TextRange::new(range.start, range.start + text.len()),
            scope,
        }))
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
                attempt_id: 1,
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
                attempt_id: 1,
                candidate: &owner,
                body_mode: StructureBodyMode::Entries,
                ambiguous: false,
            }],
        )
        .unwrap();
        assert_eq!(ready(&view, 2, 12).items.len(), 1);
        assert!(ready(&view, 4, 12).items.is_empty());
    }
    #[test]
    fn current_unknown_entry_plan_uses_the_same_provisional_selector_and_revocation() {
        for (mode,expected) in [("trigger",1),("raw",0)] {
            let source:Arc<str>=Arc::from("root:\n    change number:\n        hold");
            let mapped=skript_parser::MappedSource::identity(Arc::clone(&source));
            let index=PreparedDocument::test_fixture(Arc::clone(&source)).document.index;
            let tree=skript_parser::parse_raw_tree(&mapped,index.raw_options);
            let mut owner=candidate(&tree,0,Some(EntryValidator {entry_data:vec![entry("change",EntryKind::Unknown)]}));
            let raw=tree.get(owner.raw_node_id).unwrap();let node=raw.children[0];
            owner.metadata.insert(skript_parser::STRUCTURE_ENTRY_LAYOUT_METADATA.into(),skript_parser::structure_entry_layout(&owner,"17").unwrap());
            owner.metadata.insert(format!("example.owner/{}",skript_parser::STRUCTURE_ENTRY_PLANS_METADATA),serde_json::json!({"version":1,"token":"17","structureNode":raw.id.get(),"entries":[{
                "node":node.get(),"declarationIndex":0,"mode":mode,"adoptUnknown":true,
                "context":{"eventClasses":if mode=="raw"{serde_json::Value::Null}else{serde_json::json!([])},"values":[]}}]}).to_string());
            let mut view=PreparedDocument::new(source,index,&mapped,&tree,&[StructureHeaderPreparation {attempt_id:1,candidate:&owner,body_mode:StructureBodyMode::Entries,ambiguous:false}]).unwrap();
            assert_eq!(ready(&view,2,12).items.len(),expected,"data is never a statement completion scope");
            if mode=="trigger" {
                assert!(view.revoke(&OwnerAttempt {root:owner.raw_node_id,registration:owner.matched.registration_id.clone(),pattern:owner.matched.pattern_index,attempt_id:1}));
                assert!(matches!(view.at(Position::new(2,12),false).unwrap(),PreparedQuery::NeedsAnalysis));
            }
        }
    }
    fn section_candidate(
        tree: &skript_parser::RawTree,
        line: usize,
        mode: skript_parser::SectionBodyMode,
    ) -> skript_parser::SectionCandidate {
        let raw = tree.iter().find(|node| node.line.number == line).unwrap();
        let mut matched = candidate(tree, 0, None).matched;
        matched.kind = MatchSyntaxKind::Section;
        matched.registration_id = format!("section-{line}");
        let span = raw.code_span.clone().unwrap();
        matched.matched.span = MatchSpan {
            local_range: span.virtual_range,
            mapped: span,
        };
        skript_parser::SectionCandidate {
            raw_node_id: raw.id,
            matched,
            element_class: None,
            addon_name: None,
            addon_version: None,
            owner_component_id: None,
            parsed_captures: Vec::new(),
            loop_section: false,
            effect_section: false,
            section_expression: false,
            body_mode: mode,
            body: Vec::new(),
            handler: None,
            metadata: BTreeMap::new(),
            body_context: Default::default(),
        }
    }

    fn section(
        source: &str,
        line: usize,
        mode: skript_parser::SectionBodyMode,
        attempt: u64,
        parent: Option<u64>,
    ) -> Option<PreparedSection> {
        let mapped = skript_parser::MappedSource::identity(Arc::from(source));
        let index = PreparedDocument::test_fixture(Arc::from(source))
            .document
            .index;
        let tree = skript_parser::parse_raw_tree(&mapped, index.raw_options);
        PreparedSection::new(
            source,
            &mapped,
            &tree,
            &section_candidate(&tree, line, mode),
            attempt,
            parent,
        )
        .unwrap()
    }

    #[test]
    fn certified_nested_body_modes_preserve_utf8_ranges_and_do_not_guess_unconfirmed_scope() {
        let source = "root: # 日本語\r\n    nested:\r\n        hold";
        for (mode, expected) in [
            (skript_parser::SectionBodyMode::Trigger, Scope::Statements),
            (
                skript_parser::SectionBodyMode::Conditions,
                Scope::Conditions,
            ),
        ] {
            let mut view = prepared_view(source, StructureBodyMode::Trigger, None, false);
            assert!(matches!(
                view.at(Position::new(2, 12), false).unwrap(),
                PreparedQuery::NeedsAnalysis
            ));
            assert!(
                section(source, 1, mode, 2, None).is_none(),
                "uncertified parent must not lend scope"
            );
            assert!(!view.prepare_section(section(source, 1, mode, 2, Some(999)).unwrap()));
            assert!(view.prepare_section(section(source, 1, mode, 2, Some(1)).unwrap()));
            assert_eq!(
                std::mem::discriminant(&view.document.context[&2].scope),
                std::mem::discriminant(&expected)
            );
            if mode == skript_parser::SectionBodyMode::Trigger {
                assert_eq!(ready(&view, 2, 12).items.len(), 1);
            } else {
                assert!(
                    view.document
                        .syntax_at(Position::new(2, 12), false)
                        .unwrap()
                        .items
                        .is_empty(),
                    "condition bodies must not inherit statement effect proposals"
                );
                assert!(matches!(
                    view.at(Position::new(2, 12), false).unwrap(),
                    PreparedQuery::NeedsAnalysis
                ));
            }
        }
        let mapped = skript_parser::MappedSource::identity(Arc::from(source));
        let index = PreparedDocument::test_fixture(Arc::from(source))
            .document
            .index;
        let tree = skript_parser::parse_raw_tree(&mapped, index.raw_options);
        let candidate = section_candidate(&tree, 1, skript_parser::SectionBodyMode::Trigger);
        assert!(PreparedSection::new("different", &mapped, &tree, &candidate, 2, Some(1)).is_err());
        let mut view = prepared_view(source, StructureBodyMode::Trigger, None, false);
        let mut forged = section(
            source,
            1,
            skript_parser::SectionBodyMode::Trigger,
            2,
            Some(1),
        )
        .unwrap();
        forged.header_range.start += 1;
        assert!(
            !view.prepare_section(forged),
            "a header range from another projection cannot certify this source"
        );
        assert!(matches!(
            view.at(Position::new(2, 12), false).unwrap(),
            PreparedQuery::NeedsAnalysis
        ));
    }

    #[test]
    fn section_retry_withdraws_descendants_and_requires_the_new_parent_attempt() {
        let source = "root:\n    outer:\n        inner:\n            hold\n    hold";
        let mode = skript_parser::SectionBodyMode::Trigger;
        let mut view = prepared_view(source, StructureBodyMode::Trigger, None, false);
        assert!(!view.prepare_section(section(source, 2, mode, 3, Some(2)).unwrap()));
        assert!(view.prepare_section(section(source, 1, mode, 2, Some(1)).unwrap()));
        assert!(view.prepare_section(section(source, 2, mode, 3, Some(2)).unwrap()));
        assert_eq!(ready(&view, 3, 16).items.len(), 1);
        let old_parent = view.owner_at(Position::new(1, 4)).unwrap().clone();
        assert!(view.affects(Position::new(3, 16), &old_parent));
        assert!(view.revoke(&old_parent));
        assert!(matches!(
            view.at(Position::new(3, 16), false).unwrap(),
            PreparedQuery::NeedsAnalysis
        ));
        assert_eq!(
            ready(&view, 4, 8).items.len(),
            1,
            "a separate body line under the root survives nested revocation"
        );
        assert!(!view.prepare_section(section(source, 2, mode, 3, Some(2)).unwrap()));
        assert!(view.prepare_section(section(source, 1, mode, 4, Some(1)).unwrap()));
        assert!(
            !view.revoke(&old_parent),
            "late invalidation belongs to the superseded parent attempt"
        );
        assert!(matches!(
            view.at(Position::new(3, 16), false).unwrap(),
            PreparedQuery::NeedsAnalysis
        ));
        assert!(!view.prepare_section(section(source, 2, mode, 5, Some(2)).unwrap()));
        assert!(view.prepare_section(section(source, 2, mode, 5, Some(4)).unwrap()));
        assert_eq!(ready(&view, 3, 16).items.len(), 1);
        let root = view.owner_at(Position::new(0, 0)).unwrap().clone();
        assert!(view.revoke(&root));
        assert!(matches!(
            view.at(Position::new(3, 16), false).unwrap(),
            PreparedQuery::NeedsAnalysis
        ));
        assert!(matches!(
            view.at(Position::new(4, 8), false).unwrap(),
            PreparedQuery::NeedsAnalysis
        ));
    }

    #[test]
    fn root_retry_merges_only_its_region_and_preserves_other_entry_declarations() {
        let source: Arc<str> = Arc::from("first:\n    \nsecond:\n    ");
        let mapped = skript_parser::MappedSource::identity(Arc::clone(&source));
        let index = PreparedDocument::test_fixture(Arc::clone(&source))
            .document
            .index;
        let tree = skript_parser::parse_raw_tree(&mapped, index.raw_options);
        let first = candidate(
            &tree,
            0,
            Some(EntryValidator {
                entry_data: vec![entry("alpha", EntryKind::KeyValue)],
            }),
        );
        let second = candidate(
            &tree,
            1,
            Some(EntryValidator {
                entry_data: vec![entry("beta", EntryKind::KeyValue)],
            }),
        );
        let mut view = PreparedDocument::new(
            Arc::clone(&source),
            Arc::clone(&index),
            &mapped,
            &tree,
            &[
                StructureHeaderPreparation {
                    attempt_id: 1,
                    candidate: &first,
                    body_mode: StructureBodyMode::Entries,
                    ambiguous: false,
                },
                StructureHeaderPreparation {
                    attempt_id: 2,
                    candidate: &second,
                    body_mode: StructureBodyMode::Entries,
                    ambiguous: false,
                },
            ],
        )
        .unwrap();
        let first_owner = view.owner_at(Position::new(0, 0)).unwrap().clone();
        let second_owner = view.owner_at(Position::new(2, 0)).unwrap().clone();
        assert!(
            ready(&view, 1, 4)
                .items
                .iter()
                .any(|item| item.label == "alpha")
        );
        assert!(
            ready(&view, 3, 4)
                .items
                .iter()
                .any(|item| item.label == "beta")
        );
        for attempt_id in 3..8 {
            let retry = PreparedDocument::new(
                Arc::clone(&source),
                Arc::clone(&index),
                &mapped,
                &tree,
                &[StructureHeaderPreparation {
                    attempt_id,
                    candidate: &first,
                    body_mode: StructureBodyMode::Entries,
                    ambiguous: false,
                }],
            )
            .unwrap();
            assert!(view.prepare_structure(tree.roots[0], retry));
            assert_eq!(view.owner_at(Position::new(2, 0)), Some(&second_owner));
            assert!(
                ready(&view, 1, 4)
                    .items
                    .iter()
                    .any(|item| item.label == "alpha")
            );
            assert!(
                ready(&view, 3, 4)
                    .items
                    .iter()
                    .any(|item| item.label == "beta")
            );
            assert_eq!(
                view.document.entry_scopes.len(),
                2,
                "superseded entry scopes do not accumulate"
            );
            assert!(!view.revoke(&first_owner));
        }
    }

    #[test]
    fn duplicate_header_projection_cannot_retain_child_body_scope_after_root_retry_or_revoke() {
        let source: Arc<str> = Arc::from("root:\n    body:\n        hold");
        let mapped = skript_parser::MappedSource::identity(Arc::clone(&source));
        let index = PreparedDocument::test_fixture(Arc::clone(&source))
            .document
            .index;
        let mut tree = skript_parser::parse_raw_tree(&mapped, index.raw_options);
        let root = tree.roots[0];
        let child = tree.get(root).unwrap().children[0];
        // Two native headers project to one editor row. raw_context must omit that row,
        // but the actual native descendant remains part of replacement/revocation.
        let mut duplicate = tree.get(child).unwrap().clone();
        duplicate.id = RawNodeId::new(tree.nodes.len() as u64);
        duplicate.children.clear();
        tree.nodes[root.get() as usize].children.push(duplicate.id);
        tree.nodes.push(duplicate);
        for kind in 0..3 {
            let mut data = entry(
                "body",
                if kind == 2 {
                    EntryKind::Container
                } else {
                    EntryKind::Section
                },
            );
            data.multiple = true;
            if kind == 2 {
                data.nested_validator = Some(EntryValidator {
                    entry_data: vec![entry("inner", EntryKind::KeyValue)],
                });
            }
            let mut initial = candidate(
                &tree,
                0,
                Some(EntryValidator {
                    entry_data: vec![data],
                }),
            );
            if kind < 2 {
                initial.entry_body_modes.insert(
                    vec!["body".into()],
                    if kind == 0 {
                        skript_parser::SectionBodyMode::Trigger
                    } else {
                        skript_parser::SectionBodyMode::Conditions
                    },
                );
            }
            for revoke in [false, true] {
                let mut view = PreparedDocument::new(
                    Arc::clone(&source),
                    Arc::clone(&index),
                    &mapped,
                    &tree,
                    &[StructureHeaderPreparation {
                        attempt_id: 1,
                        candidate: &initial,
                        body_mode: StructureBodyMode::Entries,
                        ambiguous: false,
                    }],
                )
                .unwrap();
                assert!(!view.document.context.contains_key(&1));
                assert!(matches!(
                    view.body_scopes[&child],
                    Scope::Statements | Scope::Conditions | Scope::Entries(_)
                ));
                if revoke {
                    let owner = view.owner_at(Position::new(0, 0)).unwrap().clone();
                    assert!(view.revoke(&owner));
                } else {
                    let retried = candidate(&tree, 0, None);
                    let delta = PreparedDocument::new(
                        Arc::clone(&source),
                        Arc::clone(&index),
                        &mapped,
                        &tree,
                        &[StructureHeaderPreparation {
                            attempt_id: 2,
                            candidate: &retried,
                            body_mode: StructureBodyMode::Trigger,
                            ambiguous: false,
                        }],
                    )
                    .unwrap();
                    assert!(view.prepare_structure(root, delta));
                }
                assert!(
                    matches!(view.body_scopes[&child], Scope::Pending),
                    "kind {kind}, revoke {revoke}"
                );
                assert!(matches!(
                    view.at(Position::new(2, 12), false).unwrap(),
                    PreparedQuery::NeedsAnalysis
                ));
            }
        }
    }
    #[test]
    fn invisible_root_header_cannot_lend_scope_to_a_uniquely_projected_child() {
        let source: Arc<str> = Arc::from("root:\n    hold");
        let mapped = skript_parser::MappedSource::identity(Arc::clone(&source));
        let index = PreparedDocument::test_fixture(Arc::clone(&source))
            .document
            .index;
        for duplicate_projection in [false, true] {
            let mut tree = skript_parser::parse_raw_tree(&mapped, index.raw_options);
            let root = tree.roots[0];
            if duplicate_projection {
                let mut duplicate = tree.get(root).unwrap().clone();
                duplicate.id = RawNodeId::new(tree.nodes.len() as u64);
                duplicate.children.clear();
                tree.roots.push(duplicate.id);
                tree.nodes.push(duplicate);
            } else {
                // Exact range alone cannot authorize a header whose text is not this source's text.
                tree.nodes[root.get() as usize].line.raw_text = "other:".into();
            }
            let owner = candidate(
                &tree,
                0,
                Some(EntryValidator {
                    entry_data: vec![entry("hold", EntryKind::KeyValue)],
                }),
            );
            for mode in [
                StructureBodyMode::Trigger,
                StructureBodyMode::Entries,
                StructureBodyMode::Raw,
                StructureBodyMode::None,
            ] {
                let view = PreparedDocument::new(
                    Arc::clone(&source),
                    Arc::clone(&index),
                    &mapped,
                    &tree,
                    &[StructureHeaderPreparation {
                        attempt_id: 1,
                        candidate: &owner,
                        body_mode: mode,
                        ambiguous: false,
                    }],
                )
                .unwrap();
                assert!(!view.document.context.contains_key(&0));
                assert!(
                    view.document.context.contains_key(&1),
                    "the child still has a unique exact source row"
                );
                assert!(
                    view.owners.is_empty(),
                    "an omitted header cannot certify an owner"
                );
                assert!(matches!(
                    view.at(Position::new(1, 8), false).unwrap(),
                    PreparedQuery::NeedsAnalysis
                ));
            }
        }
    }
}

fn prepare_entries(
    tree: &skript_parser::RawTree,
    root: RawNodeId,
    validator: &EntryValidator,
    modes: &BTreeMap<Vec<String>, skript_parser::SectionBodyMode>,
    plans: &skript_parser::StructureEntryPlans,
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
            let index=if path.is_empty() {skript_parser::structure_entry_index_with_plans(validator,child,&seen,plans)}
                else {skript_parser::structure_entry_index(validator,child,&seen)};
            let Some(index) = index else {
                continue;
            };
            seen.insert(index);
            present.push((index, *id));
            let data = &validator.entry_data[index];
            if path.is_empty() && let Some(plan)=plans.entries.get(id) {
                scopes.insert(*id,match plan.mode {skript_parser::StructureEntryBodyMode::Trigger=>Scope::Statements,skript_parser::StructureEntryBodyMode::Conditions=>Scope::Conditions,skript_parser::StructureEntryBodyMode::Raw=>Scope::Raw});
                continue;
            }
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

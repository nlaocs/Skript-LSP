use super::*;

/// Mapped source and data-only AST passed through the AST macro pipeline.
pub struct AstMacroRequest {
    pub context: InvocationContext,
    pub source: MappedSource,
    pub tree: AstTree,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One AST macro invocation and the state/provenance it committed.
pub struct AstMacroCall {
    pub component_id: String,
    pub subscription_id: String,
    pub target: u64,
    pub accepted: bool,
    pub expansion: Option<ExpansionId>,
    pub state_accesses: StateReadWriteSet,
}

#[derive(Debug)]
/// Final AST, provenance graph, effects, and per-component failures.
pub struct AstMacroResult {
    pub decision: HookDecision,
    pub source: MappedSource,
    pub tree: AstTree,
    pub effects: HookEffects,
    pub calls: Vec<AstMacroCall>,
    pub failures: Vec<ComponentFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AstMacroCycleKey {
    component_id: String,
    subscription_id: String,
    fingerprint: [u8; 32],
}

struct AstMacroPipeline {
    effects: HookEffects,
    calls: Vec<AstMacroCall>,
    failures: Vec<ComponentFailure>,
    active: Vec<AstMacroCycleKey>,
    output_bytes: usize,
    handled: bool,
}

impl AstMacroPipeline {
    fn new() -> Self {
        Self {
            effects: empty_effects(),
            calls: Vec::new(),
            failures: Vec::new(),
            active: Vec::new(),
            output_bytes: 0,
            handled: false,
        }
    }
}

enum AstWalk {
    Continue { sibling_count: usize },
    Reject(HookDecision),
}

impl ParserHost {
    /// Runs AST macros in an automatically committed parse transaction.
    pub fn expand_ast(
        &mut self,
        project_uri: &str,
        request: AstMacroRequest,
    ) -> Result<AstMacroResult, HostError> {
        let transaction = self.begin_parse(
            project_uri,
            &request.context.document_id,
            request.context.document_revision,
        )?;
        match self.expand_ast_in_parse(&transaction, request) {
            Ok(result) if matches!(result.decision, HookDecision::Reject(_)) => {
                transaction.cancel()?;
                Ok(result)
            }
            Ok(result) => {
                if self.dynamic_syntax_registry.is_some()
                    && let Err(error) = self.dynamic_syntax_snapshot(&transaction)
                {
                    let _ = transaction.cancel();
                    return Err(error);
                }
                transaction.commit()?;
                Ok(result)
            }
            Err(error) => {
                let _ = transaction.cancel();
                Err(error)
            }
        }
    }

    /// Runs AST macros inside a caller-owned document transaction.
    pub fn expand_ast_in_parse(
        &mut self,
        transaction: &ParseTransaction,
        request: AstMacroRequest,
    ) -> Result<AstMacroResult, HostError> {
        let document_id = transaction.document_id()?;
        let document_revision = transaction.document_revision()?;
        if document_id != request.context.document_id
            || document_revision != request.context.document_revision
        {
            return Err(StateError::InvalidInput {
                message: format!(
                    "AST macro context {}@{} does not match parse transaction {}@{}",
                    request.context.document_id,
                    request.context.document_revision,
                    document_id,
                    document_revision
                ),
            }
            .into());
        }
        if request.source.virtual_source().len() > self.config.max_virtual_source_bytes {
            return Err(HostError::VirtualSourceQuotaExceeded {
                limit: self.config.max_virtual_source_bytes,
            });
        }
        validate_ast_tree(
            &request.source,
            &request.tree,
            self.config.max_ast_macro_nodes,
            self.config.max_ast_depth,
        )
        .map_err(|message| HostError::InvalidAstMacroOutput {
            component_id: "<host>".to_owned(),
            subscription_id: "<input>".to_owned(),
            message,
        })?;

        let original_source = request.source.clone();
        let original_tree = request.tree.clone();
        let state_savepoint = transaction.savepoint()?;
        match self.expand_ast_with_transaction(transaction, request) {
            Ok(mut result) if matches!(result.decision, HookDecision::Reject(_)) => {
                transaction.rollback_to(&state_savepoint)?;
                mark_ast_macro_result_rolled_back(&original_source, &original_tree, &mut result);
                Ok(result)
            }
            Ok(result) => Ok(result),
            Err(error) => {
                transaction.rollback_to(&state_savepoint)?;
                Err(error)
            }
        }
    }

    fn expand_ast_with_transaction(
        &mut self,
        transaction: &ParseTransaction,
        request: AstMacroRequest,
    ) -> Result<AstMacroResult, HostError> {
        let mut source = request.source;
        let mut tree = request.tree;
        let mut pipeline = AstMacroPipeline::new();
        let mut root_index = 0usize;
        let mut decision = HookDecision::ContinueProcessing;

        while root_index < tree.roots.len() {
            match self.expand_ast_node(
                transaction,
                &request.context,
                &mut source,
                &mut tree,
                &[root_index],
                0,
                &mut pipeline,
            )? {
                AstWalk::Continue { sibling_count } => {
                    root_index = root_index.saturating_add(sibling_count);
                }
                AstWalk::Reject(rejection) => {
                    decision = rejection;
                    break;
                }
            }
        }
        if matches!(decision, HookDecision::ContinueProcessing) && pipeline.handled {
            decision = HookDecision::Handled;
        }
        Ok(AstMacroResult {
            decision,
            source,
            tree,
            effects: pipeline.effects,
            calls: pipeline.calls,
            failures: pipeline.failures,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_ast_node(
        &mut self,
        transaction: &ParseTransaction,
        request_context: &InvocationContext,
        source: &mut MappedSource,
        tree: &mut AstTree,
        path: &[usize],
        depth: usize,
        pipeline: &mut AstMacroPipeline,
    ) -> Result<AstWalk, HostError> {
        if depth > self.config.max_ast_macro_expansion_depth {
            return Err(HostError::AstMacroExpansionDepthQuotaExceeded {
                limit: self.config.max_ast_macro_expansion_depth,
            });
        }
        if path.len() > self.config.max_ast_depth {
            return Err(HostError::AstDepthQuotaExceeded {
                limit: self.config.max_ast_depth,
            });
        }
        let Some(_) = ast_node_at_path(tree, path) else {
            return Err(HostError::InvalidAstMacroOutput {
                component_id: "<host>".to_owned(),
                subscription_id: "<ast-walk>".to_owned(),
                message: format!("AST path {path:?} no longer resolves"),
            });
        };

        let node = ast_node(
            tree,
            ast_node_at_path(tree, path).expect("path was validated"),
        )
        .expect("a resolved AST node ID must exist");
        let candidates = matching_ast_candidates(&self.registry, node);
        let mut stop_current_node = false;
        for candidate in &candidates {
            if stop_current_node {
                break;
            }
            if self.components[candidate.component_index].disabled
                || self.components[candidate.component_index].unloaded
            {
                continue;
            }
            if pipeline.calls.len() >= self.config.max_ast_macro_calls {
                return Err(HostError::AstMacroCallQuotaExceeded {
                    limit: self.config.max_ast_macro_calls,
                });
            }

            let component_id = self.components[candidate.component_index]
                .manifest
                .component_id
                .clone();
            let subscription_id = candidate.subscription.id.clone();
            let target =
                ast_node_at_path(tree, path).ok_or_else(|| HostError::InvalidAstMacroOutput {
                    component_id: component_id.clone(),
                    subscription_id: subscription_id.clone(),
                    message: format!("AST path {path:?} disappeared before invocation"),
                })?;
            let node = ast_node(tree, target)
                .expect("a resolved AST node ID must exist")
                .clone();
            let cycle_key = ast_macro_cycle_key(&component_id, &subscription_id, tree, target);
            if pipeline.active.contains(&cycle_key) {
                pipeline.calls.push(AstMacroCall {
                    component_id: component_id.clone(),
                    subscription_id: subscription_id.clone(),
                    target,
                    accepted: false,
                    expansion: None,
                    state_accesses: StateReadWriteSet::default(),
                });
                pipeline.failures.push(ComponentFailure {
                    component_id: component_id.clone(),
                    subscription_id: subscription_id.clone(),
                    error: HostError::AstMacroCycleDetected {
                        component_id,
                        subscription_id,
                    },
                });
                continue;
            }
            pipeline.active.push(cycle_key);

            let mut context = request_context.clone();
            context.subscription_id = subscription_id.clone();
            context.expansion = node
                .span
                .origins
                .first()
                .and_then(|origin| origin.expansion);
            context.syntax_context = node.syntax_context;
            let input = AstMacroInput {
                context,
                tree: tree.clone(),
                target,
                depth: u32::try_from(depth).unwrap_or(u32::MAX),
            };
            let state_invocation = transaction.begin_invocation(component_id.clone())?;
            let (call, state_invocation) =
                {
                    let entry = &mut self.components[candidate.component_index];
                    if entry.store.data().invocation.is_some()
                        || entry.store.data().dynamic_syntax_update.is_some()
                    {
                        pipeline.active.pop();
                        return Err(StateError::Internal {
                            message: format!(
                                "component {component_id} already has an active host transaction"
                            ),
                        }
                        .into());
                    }
                    entry.store.data_mut().invocation = Some(state_invocation);
                    if let Err(error) = prepare_store(
                        &mut entry.store,
                        self.config.fuel_per_call,
                        self.config.deadline_ticks(&component_id),
                        &component_id,
                        "AST macro",
                    ) {
                        entry
                            .store
                            .data_mut()
                            .invocation
                            .take()
                            .expect("the invocation was just installed")
                            .rollback();
                        pipeline.active.pop();
                        return Err(error);
                    }
                    let call = entry
                        .bindings
                        .nlaocs_skript_parser_addon_ast_macro()
                        .call_expand(&mut entry.store, &input);
                    let state_invocation =
                        entry.store.data_mut().invocation.take().expect(
                            "the invocation remains installed for the duration of the call",
                        );
                    (call, state_invocation)
                };
            let accesses = state_invocation.read_write_set();
            let mut output = match call {
                Ok(Ok(output)) => output,
                Ok(Err(mut addon_error)) => {
                    let diagnostic_error = normalize_text_macro_diagnostics(
                        source,
                        &mut addon_error.diagnostics,
                        "addon-error.diagnostics",
                    );
                    state_invocation.rollback();
                    pipeline.active.pop();
                    pipeline.calls.push(AstMacroCall {
                        component_id: component_id.clone(),
                        subscription_id: subscription_id.clone(),
                        target,
                        accepted: false,
                        expansion: None,
                        state_accesses: accesses,
                    });
                    let error = match diagnostic_error {
                        Ok(()) => {
                            pipeline.effects.diagnostics.extend(addon_error.diagnostics);
                            HostError::AddonFailure {
                                component_id: component_id.clone(),
                                message: addon_error.message,
                            }
                        }
                        Err(message) => HostError::InvalidAstMacroOutput {
                            component_id: component_id.clone(),
                            subscription_id: subscription_id.clone(),
                            message,
                        },
                    };
                    pipeline.failures.push(ComponentFailure {
                        component_id,
                        subscription_id,
                        error,
                    });
                    continue;
                }
                Err(error) => {
                    state_invocation.rollback();
                    pipeline.active.pop();
                    let error = classify_wasmtime_error(component_id.clone(), "AST macro", error);
                    if error.disables_component() {
                        self.components[candidate.component_index].disabled = true;
                        if let Some(registry) = &self.dynamic_syntax_registry {
                            registry.remove_component(&component_id)?;
                        }
                    }
                    pipeline.calls.push(AstMacroCall {
                        component_id: component_id.clone(),
                        subscription_id: subscription_id.clone(),
                        target,
                        accepted: false,
                        expansion: None,
                        state_accesses: accesses,
                    });
                    pipeline.failures.push(ComponentFailure {
                        component_id,
                        subscription_id,
                        error,
                    });
                    continue;
                }
            };

            pipeline.output_bytes = pipeline
                .output_bytes
                .saturating_add(ast_macro_output_size(&output));
            if pipeline.output_bytes > self.config.max_generated_output_bytes {
                state_invocation.rollback();
                pipeline.active.pop();
                return Err(HostError::GeneratedOutputQuotaExceeded {
                    limit: self.config.max_generated_output_bytes,
                });
            }
            if let Err(message) = normalize_ast_macro_output(source, &node, &mut output) {
                state_invocation.rollback();
                pipeline.active.pop();
                record_invalid_ast_output(
                    pipeline,
                    component_id,
                    subscription_id,
                    target,
                    node.span.clone(),
                    accesses,
                    message,
                );
                continue;
            }

            let AstMacroOutput {
                decision,
                replacement,
                mut effects,
            } = output;
            if matches!(decision, HookDecision::NotApplicable) {
                state_invocation.rollback();
                pipeline.active.pop();
                pipeline.calls.push(AstMacroCall {
                    component_id,
                    subscription_id,
                    target,
                    accepted: false,
                    expansion: None,
                    state_accesses: accesses,
                });
                continue;
            }
            stamp_parse_result_attachments(&mut effects, &component_id);
            if matches!(decision, HookDecision::Reject(_)) {
                state_invocation.rollback();
                pipeline.active.pop();
                pipeline.calls.push(AstMacroCall {
                    component_id,
                    subscription_id,
                    target,
                    accepted: false,
                    expansion: None,
                    state_accesses: accesses,
                });
                merge_effects(&mut pipeline.effects, effects);
                return Ok(AstWalk::Reject(decision));
            }

            let Some(replacement) = replacement else {
                state_invocation.commit()?;
                pipeline.active.pop();
                pipeline.calls.push(AstMacroCall {
                    component_id,
                    subscription_id,
                    target,
                    accepted: true,
                    expansion: None,
                    state_accesses: accesses,
                });
                merge_effects(&mut pipeline.effects, effects);
                if matches!(decision, HookDecision::Handled) {
                    pipeline.handled = true;
                    stop_current_node = true;
                }
                continue;
            };

            let application = match apply_ast_replacement(
                self,
                transaction,
                source,
                tree,
                path,
                &node,
                replacement,
                &component_id,
                &subscription_id,
            ) {
                Ok(application) => application,
                Err(message) => {
                    state_invocation.rollback();
                    pipeline.active.pop();
                    record_invalid_ast_output(
                        pipeline,
                        component_id,
                        subscription_id,
                        target,
                        node.span.clone(),
                        accesses,
                        message,
                    );
                    continue;
                }
            };
            if application.tree.nodes.len() > self.config.max_ast_macro_nodes {
                state_invocation.rollback();
                pipeline.active.pop();
                return Err(HostError::AstMacroNodeQuotaExceeded {
                    limit: self.config.max_ast_macro_nodes,
                });
            }
            if ast_depth(&application.tree).unwrap_or(usize::MAX) > self.config.max_ast_depth {
                state_invocation.rollback();
                pipeline.active.pop();
                return Err(HostError::AstDepthQuotaExceeded {
                    limit: self.config.max_ast_depth,
                });
            }

            state_invocation.commit()?;
            let replacement_roots = application.replacement_roots;
            *source = application.source;
            *tree = application.tree;
            pipeline.calls.push(AstMacroCall {
                component_id,
                subscription_id,
                target,
                accepted: true,
                expansion: Some(application.expansion),
                state_accesses: accesses,
            });
            merge_effects(&mut pipeline.effects, effects);
            if matches!(decision, HookDecision::Handled) {
                pipeline.handled = true;
            }

            let base_index = *path
                .last()
                .expect("AST paths always include a sibling index");
            let mut final_count = 0usize;
            for _ in 0..replacement_roots {
                let mut generated_path = path.to_vec();
                *generated_path
                    .last_mut()
                    .expect("AST paths always include a sibling index") =
                    base_index.saturating_add(final_count);
                match self.expand_ast_node(
                    transaction,
                    request_context,
                    source,
                    tree,
                    &generated_path,
                    depth.saturating_add(1),
                    pipeline,
                )? {
                    AstWalk::Continue { sibling_count } => {
                        final_count = final_count.saturating_add(sibling_count);
                    }
                    AstWalk::Reject(rejection) => {
                        pipeline.active.pop();
                        return Ok(AstWalk::Reject(rejection));
                    }
                }
            }
            pipeline.active.pop();
            return Ok(AstWalk::Continue {
                sibling_count: final_count,
            });
        }

        let Some(target) = ast_node_at_path(tree, path) else {
            return Ok(AstWalk::Continue { sibling_count: 0 });
        };
        let mut child_index = 0usize;
        while child_index < ast_node(tree, target).map_or(0, |node| node.children.len()) {
            let mut child_path = path.to_vec();
            child_path.push(child_index);
            match self.expand_ast_node(
                transaction,
                request_context,
                source,
                tree,
                &child_path,
                depth,
                pipeline,
            )? {
                AstWalk::Continue { sibling_count } => {
                    child_index = child_index.saturating_add(sibling_count);
                }
                AstWalk::Reject(rejection) => return Ok(AstWalk::Reject(rejection)),
            }
        }
        Ok(AstWalk::Continue { sibling_count: 1 })
    }
}

struct AstReplacementApplication {
    source: MappedSource,
    tree: AstTree,
    expansion: ExpansionId,
    replacement_roots: usize,
}

fn ast_node(tree: &AstTree, id: u64) -> Option<&AstNode> {
    tree.nodes.iter().find(|node| node.id == id)
}

fn matching_ast_candidates(
    registry: &SubscriptionRegistry,
    node: &AstNode,
) -> Vec<RegisteredSubscription> {
    let target = ast_dispatch_target(node);
    let mut matching = registry
        .subscriptions
        .iter()
        .filter(|entry| {
            entry.subscription.phase == HookPhase::Ast
                && entry.subscription.capability_id == CAPABILITY_AST_MACRO
        })
        .filter_map(|entry| {
            let specificity = if matches!(entry.subscription.target, HookTarget::ParseStage) {
                Some(0)
            } else {
                target_specificity(&entry.subscription.target, &target)
            }?;
            Some((specificity, entry.clone()))
        })
        .collect::<Vec<_>>();
    matching.sort_by(|(left_specificity, left), (right_specificity, right)| {
        right_specificity
            .cmp(left_specificity)
            .then_with(|| left.subscription.priority.cmp(&right.subscription.priority))
            .then_with(|| left.load_order.cmp(&right.load_order))
            .then_with(|| left.declaration_order.cmp(&right.declaration_order))
    });
    matching.into_iter().map(|(_, entry)| entry).collect()
}

fn ast_dispatch_target(node: &AstNode) -> DispatchTarget {
    let kind = node.kind;
    let Some(summary) = &node.summary else {
        return DispatchTarget::SyntaxKind(kind);
    };
    match (
        summary.definition_id.as_ref(),
        summary.registration_id.as_ref(),
        summary.pattern_index,
    ) {
        (Some(definition_id), Some(registration_id), Some(pattern_index)) => {
            DispatchTarget::Pattern {
                definition_id: definition_id.clone(),
                registration_id: registration_id.clone(),
                pattern_index,
                syntax_kind: kind,
            }
        }
        (Some(definition_id), Some(registration_id), _) => DispatchTarget::Registration {
            definition_id: definition_id.clone(),
            registration_id: registration_id.clone(),
            syntax_kind: kind,
        },
        (Some(definition_id), _, _) => DispatchTarget::Definition {
            definition_id: definition_id.clone(),
            syntax_kind: kind,
        },
        _ => DispatchTarget::SyntaxKind(kind),
    }
}

fn ast_node_at_path(tree: &AstTree, path: &[usize]) -> Option<u64> {
    let mut siblings = &tree.roots;
    let mut current = None;
    for index in path {
        let id = *siblings.get(*index)?;
        current = Some(id);
        siblings = &ast_node(tree, id)?.children;
    }
    current
}

fn validate_ast_tree(
    source: &MappedSource,
    tree: &AstTree,
    max_nodes: usize,
    max_depth: usize,
) -> Result<(), String> {
    if tree.nodes.len() > max_nodes {
        return Err(format!("AST contains more than {max_nodes} nodes"));
    }
    let mut nodes = HashMap::with_capacity(tree.nodes.len());
    for node in &tree.nodes {
        if nodes.insert(node.id, node).is_some() {
            return Err(format!("AST node ID {} is repeated", node.id));
        }
        if node.syntax_id.trim().is_empty() {
            return Err(format!("AST node {} has a blank syntax ID", node.id));
        }
        normalize_text_macro_span(source, &node.span, "AST node span")?;
        if node.syntax_context != 0
            && !source
                .expansions()
                .iter()
                .any(|expansion| u64::from(expansion.syntax_context.get()) == node.syntax_context)
        {
            return Err(format!(
                "AST node {} references unknown syntax context {}",
                node.id, node.syntax_context
            ));
        }
        validate_metadata(&node.metadata)?;
        if let Some(summary) = &node.summary {
            validate_metadata(&summary.metadata)?;
            public_data::validate(&summary.public_data)?;
        }
        for capture in &node.captures {
            if capture.name.trim().is_empty() {
                return Err(format!("AST node {} has a blank capture name", node.id));
            }
            if let CaptureValue::Span(span) = &capture.value {
                normalize_text_macro_span(source, span, "AST capture span")?;
            }
        }
    }

    let roots = tree.roots.iter().copied().collect::<HashSet<_>>();
    if roots.len() != tree.roots.len() {
        return Err("AST root ID is repeated".to_owned());
    }
    let mut parents = HashMap::<u64, u64>::new();
    for node in &tree.nodes {
        let mut local_children = HashSet::new();
        for child in &node.children {
            if !nodes.contains_key(child) {
                return Err(format!(
                    "AST node {} references missing child {child}",
                    node.id
                ));
            }
            if !local_children.insert(*child) {
                return Err(format!("AST node {} repeats child {child}", node.id));
            }
            if let Some(previous) = parents.insert(*child, node.id) {
                return Err(format!(
                    "AST node {child} has multiple parents {previous} and {}",
                    node.id
                ));
            }
        }
        for capture in &node.captures {
            for referenced in capture_node_ids(&capture.value) {
                if !nodes.contains_key(&referenced) {
                    return Err(format!(
                        "AST node {} capture {} references missing node {referenced}",
                        node.id, capture.name
                    ));
                }
                if !node.children.contains(&referenced) {
                    return Err(format!(
                        "AST node {} capture {} references non-child node {referenced}",
                        node.id, capture.name
                    ));
                }
            }
        }
    }
    for root in &tree.roots {
        if !nodes.contains_key(root) {
            return Err(format!("AST references missing root {root}"));
        }
        if parents.contains_key(root) {
            return Err(format!("AST root {root} also has a parent"));
        }
    }

    let mut visited = HashSet::new();
    let mut active = HashSet::new();
    for root in &tree.roots {
        validate_ast_subtree(*root, &nodes, 1, max_depth, &mut visited, &mut active)?;
    }
    if visited.len() != tree.nodes.len() {
        return Err("AST contains unreachable nodes".to_owned());
    }
    Ok(())
}

fn validate_ast_subtree(
    id: u64,
    nodes: &HashMap<u64, &AstNode>,
    depth: usize,
    max_depth: usize,
    visited: &mut HashSet<u64>,
    active: &mut HashSet<u64>,
) -> Result<(), String> {
    if depth > max_depth {
        return Err(format!(
            "AST exceeds the structural depth quota of {max_depth}"
        ));
    }
    if !active.insert(id) {
        return Err(format!("AST contains a cycle at node {id}"));
    }
    if visited.insert(id) {
        for child in &nodes[&id].children {
            validate_ast_subtree(
                *child,
                nodes,
                depth.saturating_add(1),
                max_depth,
                visited,
                active,
            )?;
        }
    }
    active.remove(&id);
    Ok(())
}

fn validate_metadata(metadata: &[WitMetadataEntry]) -> Result<(), String> {
    let mut keys = BTreeSet::new();
    for entry in metadata {
        if entry.key.trim().is_empty() {
            return Err("metadata keys must not be blank".to_owned());
        }
        if !keys.insert((entry.owner_component_id.as_deref(), entry.key.as_str())) {
            return Err(format!(
                "metadata key {} is repeated for owner {}",
                entry.key,
                entry.owner_component_id.as_deref().unwrap_or("<catalog>")
            ));
        }
    }
    Ok(())
}

fn capture_node_ids(value: &CaptureValue) -> Vec<u64> {
    match value {
        CaptureValue::Node(id) => vec![*id],
        CaptureValue::Nodes(ids) => ids.clone(),
        CaptureValue::Text(_) | CaptureValue::Span(_) => Vec::new(),
    }
}

fn ast_depth(tree: &AstTree) -> Option<usize> {
    let nodes = tree
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<HashMap<_, _>>();
    tree.roots
        .iter()
        .map(|root| ast_subtree_depth(*root, &nodes, &mut HashSet::new()))
        .collect::<Option<Vec<_>>>()
        .map(|depths| depths.into_iter().max().unwrap_or(0))
}

fn ast_subtree_depth(
    id: u64,
    nodes: &HashMap<u64, &AstNode>,
    active: &mut HashSet<u64>,
) -> Option<usize> {
    if !active.insert(id) {
        return None;
    }
    let node = nodes.get(&id)?;
    let child_depth = node
        .children
        .iter()
        .map(|child| ast_subtree_depth(*child, nodes, active))
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .max()
        .unwrap_or(0);
    active.remove(&id);
    Some(child_depth.saturating_add(1))
}

fn normalize_ast_macro_output(
    source: &MappedSource,
    target: &AstNode,
    output: &mut AstMacroOutput,
) -> Result<(), String> {
    normalize_text_macro_effects(source, &mut output.effects, "effects")?;
    if let HookDecision::Reject(rejection) = &mut output.decision {
        normalize_text_macro_diagnostics(
            source,
            &mut rejection.diagnostics,
            "rejection.diagnostics",
        )?;
    }
    let Some(replacement) = &mut output.replacement else {
        return Ok(());
    };
    for node in &mut replacement.nodes {
        node.span = normalize_text_macro_span(source, &node.span, "replacement node span")?;
        if !range_contains(&target.span.virtual_range, &node.span.virtual_range) {
            return Err(format!(
                "replacement node {} span lies outside target {}",
                node.id, target.id
            ));
        }
        for capture in &mut node.captures {
            if let CaptureValue::Span(span) = &mut capture.value {
                *span = normalize_text_macro_span(source, span, "replacement capture span")?;
                if !range_contains(&target.span.virtual_range, &span.virtual_range) {
                    return Err(format!(
                        "replacement capture {} span lies outside target {}",
                        capture.name, target.id
                    ));
                }
            }
        }
    }
    Ok(())
}

fn range_contains(outer: &WitTextRange, inner: &WitTextRange) -> bool {
    outer.start <= inner.start && inner.end <= outer.end
}

#[allow(clippy::too_many_arguments)]
fn apply_ast_replacement(
    host: &ParserHost,
    transaction: &ParseTransaction,
    source: &MappedSource,
    tree: &AstTree,
    path: &[usize],
    target: &AstNode,
    mut fragment: AstTree,
    component_id: &str,
    subscription_id: &str,
) -> Result<AstReplacementApplication, String> {
    let requested_origins = fragment
        .nodes
        .iter()
        .map(|node| (node.id, node.context_origin))
        .collect::<HashMap<_, _>>();
    for node in &mut fragment.nodes {
        node.syntax_context = 0;
    }
    validate_ast_tree(
        source,
        &fragment,
        host.config.max_ast_macro_nodes,
        host.config.max_ast_depth,
    )?;

    let target_subtree = ast_subtree_ids(tree, target.id)?;
    let dynamic_snapshot = host
        .dynamic_syntax_registry
        .as_ref()
        .and_then(|_| host.dynamic_syntax_snapshot(transaction).ok());
    for node in &mut fragment.nodes {
        let previous = ast_node(tree, node.id).filter(|previous| {
            target_subtree.contains(&previous.id) && same_ast_identity(previous, node)
        });
        validate_ast_syntax(host, dynamic_snapshot.as_ref(), tree, node, component_id)?;
        merge_owned_metadata(
            previous.map_or(&[], |node| node.metadata.as_slice()),
            &mut node.metadata,
            component_id,
        )?;
        if let Some(summary) = &mut node.summary {
            let original_metadata: &[WitMetadataEntry] = previous
                .and_then(|node| node.summary.as_ref())
                .map_or(&[], |summary| summary.metadata.as_slice());
            merge_owned_metadata(original_metadata, &mut summary.metadata, component_id)?;
            public_data::validate(&summary.public_data)?;
            if let Some(registration_id) = &summary.registration_id
                && node.syntax_id != *registration_id
                && !node.syntax_id.starts_with("parser:")
                && !node.syntax_id.starts_with("macro:")
            {
                return Err(format!(
                    "AST node {} syntax ID does not match its parse summary registration ID",
                    node.id
                ));
            }
        }
    }

    validate_parent_capture_replacement(tree, path, target.id, fragment.roots.len())?;
    let call_site = parser_mapped_span_from_wit(&target.span)
        .map_err(|error| format!("AST call-site span: {error}"))?;
    let definition_site = fragment
        .nodes
        .iter()
        .find(|node| {
            matches!(
                requested_origins.get(&node.id),
                Some(AstContextOrigin::DefinitionSite)
            )
        })
        .and_then(|node| {
            parser_mapped_span_from_wit(&node.span)
                .ok()?
                .primary_origin()
                .map(|origin| skript_parser::ExpansionSite {
                    original_range: origin.original_range,
                    expansion: origin.expansion,
                })
        });
    let expansion = source
        .register_ast_expansion(
            std::slice::from_ref(&call_site),
            AstExpansion {
                component: skript_parser::ComponentId::new(component_id),
                hook: skript_parser::HookId::new(subscription_id),
                definition_site,
            },
        )
        .map_err(|error| error.to_string())?;
    let expansion_id = u64::from(expansion.expansion.get());

    let next_id = tree
        .nodes
        .iter()
        .map(|node| node.id)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| "AST node ID space is exhausted".to_owned())?;
    let id_map = fragment
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let offset = u64::try_from(index).map_err(|_| "AST fragment is too large")?;
            let id = next_id
                .checked_add(offset)
                .ok_or("AST node ID space is exhausted")?;
            Ok((node.id, id))
        })
        .collect::<Result<HashMap<_, _>, &str>>()
        .map_err(str::to_owned)?;

    for node in &mut fragment.nodes {
        let guest_id = node.id;
        let requested = requested_origins
            .get(&guest_id)
            .expect("every fragment node recorded a context origin");
        node.syntax_context = match requested {
            AstContextOrigin::Preserved => {
                let previous = ast_node(tree, guest_id)
                    .filter(|previous| target_subtree.contains(&previous.id))
                    .ok_or_else(|| {
                        format!("new AST node {guest_id} cannot request a preserved context")
                    })?;
                if !same_ast_identity(previous, node) {
                    return Err(format!(
                        "modified AST node {guest_id} cannot request a preserved context"
                    ));
                }
                previous.syntax_context
            }
            AstContextOrigin::CallSite => target.syntax_context,
            AstContextOrigin::Macro | AstContextOrigin::DefinitionSite => {
                u64::from(expansion.syntax_context.get())
            }
        };
        node.id = id_map[&guest_id];
        for child in &mut node.children {
            *child = id_map[child];
        }
        for capture in &mut node.captures {
            remap_capture_ids(&mut capture.value, &id_map);
            if let CaptureValue::Span(span) = &mut capture.value {
                stamp_ast_expansion(span, expansion_id);
            }
        }
        stamp_ast_expansion(&mut node.span, expansion_id);
    }
    let replacement_roots = fragment.roots.len();
    let roots = fragment
        .roots
        .iter()
        .map(|id| id_map[id])
        .collect::<Vec<_>>();

    let mut merged = tree.clone();
    merged.nodes.extend(fragment.nodes);
    if path.len() == 1 {
        let index = path[0];
        merged.roots.splice(index..=index, roots.iter().copied());
    } else {
        let parent = ast_node_at_path(tree, &path[..path.len() - 1])
            .ok_or_else(|| "AST replacement parent disappeared".to_owned())?;
        let child_index = *path.last().expect("non-root paths have a child index");
        let parent = merged
            .nodes
            .iter_mut()
            .find(|node| node.id == parent)
            .expect("the replacement parent exists in the merged AST");
        parent
            .children
            .splice(child_index..=child_index, roots.iter().copied());
        rewrite_parent_captures(parent, target.id, &roots)?;
    }

    let tree = canonicalize_ast(&merged)?;
    validate_ast_tree(
        &expansion.source,
        &tree,
        host.config.max_ast_macro_nodes,
        host.config.max_ast_depth,
    )?;
    Ok(AstReplacementApplication {
        source: expansion.source,
        tree,
        expansion: expansion.expansion,
        replacement_roots,
    })
}

fn ast_subtree_ids(tree: &AstTree, root: u64) -> Result<HashSet<u64>, String> {
    let nodes = tree
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<HashMap<_, _>>();
    let mut found = HashSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !found.insert(id) {
            continue;
        }
        let node = nodes
            .get(&id)
            .ok_or_else(|| format!("AST references missing node {id}"))?;
        stack.extend(node.children.iter().copied());
    }
    Ok(found)
}

fn same_ast_identity(previous: &AstNode, replacement: &AstNode) -> bool {
    mem::discriminant(&previous.kind) == mem::discriminant(&replacement.kind)
        && previous.syntax_id == replacement.syntax_id
        && previous.text == replacement.text
        && same_mapped_span(&previous.span, &replacement.span)
}

fn validate_ast_syntax(
    host: &ParserHost,
    dynamic_snapshot: Option<&DynamicSyntaxSnapshot>,
    original: &AstTree,
    node: &AstNode,
    component_id: &str,
) -> Result<(), String> {
    if node.syntax_id.starts_with("parser:")
        || node
            .syntax_id
            .starts_with(&format!("macro:{component_id}/"))
        || original.nodes.iter().any(|candidate| {
            mem::discriminant(&candidate.kind) == mem::discriminant(&node.kind)
                && candidate.syntax_id == node.syntax_id
        })
    {
        return Ok(());
    }
    let kind = catalog_syntax_kind(node.kind);
    if host.config.syntax_catalog.as_ref().is_some_and(|catalog| {
        catalog.syntaxes().iter().any(|syntax| {
            syntax.kind() == kind
                && (syntax.definition_id().as_str() == node.syntax_id
                    || syntax.registration_id().as_str() == node.syntax_id)
        })
    }) || dynamic_snapshot.is_some_and(|snapshot| {
        snapshot
            .definitions
            .values()
            .any(|syntax| syntax.kind == kind && syntax.id.qualified() == node.syntax_id)
    }) {
        Ok(())
    } else {
        Err(format!(
            "AST node {} references unknown {:?} syntax {}",
            node.id, node.kind, node.syntax_id
        ))
    }
}

fn validate_parent_capture_replacement(
    tree: &AstTree,
    path: &[usize],
    target: u64,
    replacement_count: usize,
) -> Result<(), String> {
    if path.len() == 1 {
        return Ok(());
    }
    let parent = ast_node_at_path(tree, &path[..path.len() - 1])
        .and_then(|id| ast_node(tree, id))
        .ok_or_else(|| "AST replacement parent does not exist".to_owned())?;
    for capture in &parent.captures {
        if matches!(capture.value, CaptureValue::Node(id) if id == target) && replacement_count != 1
        {
            return Err(format!(
                "capture {} is singular and requires exactly one replacement node",
                capture.name
            ));
        }
    }
    Ok(())
}

fn remap_capture_ids(value: &mut CaptureValue, ids: &HashMap<u64, u64>) {
    match value {
        CaptureValue::Node(id) => *id = ids[id],
        CaptureValue::Nodes(values) => {
            for id in values {
                *id = ids[id];
            }
        }
        CaptureValue::Text(_) | CaptureValue::Span(_) => {}
    }
}

fn rewrite_parent_captures(
    parent: &mut AstNode,
    target: u64,
    replacements: &[u64],
) -> Result<(), String> {
    for capture in &mut parent.captures {
        match &mut capture.value {
            CaptureValue::Node(id) if *id == target => {
                *id = *replacements.first().ok_or_else(|| {
                    format!("capture {} cannot reference a deleted node", capture.name)
                })?;
            }
            CaptureValue::Nodes(ids) => {
                let mut rewritten = Vec::with_capacity(
                    ids.len()
                        .saturating_sub(1)
                        .saturating_add(replacements.len()),
                );
                for id in ids.iter().copied() {
                    if id == target {
                        rewritten.extend_from_slice(replacements);
                    } else {
                        rewritten.push(id);
                    }
                }
                *ids = rewritten;
            }
            CaptureValue::Node(_) | CaptureValue::Text(_) | CaptureValue::Span(_) => {}
        }
    }
    Ok(())
}

fn canonicalize_ast(tree: &AstTree) -> Result<AstTree, String> {
    let nodes = tree
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<HashMap<_, _>>();
    let mut order = Vec::new();
    let mut id_map = HashMap::new();
    let mut active = HashSet::new();
    for root in &tree.roots {
        collect_canonical_order(*root, &nodes, &mut id_map, &mut order, &mut active)?;
    }
    let roots = tree.roots.iter().map(|id| id_map[id]).collect();
    let mut canonical = Vec::with_capacity(order.len());
    for old_id in order {
        let mut node = nodes[&old_id].clone();
        node.id = id_map[&old_id];
        for child in &mut node.children {
            *child = id_map[child];
        }
        for capture in &mut node.captures {
            remap_capture_ids(&mut capture.value, &id_map);
        }
        canonical.push(node);
    }
    Ok(AstTree {
        roots,
        nodes: canonical,
    })
}

fn collect_canonical_order(
    id: u64,
    nodes: &HashMap<u64, &AstNode>,
    id_map: &mut HashMap<u64, u64>,
    order: &mut Vec<u64>,
    active: &mut HashSet<u64>,
) -> Result<(), String> {
    if id_map.contains_key(&id) {
        return Ok(());
    }
    if !active.insert(id) {
        return Err(format!("AST contains a cycle at node {id}"));
    }
    let node = nodes
        .get(&id)
        .ok_or_else(|| format!("AST references missing node {id}"))?;
    let canonical_id = u64::try_from(order.len()).map_err(|_| "AST is too large")?;
    id_map.insert(id, canonical_id);
    order.push(id);
    for child in &node.children {
        collect_canonical_order(*child, nodes, id_map, order, active)?;
    }
    active.remove(&id);
    Ok(())
}

fn ast_macro_cycle_key(
    component_id: &str,
    subscription_id: &str,
    tree: &AstTree,
    target: u64,
) -> AstMacroCycleKey {
    let nodes = tree
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<HashMap<_, _>>();
    let mut digest = Sha256::new();
    hash_ast_subtree(target, &nodes, &mut digest);
    AstMacroCycleKey {
        component_id: component_id.to_owned(),
        subscription_id: subscription_id.to_owned(),
        fingerprint: digest.finalize().into(),
    }
}

fn hash_ast_subtree(id: u64, nodes: &HashMap<u64, &AstNode>, digest: &mut Sha256) {
    let Some(node) = nodes.get(&id) else {
        return;
    };
    hash_debug(&node.kind, digest);
    hash_text(&node.syntax_id, digest);
    hash_text(&node.text, digest);
    hash_mapped_span(&node.span, digest);
    hash_debug(&node.context_origin, digest);
    hash_debug(&node.summary, digest);
    hash_len(node.captures.len(), digest);
    for capture in &node.captures {
        hash_text(&capture.name, digest);
        match &capture.value {
            CaptureValue::Text(value) => {
                digest.update([0]);
                hash_text(value, digest);
            }
            CaptureValue::Node(child) => {
                digest.update([1]);
                hash_child_reference(node, *child, digest);
            }
            CaptureValue::Nodes(children) => {
                digest.update([2]);
                hash_len(children.len(), digest);
                for child in children {
                    hash_child_reference(node, *child, digest);
                }
            }
            CaptureValue::Span(span) => {
                digest.update([3]);
                hash_mapped_span(span, digest);
            }
        }
    }
    hash_len(node.metadata.len(), digest);
    for entry in &node.metadata {
        hash_text(
            entry.owner_component_id.as_deref().unwrap_or_default(),
            digest,
        );
        hash_text(&entry.key, digest);
        hash_text(&entry.value, digest);
    }
    hash_len(node.children.len(), digest);
    for child in &node.children {
        hash_ast_subtree(*child, nodes, digest);
    }
}

fn hash_child_reference(node: &AstNode, child: u64, digest: &mut Sha256) {
    let position = node
        .children
        .iter()
        .position(|candidate| *candidate == child)
        .unwrap_or(usize::MAX);
    hash_len(position, digest);
}

fn hash_mapped_span(span: &MappedSpan, digest: &mut Sha256) {
    digest.update(span.virtual_range.start.to_le_bytes());
    digest.update(span.virtual_range.end.to_le_bytes());
    hash_len(span.origins.len(), digest);
    for origin in &span.origins {
        digest.update(origin.original_range.start.to_le_bytes());
        digest.update(origin.original_range.end.to_le_bytes());
        hash_debug(&origin.kind, digest);
    }
}

fn hash_text(value: &str, digest: &mut Sha256) {
    hash_len(value.len(), digest);
    digest.update(value.as_bytes());
}

fn hash_len(value: usize, digest: &mut Sha256) {
    digest.update(u64::try_from(value).unwrap_or(u64::MAX).to_le_bytes());
}

fn hash_debug(value: &impl std::fmt::Debug, digest: &mut Sha256) {
    hash_text(&format!("{value:?}"), digest);
}

fn stamp_ast_expansion(span: &mut MappedSpan, expansion: u64) {
    for origin in &mut span.origins {
        origin.expansion = Some(expansion);
    }
}

fn ast_macro_output_size(output: &AstMacroOutput) -> usize {
    output
        .replacement
        .as_ref()
        .map_or(0, ast_tree_size)
        .saturating_add(hook_effects_size(&output.effects))
        .saturating_add(match &output.decision {
            HookDecision::Reject(rejection) => rejection_size(rejection),
            HookDecision::NotApplicable
            | HookDecision::ContinueProcessing
            | HookDecision::Handled => 0,
        })
}

fn record_invalid_ast_output(
    pipeline: &mut AstMacroPipeline,
    component_id: String,
    subscription_id: String,
    target: u64,
    span: MappedSpan,
    state_accesses: StateReadWriteSet,
    message: String,
) {
    pipeline.calls.push(AstMacroCall {
        component_id: component_id.clone(),
        subscription_id: subscription_id.clone(),
        target,
        accepted: false,
        expansion: None,
        state_accesses,
    });
    pipeline.effects.diagnostics.push(Diagnostic {
        code: "ast-macro-invalid-output".to_owned(),
        message: format!(
            "component {component_id} returned invalid AST macro output for {subscription_id}: {message}"
        ),
        severity: DiagnosticSeverity::Error,
        span,
        related: Vec::new(),
    });
    pipeline.failures.push(ComponentFailure {
        component_id: component_id.clone(),
        subscription_id: subscription_id.clone(),
        error: HostError::InvalidAstMacroOutput {
            component_id,
            subscription_id,
            message,
        },
    });
}

fn mark_ast_macro_result_rolled_back(
    original_source: &MappedSource,
    original_tree: &AstTree,
    result: &mut AstMacroResult,
) {
    for call in &mut result.calls {
        call.accepted = false;
        call.expansion = None;
    }
    result.source = original_source.clone();
    result.tree = original_tree.clone();
    result.effects.context_updates.clear();
    result.effects.parse_requests.clear();
    retain_known_diagnostic_expansions(original_source, &mut result.effects.diagnostics);
    if let HookDecision::Reject(rejection) = &mut result.decision {
        retain_known_diagnostic_expansions(original_source, &mut rejection.diagnostics);
    }
}

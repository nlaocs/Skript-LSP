//! Conservative type exclusion over a complete editing hole. Missing proofs keep
//! suggestions; ranking/signature heuristics never select a type environment.
//! Output types belong to the definition represented by a proposal. Another
//! interpretation of identical inserted text does not change that definition.
//! Completion selection does not constrain the native parser's eventual choice.
use super::*;
use parser_wasm::host::source_geometry;
use skript_parser::{
    ExpressionExpectedType, FunctionVersionPolicy, expression_types_are_incompatible,
};
use syntax_pattern_parser::syntax::PatternTypeExpr;
use syntaxes::{ClassName, Multiplicity, ResolutionState, ReturnTypeState};

pub(super) const HOLE: &str = "\u{fffc}";
const MAX_DEPTH: usize = 12;

struct Output {
    definition: String,
    registration: String,
    actual: Actual,
    capture_routes: Option<Vec<skript_parser::RegisteredCaptureBinding>>,
    declared: Option<ClassName>,
    declared_state: ReturnTypeState,
    possible: Vec<ClassName>,
    possible_state: syntaxes::PossibleReturnTypesState,
    section_expression: bool,
}
struct Actual {
    classes: Vec<ClassName>,
    complete: bool,
    multiplicity: Option<Multiplicity>,
}
impl Actual {
    fn incompatible(&self, catalog: &Catalog, expected: &[ExpressionExpectedType]) -> bool {
        expression_types_are_incompatible(
            catalog,
            &self.classes,
            self.complete,
            self.multiplicity,
            expected,
        )
    }
}
pub(super) struct Index {
    pub(super) catalog: Arc<Catalog>,
    pub(super) type_names: Vec<(String, String)>,
    pub(super) type_names_limited: bool,
    outputs: HashMap<String, Output>,
    pub(super) complete_patterns: bool,
    closed: bool,
    candidate_bindings_closed: bool,
    static_locals_stable: bool,
    function_policy: Option<FunctionVersionPolicy>,
    certificate: Option<parser_wasm::host::CompletionTypeCertificate>,
}
impl Index {
    pub(super) fn new(catalog: Arc<Catalog>) -> Self {
        Self {
            catalog,
            type_names: Vec::new(),
            type_names_limited: false,
            outputs: HashMap::new(),
            complete_patterns: true,
            closed: false,
            candidate_bindings_closed: false,
            static_locals_stable: false,
            function_policy: None,
            certificate: None,
        }
    }
    pub(super) fn current(&self) -> bool {
        self.certificate.as_ref().is_none_or(|c| c.is_current())
    }
    pub(super) fn finite_captures_current(&self) -> bool {
        self.certificate.is_some()
            && self.current()
            && self.closed
            && self.candidate_bindings_closed
    }
    pub(super) fn insert(&mut self, identity: &str, expression: &syntaxes::Expression) {
        let (classes, complete) = match expression.return_type_state {
            ReturnTypeState::Static => (
                expression.return_type.iter().cloned().collect(),
                expression.return_type.is_some(),
            ),
            _ => (Vec::new(), false),
        };
        self.outputs.insert(
            identity.into(),
            Output {
                definition: expression.common.definition_id.0.clone(),
                registration: expression.common.registration_id.0.clone(),
                capture_routes: None,
                declared: expression.return_type.clone(),
                declared_state: expression.return_type_state,
                possible: expression.possible_return_types.clone(),
                possible_state: expression.possible_return_types_state,
                section_expression: expression.section_expression,
                actual: Actual {
                    classes,
                    complete: complete
                        && expression.return_type_multiplicity_state == ResolutionState::Resolved,
                    multiplicity: (expression.return_type_multiplicity_state
                        == ResolutionState::Resolved)
                        .then_some(expression.return_type_multiplicity)
                        .flatten(),
                },
            },
        );
    }
}
impl Index {
    /// Display snapshot declarations separately from the current provider proof.
    /// A revoked proof must never leave a previously narrowed type in the menu.
    fn output_display(&self, identity: &str) -> (String, &'static str) {
        let Some(output) = self.outputs.get(identity) else {
            return ("unknown".into(), "unknown");
        };
        let actual = &output.actual;
        let proven = self.closed && self.current() && actual.complete && !actual.classes.is_empty();
        let classes = |values: &[ClassName]| {
            values
                .iter()
                .map(|class| super::super::hover_text::type_name(&self.catalog, class.as_str()))
                .collect::<Vec<_>>()
                .join(" | ")
        };
        let ty = if proven {
            classes(&actual.classes)
        } else {
            let mut value = match output.declared_state {
                ReturnTypeState::Dynamic => "dynamic".to_owned(),
                _ => "unknown".to_owned(),
            };
            if let Some(declared) = &output.declared {
                value.push_str(&format!(
                    " (snapshot {:?}: {})",
                    output.declared_state,
                    super::super::hover_text::type_name(&self.catalog, declared.as_str())
                ));
            }
            if !output.possible.is_empty() {
                value.push_str(&format!(
                    "; snapshot possibilities {:?}: {}",
                    output.possible_state,
                    classes(&output.possible)
                ));
            }
            value
        };
        let cardinality = if proven { actual.multiplicity } else { None };
        (
            ty,
            match cardinality {
                Some(Multiplicity::Single) => "single",
                Some(Multiplicity::Multiple) => "plural",
                Some(Multiplicity::Both) => "single or plural",
                None => "unknown",
            },
        )
    }
    pub(super) fn output_description(&self, identity: &str) -> String {
        let (ty, cardinality) = self.output_display(identity);
        format!("return: {ty}; cardinality: {cardinality}")
    }
    pub(super) fn display_type(&self, class: &str) -> String {
        super::super::hover_text::type_name(&self.catalog, class)
    }
    pub(super) fn output_fields(&self, identity: &str) -> Vec<(&'static str, String)> {
        let (ty, cardinality) = self.output_display(identity);
        let mut fields = vec![("Return type", ty), ("Multiplicity", cardinality.into())];
        if let Some(output) = self.outputs.get(identity) {
            let classes = output
                .actual
                .classes
                .iter()
                .chain(output.declared.iter())
                .chain(output.possible.iter())
                .map(ClassName::as_str)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
                .join(" | ");
            if !classes.is_empty() {
                fields.push(("Snapshot value classes", classes));
            }
        }
        fields
    }
}
impl CatalogIndex {
    pub(in crate::lsp) fn certify_types(&mut self, host: &parser_wasm::host::ParserHost) {
        let Some(index) = &mut self.typing else {
            return;
        };
        // Unadvertised document-dependent registrations make the Catalog an
        // incomplete description of possible parent grammars.
        (index.type_names, index.type_names_limited) = host.completion_type_names();
        index.closed = host.completion_catalog_patterns_are_closed();
        index.candidate_bindings_closed = host.completion_candidate_bindings_are_closed();
        index.static_locals_stable = host.completion_static_local_substitution_is_stable();
        index.function_policy = Some(host.function_policy());
        index.certificate = Some(host.completion_type_certificate());
        for output in index.outputs.values_mut() {
            output.capture_routes = host
                .completion_expression_capture_bindings(&output.definition, &output.registration);
            if let Some(classes) =
                host.completion_output_bound(&output.definition, &output.registration)
            {
                output.actual.classes = classes.into_iter().map(ClassName).collect();
                output.actual.complete = true;
                // An output-class bound does not certify runtime cardinality.
                output.actual.multiplicity = None;
            } else {
                output.actual.complete &= !host.registered_syntax_may_resolve(
                    SyntaxKind::Expression,
                    &output.definition,
                    &output.registration,
                );
            }
        }
        let catalog = Arc::clone(&index.catalog);
        let routes = if index.finite_captures_current() {
            index
                .outputs
                .iter()
                .filter_map(|(id, output)| {
                    output
                        .capture_routes
                        .clone()
                        .map(|routes| (id.clone(), routes))
                })
                .collect()
        } else {
            HashMap::new()
        };
        finite::rebuild(self, &catalog, &routes);
    }
}
// Current adopted syntax supplies a stronger context than competing lexical
// hole paths (a broad object slot can also consume a narrower trailing slot).
// This index never survives a source change and never fixes the parser's choice
// after an edit. Incomplete/ambiguous captures fall back to the conservative graph.
pub(super) fn accepted_slots(
    witnesses: &parser_wasm::host::semantic_proof::SemanticWitnesses<'_>,
    nodes: &HashMap<u64, &AstNode>,
    reachable: &HashSet<u64>,
    index: &CatalogIndex,
    source: &str,
    blocked: &[TextRange],
) -> HashMap<(usize, usize), Vec<ExpressionExpectedType>> {
    let Some(typing) = &index.typing else {
        return HashMap::new();
    };
    let patterns = index
        .patterns
        .iter()
        .map(|p| (p.identity.as_str(), p))
        .collect::<HashMap<_, _>>();
    let mut slots = HashMap::new();
    let mut ambiguous = HashSet::new();
    for id in reachable {
        let parent = nodes[id];
        if !witnesses.native_valid(parent) || exact_node(parent, source).is_none() {
            continue;
        }
        let Some(summary) = &parent.summary else {
            continue;
        };
        let Some(identity) = summary
            .registration_id
            .as_ref()
            .zip(summary.pattern_index)
            .map(|(id, pattern)| format!("{id}:{pattern}"))
        else {
            continue;
        };
        let Some(pattern) = patterns.get(identity.as_str()) else {
            continue;
        };
        for (parameter, types) in pattern.graph.slot_types.iter().enumerate() {
            let Some(expected) = types
                .as_ref()
                .and_then(|t| expected_types(&typing.catalog, t, false))
            else {
                continue;
            };
            let Some(child) = super::super::semantic::capture_node(parent, parameter as u64, nodes)
                .and_then(|id| nodes.get(&id).copied())
            else {
                continue;
            };
            if !witnesses.native_valid(child) {
                continue;
            }
            let Some(range) = exact_node(child, source) else {
                continue;
            };
            let native = format!("{}{parameter}", source_geometry::PATTERN_CAPTURE_PREFIX);
            if !parent.captures.iter().any(|c| c.name == native && matches!(&c.value,
                parser_wasm::bindings::nlaocs::skript_parser_addon::types::CaptureValue::Span(span)
                    if span.virtual_range.start == child.span.virtual_range.start && span.virtual_range.end == child.span.virtual_range.end))
                || blocked.iter().any(|b| overlaps(*b, range)) { continue; }
            let key = (range.start, range.end);
            if slots.insert(key, expected).is_some() {
                ambiguous.insert(key);
            }
        }
    }
    for key in ambiguous {
        slots.remove(&key);
    }
    slots
}
impl Document {
    /// Positive literal guidance may use an unchanged native line after an
    /// unrelated macro edit. This does not certify hard filtering or binding
    /// substitution: those retain the stronger whole-source proof below.
    fn literal_guidance_context_is_exact(&self, line: &Line, edit: TextRange) -> bool {
        if edit.start < line.code_start
            || edit.end > line.range.end
            || !edit.is_valid_for(self.source.as_ref())
            || self.blocked.iter().any(|range| overlaps(*range, line.range))
        {
            return false;
        }
        self.type_context_exact
            || line.exact_source
                && line.node.is_some()
                && line.parent.is_some()
                && line.ancestors.first().copied() == line.parent
                && !line.root
                && !line.blocked
                && matches!(line.scope, Scope::Statements | Scope::Conditions | Scope::Expressions)
    }

    pub(super) fn expected_at(
        &self,
        line: &Line,
        edit: TextRange,
        references: bool,
    ) -> Option<Vec<ExpressionExpectedType>> {
        self.expected_at_with_budget(line, edit, references, &mut continuation::Budget::new())
    }
    pub(super) fn expected_at_with_budget(
        &self,
        line: &Line,
        edit: TextRange,
        references: bool,
        budget: &mut continuation::Budget,
    ) -> Option<Vec<ExpressionExpectedType>> {
        self.expected_at_mode(line, edit, references, false, budget)
    }
    pub(super) fn expected_literal_at(
        &self,
        line: &Line,
        edit: TextRange,
        references: bool,
        budget: &mut continuation::Budget,
    ) -> Option<Vec<ExpressionExpectedType>> {
        self.expected_at_mode(line, edit, references, true, budget)
    }
    /// Finite literal proposals may use a typed structural prefix when the
    /// user has not entered the required suffix yet. This evidence is separate
    /// from expected_at/replacement_types, which remain complete-path proofs.
    pub(super) fn literal_value_targets_at(
        &self,
        line: &Line,
        edit: TextRange,
        references: bool,
        budget: &mut continuation::Budget,
    ) -> Option<(Vec<ExpressionExpectedType>, bool)> {
        if let Some(types) = self.expected_literal_at(line, edit, references, budget) {
            return Some((types, false));
        }
        let index = self.index.typing.as_ref()?;
        if budget.exhausted() || !index.current() || !index.closed
            || !index.complete_patterns
            || !self.literal_guidance_context_is_exact(line, edit) || line.root
        {
            return None;
        }
        let text = self.editing_hole(line, edit)?;
        let start = text.find(HOLE)?;
        if !matches!(lexical_context(&text[..start]), Lexical::Code(_))
            || text.contains('%') || text.starts_with('(')
            || (text.contains('(') && !text.ends_with(')'))
        {
            return None;
        }
        let domain = skript_parser::BodySyntaxDomain::for_node(
            if matches!(line.scope, Scope::Conditions) {
                skript_parser::SectionBodyMode::Conditions
            } else {
                skript_parser::SectionBodyMode::Trigger
            },
            if line.section { RawNodeKind::Section } else { RawNodeKind::Simple },
        );
        for suffix_pending in [false, true] {
            if suffix_pending && !text.ends_with(HOLE) { break; }
            let mut types = Vec::new();
            for pattern in &self.index.patterns {
                if !budget.scan(1) { return None; }
                let allowed = match line.scope {
                    Scope::Expressions => pattern.kind == SyntaxKind::Expression,
                    Scope::Conditions | Scope::Statements => domain.includes(
                        pattern.kind,
                        index.outputs.get(&pattern.identity).is_some_and(|o| o.section_expression),
                    ),
                    _ => false,
                };
                if !allowed { continue; }
                let hole = start..start + HOLE.len();
                let slots = if suffix_pending {
                    pattern.graph.literal_prefix_slots(&text, hole, budget)
                } else {
                    pattern.graph.hole_slots(&text, hole, budget)
                }.ok()?;
                for (parameter, range) in slots {
                    // Positive proposal evidence only. Unknown nested/regex
                    // routes cannot erase another direct literal route, nor
                    // donate an outer type to an unresolved inner expression.
                    if native_trim(text.get(range)?) != HOLE { continue; }
                    let Some(Some(slot)) = pattern.graph.slot_types.get(parameter) else { continue; };
                    if !slot.allow_literals { continue; }
                    if let Some(known) = expected_types(&index.catalog, slot, true) {
                        types.extend(known);
                    }
                }
            }
            if budget.exhausted() { return None; }
            if !types.is_empty() {
                types.sort_by(|a, b| a.class_name.0.cmp(&b.class_name.0).then(a.plural.cmp(&b.plural)));
                types.dedup();
                return Some((types, suffix_pending));
            }
        }
        None
    }

    fn expected_at_mode(
        &self,
        line: &Line,
        edit: TextRange,
        references: bool,
        literals: bool,
        budget: &mut continuation::Budget,
    ) -> Option<Vec<ExpressionExpectedType>> {
        let index = self.index.typing.as_ref()?;
        let exact_context = if literals {
            self.literal_guidance_context_is_exact(line, edit)
        } else {
            self.type_context_exact
        };
        if !index.current() || !index.closed || !exact_context || line.root {
            return None;
        }
        let end = line
            .comments
            .iter()
            .map(|r| r.start)
            .min()
            .unwrap_or(line.range.end);
        if edit.start < line.code_start
            || edit.end > end
            || end - line.code_start > MAX_CONTINUATION_BYTES
        {
            return None;
        }
        if !literals {
            if let Some(expected) = self.accepted_slots.get(&(edit.start, edit.end)) {
                return Some(expected.clone());
            }
        }
        if !index.complete_patterns {
            return None;
        }
        let text = self.editing_hole(line, edit)?;
        self.hole_expectations(
            &text,
            line.scope,
            line.section,
            references,
            literals,
            0,
            budget,
            &mut HashSet::new(),
        )
        .filter(|types| !types.is_empty())
    }
    pub(super) fn editing_hole(&self, line: &Line, edit: TextRange) -> Option<String> {
        let end = line
            .comments
            .iter()
            .map(|r| r.start)
            .min()
            .unwrap_or(line.range.end);
        if edit.start < line.code_start
            || edit.end > end
            || end - line.code_start > MAX_CONTINUATION_BYTES
        {
            return None;
        }
        let original = self.source.get(line.code_start..end)?;
        if original.contains(HOLE) {
            return None;
        }
        let mut text = String::new();
        text.push_str(self.source.get(line.code_start..edit.start)?);
        text.push_str(HOLE);
        text.push_str(self.source.get(edit.end..end)?);
        let trimmed = native_trim_end(&text);
        let result = if line.section {
            native_trim_end(trimmed.strip_suffix(':')?)
        } else {
            trimmed
        };
        Some(result.to_owned())
    }
    fn hole_expectations(
        &self,
        text: &str,
        scope: Scope,
        section: bool,
        references: bool,
        literals: bool,
        depth: usize,
        budget: &mut continuation::Budget,
        path: &mut HashSet<(String, u8, bool)>,
    ) -> Option<Vec<ExpressionExpectedType>> {
        if depth >= MAX_DEPTH || !budget.scan(text.len() + 1) {
            return None;
        }
        let start = text.find(HOLE)?;
        self.expression_context_types(
            text,
            start..start + HOLE.len(),
            None,
            scope,
            section,
            references,
            literals,
            depth,
            budget,
            path,
        )
    }
    fn expression_context_types(
        &self,
        text: &str,
        hole: std::ops::Range<usize>,
        candidate: Option<&str>,
        scope: Scope,
        section: bool,
        references: bool,
        literals: bool,
        depth: usize,
        budget: &mut continuation::Budget,
        path: &mut HashSet<(String, u8, bool)>,
    ) -> Option<Vec<ExpressionExpectedType>> {
        let key = (
            text.to_owned(),
            if matches!(scope, Scope::Expressions) {
                0
            } else if matches!(scope, Scope::Conditions) {
                1
            } else {
                2
            },
            section,
        );
        if !path.insert(key.clone()) {
            return self
                .index
                .typing
                .as_ref()?
                .candidate_bindings_closed
                .then(Vec::new);
        }
        let result = self.expression_context_types_inner(
            text, hole, candidate, scope, section, references, literals, depth, budget, path,
        );
        path.remove(&key);
        result
    }
    fn expression_context_types_inner(
        &self,
        text: &str,
        hole: std::ops::Range<usize>,
        candidate: Option<&str>,
        scope: Scope,
        section: bool,
        references: bool,
        literals: bool,
        depth: usize,
        budget: &mut continuation::Budget,
        path: &mut HashSet<(String, u8, bool)>,
    ) -> Option<Vec<ExpressionExpectedType>> {
        let index = self.index.typing.as_ref()?;
        // Interpolation and unfinished native frames are not Catalog slot paths.
        if !matches!(lexical_context(text.get(..hole.start)?), Lexical::Code(_))
            || text.contains('%')
            || text.starts_with('(')
            || (text.contains('(') && !text.ends_with(')'))
        {
            return None;
        }
        if !budget.scan(
            text.len()
                .saturating_mul(index.catalog.operators().len() + 1),
        ) {
            return None;
        }
        let intrinsic = skript_parser::expression_has_intrinsic_shape(&index.catalog, text);
        if intrinsic {
            return None;
        }
        let domain = skript_parser::BodySyntaxDomain::for_node(
            if matches!(scope, Scope::Conditions) {
                skript_parser::SectionBodyMode::Conditions
            } else {
                skript_parser::SectionBodyMode::Trigger
            },
            if section {
                RawNodeKind::Section
            } else {
                RawNodeKind::Simple
            },
        );
        let mut expected = Vec::new();
        let mut found = false;
        // A native complete function call reuses the parser's exact argument
        // binder; list/named/optional uncertainty is never guessed by commas.
        if let Some(open) = text.find('(').filter(|_| {
            text.ends_with(')')
                && (matches!(scope, Scope::Expressions) || domain.allows_function_call())
        }) {
            let name = &text[..open];
            if name.starts_with(|c: char| c == '_' || c.is_alphabetic())
                && name.chars().all(|c| c == '_' || c.is_alphanumeric())
            {
                if !references || !self.function_namespace_complete {
                    return None;
                }
                let mut shadowed = HashSet::new();
                for function in &self.functions {
                    if function.name == name {
                        shadowed.insert(function.shape());
                    }
                }
                let mut matched = false;
                for (order, function) in self
                    .functions
                    .iter()
                    .chain(&self.index.functions)
                    .enumerate()
                    .filter(|(_, f)| f.name == name)
                {
                    if !budget.scan(function.parameters.len() + name.len() + 1) {
                        return None;
                    }
                    if order >= self.functions.len() && shadowed.contains(&function.shape()) {
                        continue;
                    }
                    let definition = function.definition.as_ref()?;
                    let Some(arguments) =
                        definition.argument_expectations(text, index.function_policy?)
                    else {
                        return None;
                    };
                    for (range, ty) in arguments {
                        if range.start <= hole.start && hole.end <= range.end {
                            matched = true;
                            let captured = native_trim(text.get(range.start..range.end)?);
                            if captured == candidate.unwrap_or(HOLE) {
                                expected.push(ty);
                            } else {
                                if candidate.is_some() {
                                    return None;
                                }
                                expected.extend(self.hole_expectations(
                                    captured,
                                    Scope::Expressions,
                                    false,
                                    references,
                                    literals,
                                    depth + 1,
                                    budget,
                                    path,
                                )?);
                            }
                        }
                    }
                }
                if !matched {
                    return None;
                }
                found = true;
            }
        }
        for pattern in &self.index.patterns {
            if !budget.scan(1) {
                return None;
            }
            let allowed = match scope {
                Scope::Expressions => pattern.kind == SyntaxKind::Expression,
                Scope::Conditions | Scope::Statements => domain.includes(
                    pattern.kind,
                    index
                        .outputs
                        .get(&pattern.identity)
                        .is_some_and(|o| o.section_expression),
                ),
                _ => false,
            };
            if !allowed {
                continue;
            }
            let slots = match pattern.graph.hole_slots(text, hole.clone(), budget) {
                Ok(slots) => slots,
                Err(()) => {
                    return None;
                }
            };
            for (parameter, range) in slots {
                let types = if let Some(types) = pattern.graph.slot_types.get(parameter)?.as_ref() {
                    if !(if literals {
                        types.allow_literals
                    } else {
                        types.allow_expressions
                    }) && matches!(scope, Scope::Expressions)
                        && index.candidate_bindings_closed
                    {
                        // A native literals-only slot cannot bind the proposed
                        // registered Expression node. Its literal interpretation
                        // is a different candidate, not a return-type escape.
                        continue;
                    }
                    expected_types(&index.catalog, types, literals)?
                } else {
                    if !matches!(scope, Scope::Expressions) {
                        return None;
                    }
                    let routes = index
                        .outputs
                        .get(&pattern.identity)?
                        .capture_routes
                        .as_ref()?;
                    let Some(route) = routes.iter().find(|r| r.capture_index == parameter) else {
                        // Another opaque whole-range leaf is not a binding site
                        // for the proposal's own definition.
                        continue;
                    };
                    if route.parser_id != "host.expression" {
                        return None;
                    }
                    let (allow_literals, expressions) =
                        skript_parser::capture_expression_flags(route).ok()?;
                    if !(if literals {
                        allow_literals
                    } else {
                        expressions
                    }) {
                        continue;
                    }
                    let captured = native_trim(text.get(range.clone())?);
                    if captured != candidate.unwrap_or(HOLE) {
                        if candidate.is_some() {
                            return None;
                        }
                        expected.extend(self.hole_expectations(
                            captured,
                            Scope::Expressions,
                            false,
                            references,
                            literals,
                            depth + 1,
                            budget,
                            path,
                        )?);
                        found = true;
                        continue;
                    }
                    skript_parser::capture_expression_expected_types(route).ok()?
                };
                let captured = native_trim(text.get(range.clone())?);
                if captured == candidate.unwrap_or(HOLE) {
                    expected.extend(types);
                } else {
                    if candidate.is_some() {
                        return None;
                    }
                    // An outer slot's expected type does not describe the
                    // expression nested inside it. No recognized inner grammar
                    // means Unknown, even when the outer type is exact.
                    if range.start == 0
                        && range.end == text.len()
                        && !index.candidate_bindings_closed
                    {
                        return None;
                    }
                    expected.extend(self.hole_expectations(
                        captured,
                        Scope::Expressions,
                        false,
                        references,
                        literals,
                        depth + 1,
                        budget,
                        path,
                    )?);
                }
                found = true;
            }
        }
        if budget.exhausted() {
            return None;
        }
        if !found || expected.is_empty() {
            // Some(empty) means no declared place for this proposal node; it
            // does not mean the surrounding text is rejected by the language.
            // None remains unknown (intrinsic/routing/provider/limit evidence).
            return (matches!(scope, Scope::Expressions) && index.candidate_bindings_closed)
                .then(Vec::new);
        }
        expected.sort_by(|a, b| {
            a.class_name
                .0
                .cmp(&b.class_name.0)
                .then(a.plural.cmp(&b.plural))
        });
        expected.dedup();
        Some(expected)
    }
    fn replacement_types(
        &self,
        line: &Line,
        edit: TextRange,
        candidate: &str,
        references: bool,
        budget: &mut continuation::Budget,
    ) -> Option<Vec<ExpressionExpectedType>> {
        let index = self.index.typing.as_ref()?;
        if !index.current()
            || !index.closed
            || !index.complete_patterns
            || !self.type_context_exact
            || line.root
        {
            return None;
        }
        let end = line
            .comments
            .iter()
            .map(|r| r.start)
            .min()
            .unwrap_or(line.range.end);
        if edit.start < line.code_start || edit.end > end || candidate.contains(['\r', '\n']) {
            return None;
        }
        let mut text = String::new();
        text.push_str(self.source.get(line.code_start..edit.start)?);
        let start = text.len();
        text.push_str(candidate);
        let hole = start..text.len();
        text.push_str(self.source.get(edit.end..end)?);
        let text = native_trim_end(&text);
        let text = if line.section {
            native_trim_end(text.strip_suffix(':')?)
        } else {
            text
        };
        if text.len() > MAX_CONTINUATION_BYTES || !budget.scan(text.len() + 1) {
            return None;
        }
        self.expression_context_types(
            text,
            hole,
            Some(candidate),
            line.scope,
            line.section,
            references,
            false,
            0,
            budget,
            &mut HashSet::new(),
        )
    }
    pub(super) fn incompatible_template(
        &self,
        template: &Template,
        expected: Option<&[ExpressionExpectedType]>,
        line: &Line,
        edit: TextRange,
        item: &CompletionItem,
        whole_template: bool,
        references: bool,
        budget: &mut continuation::Budget,
    ) -> bool {
        // An insertion that changes line structure invalidates the raw-node
        // entry proof. Keep the candidate rather than projecting that change.
        if template.parts.iter().any(|part| {
            matches!(part,
            Part::Text(text) if text.contains(['\r', '\n']))
        }) || matches!(template.parts.last(), Some(Part::Text(text))
                if text.trim_end().ends_with(':'))
        {
            return false;
        }
        let Some((index, expected)) = self.index.typing.as_ref().zip(expected) else {
            return false;
        };
        if !index.current() {
            return false;
        }
        let Some(output) = index
            .outputs
            .get(&template.identity)
            .filter(|o| o.actual.incompatible(&index.catalog, expected))
        else {
            return false;
        };
        // A complete proposal keeps its definition's output type even when its
        // arguments are still snippet placeholders. A prefix reusing existing
        // right-hand text may instead belong to an inner expression capture.
        if whole_template {
            return true;
        }
        // Preserve prefix-only proposals whose full expression extent is unknown.
        if template.parts.iter().any(|p| matches!(p, Part::Slot(_))) {
            return false;
        }
        let candidate = render(&template.parts, false);
        let encoded = if item.insert_text_format == Some(InsertTextFormat::SNIPPET) {
            escape(&candidate)
        } else {
            candidate.clone()
        };
        if !matches!(&item.text_edit, Some(CompletionTextEdit::Edit(e))
            if e.new_text == encoded)
        {
            return false;
        }
        self.replacement_types(line, edit, &candidate, references, budget)
            .is_some_and(|types| output.actual.incompatible(&index.catalog, &types))
    }
    pub(super) fn incompatible_function(
        &self,
        function: &Function,
        expected: Option<&[ExpressionExpectedType]>,
    ) -> bool {
        let Some((index, expected)) = self.index.typing.as_ref().zip(expected) else {
            return false;
        };
        if !index.current() {
            return false;
        }
        let Some(definition) = &function.definition else {
            return false;
        };
        expression_types_are_incompatible(
            &index.catalog,
            definition.return_type.as_slice(),
            true,
            Some(if definition.return_type_is_single {
                Multiplicity::Single
            } else {
                Multiplicity::Multiple
            }),
            expected,
        )
    }
    pub(super) fn incompatible_variable(
        &self,
        name: &str,
        candidate: &str,
        edit: TextRange,
        budget: &mut continuation::Budget,
    ) -> bool {
        let Some(index) = self
            .index
            .typing
            .as_ref()
            .filter(|i| i.current() && i.static_locals_stable && self.type_context_exact)
        else {
            return false;
        };
        // A static scalar name replacement preserves this actual native lookup
        // under the explicit provider contract. Its requested types and incoming
        // facts are stronger evidence than a speculative parent Graph path.
        let Some((ty, requested, owner)) = self
            .semantic_types
            .as_ref()
            .and_then(|s| s.completion_input(&self.source, edit, name))
        else {
            return false;
        };
        let Some((classes, multiple)) = ty.proven_present() else {
            return false;
        };
        let adopted = self.accepted_slots.get(&(edit.start, edit.end));
        let expected = adopted.cloned().unwrap_or_else(|| {
            requested
                .iter()
                .cloned()
                .map(|name| ExpressionExpectedType {
                    class_name: ClassName(name),
                    plural: true,
                })
                .collect::<Vec<_>>()
        });
        // `multiple` is OR-joined across flow branches: true is not proof that
        // every runtime path is plural. Preserve that uncertainty here.
        expression_types_are_incompatible(
            &index.catalog,
            &classes.iter().cloned().map(ClassName).collect::<Vec<_>>(),
            true,
            (!multiple).then_some(Multiplicity::Single),
            &expected,
        ) && (adopted.is_some() || self.variable_parent_unchanged(edit, candidate, owner, budget))
    }
    fn variable_parent_unchanged(
        &self,
        edit: TextRange,
        candidate: &str,
        owner: &super::super::semantic::CompletionOwner,
        budget: &mut continuation::Budget,
    ) -> bool {
        self.variable_parent_proof(edit, candidate, owner, budget)
            .unwrap_or(false)
    }
    fn variable_parent_proof(
        &self,
        edit: TextRange,
        candidate: &str,
        owner: &super::super::semantic::CompletionOwner,
        budget: &mut continuation::Budget,
    ) -> Option<bool> {
        let index = self.index.typing.as_ref()?;
        if !index.closed || !index.complete_patterns {
            return None;
        }
        let line = self
            .context
            .get(&self.lines.position(&self.source, edit.start).ok()?.line)?;
        if line.section
            || !matches!(line.scope, Scope::Statements | Scope::Conditions)
            || owner.range.start != line.code_start
            || owner.range.end > line.range.end
            || edit.start < owner.range.start
            || edit.end > owner.range.end
        {
            return None;
        }
        let code_end = line
            .comments
            .iter()
            .map(|r| r.start)
            .min()
            .unwrap_or(line.range.end);
        if !self
            .source
            .get(owner.range.end..code_end)?
            .bytes()
            .all(|b| b <= 0x20)
        {
            return None;
        }
        let mut input = String::new();
        input.push_str(self.source.get(owner.range.start..edit.start)?);
        let start = input.len();
        input.push_str(candidate);
        let hole = start..input.len();
        input.push_str(self.source.get(edit.end..owner.range.end)?);
        if input.len() > MAX_CONTINUATION_BYTES {
            return None;
        }
        let domain = skript_parser::BodySyntaxDomain::for_node(
            if matches!(line.scope, Scope::Conditions) {
                skript_parser::SectionBodyMode::Conditions
            } else {
                skript_parser::SectionBodyMode::Trigger
            },
            RawNodeKind::Simple,
        );
        let mut found = false;
        for pattern in &self.index.patterns {
            if !budget.scan(1) {
                return None;
            }
            if !domain.includes(pattern.kind, false) {
                continue;
            }
            for (parameter, range) in pattern
                .graph
                .hole_slots(&input, hole.clone(), budget)
                .ok()?
            {
                if pattern.identity != owner.identity
                    || parameter != owner.parameter
                    || pattern.graph.slot_types.get(parameter)?.as_ref().is_none()
                    || native_trim(input.get(range)?) != candidate
                {
                    return None;
                }
                found = true;
            }
        }
        Some(found && !budget.exhausted())
    }
}
fn native_trim(input: &str) -> &str {
    let range = skript_parser::java_trim_range(input);
    &input[range.start..range.end]
}
fn native_trim_end(input: &str) -> &str {
    &input[..skript_parser::java_trim_range(input).end]
}
fn expected_types(
    catalog: &Catalog,
    types: &PatternTypeExpr,
    literals: bool,
) -> Option<Vec<ExpressionExpectedType>> {
    if !(if literals {
        types.allow_literals
    } else {
        types.allow_expressions
    }) || types.alternatives.is_empty()
    {
        return None;
    }
    types
        .alternatives
        .iter()
        .map(|alternative| {
            Some(ExpressionExpectedType {
                class_name: catalog
                    .type_by_code_name(&alternative.name)?
                    .original_class
                    .clone(),
                plural: alternative.plural,
            })
        })
        .collect()
}

#[cfg(test)]
mod binding_tests {
    use super::super::parameters::tests::{catalog, parse};
    use super::*;

    fn fixture(unknown: bool) -> Document {
        fixture_source(unknown, "on load:\n    consume wrap p\n")
    }
    fn fixture_source(unknown: bool, source: &str) -> Document {
        let (early, mut view, _, _) = parse(source);
        drop(early);
        let rules = catalog();
        let index = Arc::get_mut(&mut view.index).unwrap();
        index.patterns.clear();
        let definition = rules.expressions().next().unwrap();
        for (id, pattern, kind) in [
            ("parent", "consume %objects%", SyntaxKind::Effect),
            ("self", "<.+>", SyntaxKind::Expression),
            ("exit", "wrap %numbers%", SyntaxKind::Expression),
        ] {
            let parsed =
                syntax_pattern_parser::syntax::parse(pattern, rules.plural_rules()).unwrap();
            index.patterns.push(Pattern {
                documentation: None,
                kind,
                label: pattern.into(),
                detail: id.into(),
                identity: id.into(),
                graph: continuation::Graph::new(&parsed.elements).unwrap(),
            });
            if kind == SyntaxKind::Expression {
                index.typing.as_mut().unwrap().insert(id, definition);
            }
        }
        let typing = index.typing.as_mut().unwrap();
        typing.outputs.get_mut("self").unwrap().capture_routes =
            Some(vec![skript_parser::RegisteredCaptureBinding {
                capture_index: 0,
                parser_id: "host.expression".into(),
                required: true,
                options: std::collections::BTreeMap::from([(
                    "expression.expected-types".into(),
                    "java.lang.Object[]".into(),
                )]),
            }]);
        if unknown {
            let parsed =
                syntax_pattern_parser::syntax::parse("<.*>", rules.plural_rules()).unwrap();
            index.patterns.push(Pattern {
                documentation: None,
                kind: SyntaxKind::Expression,
                label: "<.*>".into(),
                detail: "unknown".into(),
                identity: "unknown".into(),
                graph: continuation::Graph::new(&parsed.elements).unwrap(),
            });
            typing.insert("unknown", definition);
            typing.outputs.get_mut("unknown").unwrap().capture_routes =
                Some(vec![skript_parser::RegisteredCaptureBinding {
                    capture_index: 0,
                    parser_id: "private.fixture.parser".into(),
                    required: true,
                    options: Default::default(),
                }]);
        }
        view
    }
    fn query(view: &Document) -> Option<Vec<ExpressionExpectedType>> {
        let start = view.source.rfind('p').unwrap();
        view.expected_at(&view.context[&1], TextRange::new(start, start + 1), true)
    }
    #[test]
    fn recursive_route_explores_typed_exit_but_unknown_exit_invalidates_proof() {
        let view = fixture(false);
        let types = query(&view).unwrap();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].class_name.as_str(), "java.lang.Number");
        assert!(types[0].plural);
        assert!(query(&fixture(true)).is_none());
    }
    #[test]
    fn regex_wrapper_keeps_its_typed_exit_and_other_root_union() {
        let mut view = fixture_source(false, "on load:\n    consume prefix wrap p\n");
        let rules = catalog();
        let index = Arc::get_mut(&mut view.index).unwrap();
        let wrapper = index
            .patterns
            .iter_mut()
            .find(|p| p.identity == "self")
            .unwrap();
        let parsed =
            syntax_pattern_parser::syntax::parse("prefix <.+>", rules.plural_rules()).unwrap();
        wrapper.label = "prefix <.+>".into();
        wrapper.graph = continuation::Graph::new(&parsed.elements).unwrap();
        let number = query(&view).unwrap();
        assert_eq!(
            number
                .iter()
                .map(|t| t.class_name.as_str())
                .collect::<Vec<_>>(),
            ["java.lang.Number"]
        );
        let index = Arc::get_mut(&mut view.index).unwrap();
        let label = "consume prefix wrap %strings%";
        let parsed = syntax_pattern_parser::syntax::parse(label, rules.plural_rules()).unwrap();
        index.patterns.push(Pattern {
            documentation: None,
            kind: SyntaxKind::Effect,
            label: label.into(),
            detail: "another root exit".into(),
            identity: "other-parent".into(),
            graph: continuation::Graph::new(&parsed.elements).unwrap(),
        });
        let union = query(&view).unwrap();
        assert_eq!(
            union
                .iter()
                .map(|t| t.class_name.as_str())
                .collect::<Vec<_>>(),
            ["java.lang.Number", "java.lang.String"]
        );
    }
    #[test]
    fn missing_child_closure_and_exhausted_budget_do_not_close_a_cycle() {
        let mut view = fixture(false);
        Arc::get_mut(&mut view.index)
            .unwrap()
            .typing
            .as_mut()
            .unwrap()
            .candidate_bindings_closed = false;
        assert!(query(&view).is_none());
        let view = fixture(false);
        let mut budget = continuation::Budget::new();
        assert!(!budget.scan(usize::MAX));
        let start = view.source.rfind('p').unwrap();
        assert!(
            view.expected_at_with_budget(
                &view.context[&1],
                TextRange::new(start, start + 1),
                true,
                &mut budget
            )
            .is_none()
        );
    }
    #[test]
    fn interpolation_and_unfinished_native_frames_keep_unknown() {
        for line in [
            "send \"a %p%\" to p",
            "send \"a\" to call(p",
            "send \"a\" to (p",
        ] {
            let source = format!("on load:\n    {line}\n");
            let (_, view, _, _) = parse(&source);
            let start = source.rfind('p').unwrap();
            assert!(
                view.expected_at(&view.context[&1], TextRange::new(start, start + 1), true)
                    .is_none(),
                "{line}"
            );
        }
    }
    #[test]
    fn displayed_dynamic_partial_union_and_cardinality_are_distinct() {
        let rules = catalog();
        let mut definition = rules.expressions().next().unwrap().clone();
        definition.return_type = Some(ClassName("java.lang.Object".into()));
        definition.return_type_state = ReturnTypeState::Dynamic;
        definition.possible_return_types = vec![
            ClassName("java.lang.Number".into()),
            ClassName("java.lang.String".into()),
        ];
        definition.possible_return_types_state = syntaxes::PossibleReturnTypesState::Partial;
        let mut index = Index::new(rules);
        index.insert("display", &definition);
        let dynamic = index.output_description("display");
        assert!(dynamic.starts_with("return: dynamic"), "{dynamic}");
        assert!(
            dynamic.contains("possibilities Partial: number | string"),
            "{dynamic}"
        );
        assert!(dynamic.ends_with("cardinality: unknown"), "{dynamic}");
        let fields = index.output_fields("display");
        assert!(
            fields
                .iter()
                .any(|(label, value)| *label == "Snapshot value classes"
                    && value.contains("java.lang.Number")
                    && value.contains("java.lang.String"))
        );
        index.closed = true;
        let output = index.outputs.get_mut("display").unwrap();
        output.actual = Actual {
            classes: definition.possible_return_types,
            complete: true,
            multiplicity: Some(Multiplicity::Both),
        };
        assert_eq!(
            index.output_description("display"),
            "return: number | string; cardinality: single or plural"
        );
        let output = index.outputs.get_mut("display").unwrap();
        output.actual.complete = false;
        output.declared_state = ReturnTypeState::Unresolved;
        assert!(
            index
                .output_description("display")
                .starts_with("return: unknown")
        );
    }
    #[test]
    fn ambiguous_complete_slot_guidance_does_not_assert_parameter_zero() {
        let (early, mut view, _, _) = parse("on load:\n    test p\n");
        drop(early);
        let rules = catalog();
        let index = Arc::get_mut(&mut view.index).unwrap();
        index.patterns.clear();
        let label = "test [%strings%] [%strings%]";
        let parsed = syntax_pattern_parser::syntax::parse(label, rules.plural_rules()).unwrap();
        index.patterns.push(Pattern {
            documentation: None,
            kind: SyntaxKind::Effect,
            label: label.into(),
            detail: "ambiguous editing slots".into(),
            identity: "optional".into(),
            graph: continuation::Graph::new(&parsed.elements).unwrap(),
        });
        let position = view
            .lines
            .position(&view.source, view.source.len() - 1)
            .unwrap();
        let SignatureQuery::Ready(Some(help)) = view.signature_at(position, true).unwrap() else {
            panic!("missing guidance")
        };
        assert_eq!(help.signatures[0].label, label);
        assert!(help.signatures[0].parameters.is_none());
        assert!(help.signatures[0].active_parameter.is_none());
    }
    #[test]
    fn full_recipient_slot_precedes_opaque_prefix_guidance_without_deleting_valid_connect() {
        for recipient in ["\"a\"", "all players"] {
            let source = format!("on load:\n    send {recipient} to p\n");
            let (_, view, _, _) = parse(&source);
            let position = view.lines.position(&source, source.len() - 1).unwrap();
            let SignatureQuery::Ready(Some(help)) = view.signature_at(position, true).unwrap()
            else {
                panic!("missing recipient guidance")
            };
            assert!(
                help.signatures[0].label.contains("%audiences%"),
                "{}",
                help.signatures[0].label
            );
            assert_eq!(help.signatures[0].active_parameter, Some(1));
            let list = view.at(position, false).unwrap();
            assert!(
                list.items.iter().any(|item| item.label == "proxy"),
                "a valid alternative grammar is not a type mismatch"
            );
            assert!(
                list.items
                    .iter()
                    .find(|item| item.label == "proxy")
                    .unwrap()
                    .detail
                    .as_deref()
                    .unwrap()
                    .contains("arguments unverified")
            );
        }
    }
}

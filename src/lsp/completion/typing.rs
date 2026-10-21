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

const HOLE: &str = "\u{fffc}";
const MAX_DEPTH: usize = 12;

struct Output {
    definition: String,
    registration: String,
    actual: Actual,
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
    catalog: Arc<Catalog>,
    outputs: HashMap<String, Output>,
    pub(super) complete_patterns: bool,
    closed: bool,
    static_locals_stable: bool,
    function_policy: Option<FunctionVersionPolicy>,
    certificate: Option<parser_wasm::host::CompletionTypeCertificate>,
}
impl Index {
    pub(super) fn new(catalog: Arc<Catalog>) -> Self {
        Self {
            catalog,
            outputs: HashMap::new(),
            complete_patterns: true,
            closed: false,
            static_locals_stable: false,
            function_policy: None,
            certificate: None,
        }
    }
    pub(super) fn current(&self) -> bool {
        self.certificate.as_ref().is_none_or(|c| c.is_current())
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
impl CatalogIndex {
    pub(in crate::lsp) fn certify_types(&mut self, host: &parser_wasm::host::ParserHost) {
        let Some(index) = &mut self.typing else {
            return;
        };
        // Unadvertised document-dependent registrations make the Catalog an
        // incomplete description of possible parent grammars.
        index.closed = host.completion_catalog_patterns_are_closed();
        index.static_locals_stable = host.completion_static_local_substitution_is_stable();
        index.function_policy = Some(host.function_policy());
        index.certificate = Some(host.completion_type_certificate());
        for output in index.outputs.values_mut() {
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
                .and_then(|t| expected_types(&typing.catalog, t))
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
    pub(super) fn expected_at(
        &self,
        line: &Line,
        edit: TextRange,
        references: bool,
    ) -> Option<Vec<ExpressionExpectedType>> {
        let index = self.index.typing.as_ref()?;
        if !index.current() || !index.closed || !self.type_context_exact || line.root {
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
        if let Some(expected) = self.accepted_slots.get(&(edit.start, edit.end)) {
            return Some(expected.clone());
        }
        if !index.complete_patterns {
            return None;
        }
        let original = &self.source[line.code_start..end];
        if original.contains(HOLE) {
            return None;
        }
        let mut text = String::with_capacity(original.len() + HOLE.len());
        text.push_str(&self.source[line.code_start..edit.start]);
        text.push_str(HOLE);
        text.push_str(&self.source[edit.end..end]);
        let text = native_trim_end(&text);
        let text = if line.section {
            native_trim_end(text.strip_suffix(':')?)
        } else {
            text
        };
        self.hole_expectations(
            text,
            line.scope,
            line.section,
            references,
            0,
            &mut continuation::Budget::new(),
        )
    }
    fn hole_expectations(
        &self,
        text: &str,
        scope: Scope,
        section: bool,
        references: bool,
        depth: usize,
        budget: &mut continuation::Budget,
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
            depth,
            budget,
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
        depth: usize,
        budget: &mut continuation::Budget,
    ) -> Option<Vec<ExpressionExpectedType>> {
        let index = self.index.typing.as_ref()?;
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
                                    depth + 1,
                                    budget,
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
                let types = pattern.graph.slot_types.get(parameter)?.as_ref()?;
                let captured = native_trim(text.get(range.clone())?);
                if captured == candidate.unwrap_or(HOLE) {
                    expected.extend(expected_types(&index.catalog, types)?);
                } else {
                    if candidate.is_some() {
                        return None;
                    }
                    // An outer slot's expected type does not describe the
                    // expression nested inside it. No recognized inner grammar
                    // means Unknown, even when the outer type is exact.
                    if range.start == 0 && range.end == text.len() {
                        return None;
                    }
                    expected.extend(self.hole_expectations(
                        captured,
                        Scope::Expressions,
                        false,
                        references,
                        depth + 1,
                        budget,
                    )?);
                }
                found = true;
            }
        }
        if !found || expected.is_empty() || budget.exhausted() {
            return None;
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
            0,
            budget,
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
            &definition.return_type.iter().cloned().collect::<Vec<_>>(),
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
        let Some((
            super::super::semantic::InferredType::Values {
                classes, multiple, ..
            },
            requested,
            owner,
        )) = self
            .semantic_types
            .as_ref()
            .and_then(|s| s.completion_input(&self.source, edit, name))
        else {
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
            (!*multiple).then_some(Multiplicity::Single),
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
) -> Option<Vec<ExpressionExpectedType>> {
    if !types.allow_expressions || types.alternatives.is_empty() {
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
